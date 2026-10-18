//! pla-snapshot: snapshot loop for PLA-003 (M1-c manual, M2 triggers+cleanup).
//!
//! Snapshot = pure file copy of the current `.plasticity` file (D1) into
//! `<base>/<doc_filename>/snapshots/`, with a sibling `.json` metadata file.
//! Doc filename is the only isolation key (D5). Restore opens the snapshot
//! with the system default app in a new window; the current window is NOT
//! closed. M2 adds the predicate-style [`SnapshotTrigger`] trait (D2) with
//! [`TimerTrigger`] / [`StepCountTrigger`] impls, plus retention cleanup.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How the snapshot was triggered. Metadata-only field; firing decisions are
/// made by [`SnapshotTrigger`] implementations (M2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    Manual,
    Timed,
    StepCounter,
}

/// Metadata persisted next to each snapshot as `<stem>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMeta {
    /// RFC3339 UTC timestamp of creation.
    pub ts: String,
    /// Sanitized user label.
    pub label: String,
    /// Entity count from the bridge (best-effort; 0 when unavailable).
    pub entity_count: u64,
    /// Isolation key: document filename.
    pub doc_filename: String,
    /// Trigger kind.
    pub trigger: TriggerKind,
    /// Snapshot id (on-disk file stem). Absent in M1 metadata; defaults
    /// empty and is backfilled from the filename on read.
    #[serde(default)]
    pub id: String,
    /// Pinned snapshots are exempt from cleanup deletion. Absent in older
    /// metadata; defaults to false.
    #[serde(default)]
    pub pinned: bool,
}

/// Read-only context handed to [`SnapshotTrigger::tick`] each poll.
pub struct TriggerContext<'a> {
    /// Isolation key (document filename).
    pub doc_key: &'a str,
    /// Injected clock (unix seconds) so triggers are testable without sleep.
    pub now_secs: u64,
    /// New-version count observed for `doc_key` since the last snapshot.
    pub new_version_count: u64,
}

/// Predicate-style snapshot trigger (arch D2). `tick` is polled by the
/// snapshot loop; returning `true` means "take a snapshot now" and the
/// implementation should have recorded whatever firing state it needs.
pub trait SnapshotTrigger {
    fn tick(&mut self, ctx: &TriggerContext) -> bool;
}

/// Default timer interval: 5 minutes.
pub const DEFAULT_TIMER_INTERVAL_SECS: u64 = 300;

/// Fires when `now - last_fire >= interval_secs`.
#[derive(Debug, Clone)]
pub struct TimerTrigger {
    pub interval_secs: u64,
    pub last_fire: Option<u64>,
}

impl Default for TimerTrigger {
    fn default() -> Self {
        Self {
            interval_secs: DEFAULT_TIMER_INTERVAL_SECS,
            last_fire: None,
        }
    }
}

impl TimerTrigger {
    pub fn new(interval_secs: u64) -> Self {
        Self {
            interval_secs,
            last_fire: None,
        }
    }
}

impl SnapshotTrigger for TimerTrigger {
    fn tick(&mut self, ctx: &TriggerContext) -> bool {
        let due = match self.last_fire {
            // First poll: fire immediately so protection starts at once.
            None => true,
            Some(last) => ctx.now_secs >= last && ctx.now_secs - last >= self.interval_secs,
        };
        if due {
            self.last_fire = Some(ctx.now_secs);
            true
        } else {
            false
        }
    }
}

/// Fires when `ctx.new_version_count >= threshold`. Stateless per tick;
/// per-doc-key counting lives in the caller-supplied context, so distinct
/// doc keys are naturally independent.
#[derive(Debug, Clone)]
pub struct StepCountTrigger {
    pub threshold: u64,
}

impl StepCountTrigger {
    pub fn new(threshold: u64) -> Self {
        Self { threshold }
    }
}

impl Default for StepCountTrigger {
    fn default() -> Self {
        Self { threshold: 100 }
    }
}

impl SnapshotTrigger for StepCountTrigger {
    fn tick(&mut self, ctx: &TriggerContext) -> bool {
        ctx.new_version_count >= self.threshold
    }
}

/// Fires on every tick (manual-loop helper / test stub).
#[derive(Debug, Clone, Default)]
pub struct AlwaysTrigger;

impl SnapshotTrigger for AlwaysTrigger {
    fn tick(&mut self, _ctx: &TriggerContext) -> bool {
        true
    }
}

/// Retention policy for [`SnapshotStore::cleanup`].
#[derive(Debug, Clone)]
pub struct CleanupPolicy {
    /// Keep at most this many (unpinned) snapshots per doc, newest first.
    pub max_count: usize,
    /// Delete unpinned snapshots older than this many seconds.
    pub max_age_secs: u64,
}

pub const DEFAULT_CLEANUP_MAX_COUNT: usize = 50;
pub const DEFAULT_CLEANUP_MAX_AGE_SECS: u64 = 7 * 86_400;

impl Default for CleanupPolicy {
    fn default() -> Self {
        Self {
            max_count: DEFAULT_CLEANUP_MAX_COUNT,
            max_age_secs: DEFAULT_CLEANUP_MAX_AGE_SECS,
        }
    }
}

/// Event sink placeholder so the store layer can notify observers without a
/// tauri dependency. Non-tauri builds use [`NoopEventSink`]; the tauri
/// feature in pla-app provides an emitting implementation.
pub trait SnapshotEventSink: Send + Sync {
    fn snapshot_created(&self, doc_key: &str, meta: &SnapshotMeta);
}

/// No-op sink for non-tauri builds and tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopEventSink;

impl SnapshotEventSink for NoopEventSink {
    fn snapshot_created(&self, _doc_key: &str, _meta: &SnapshotMeta) {}
}

/// Errors for the snapshot store.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("snapshot not found: {0}")]
    NotFound(String),
    #[error("invalid label: {0:?} (empty after sanitize)")]
    InvalidLabel(String),
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to open snapshot {path} with system handler: {source}")]
    OpenFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("metadata codec error: {0}")]
    Codec(#[from] serde_json::Error),
}

impl SnapshotError {
    fn io(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        move |source| SnapshotError::Io { path, source }
    }
}

/// File-backed snapshot store rooted at an injectable base directory
/// (the `data/` dir; tests inject a temp dir).
pub struct SnapshotStore {
    base: PathBuf,
}

impl SnapshotStore {
    /// `base` is the `data/` directory; snapshots live under
    /// `data/<doc_filename>/snapshots/`.
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    /// Base directory (the `data/` dir), for cleanup sweeps.
    pub fn base(&self) -> &Path {
        &self.base
    }

    fn snapshots_dir(&self, doc_filename: &str) -> PathBuf {
        self.base.join(doc_filename).join("snapshots")
    }

    /// Copy the current `.plasticity` file into the store and write a sibling
    /// `.json` metadata file (manual trigger). Returns the metadata.
    pub fn snapshot_create(
        &self,
        source_plasticity: &Path,
        doc_filename: &str,
        label: &str,
        entity_count: u64,
    ) -> Result<SnapshotMeta, SnapshotError> {
        self.snapshot_create_with_trigger(
            source_plasticity,
            doc_filename,
            label,
            entity_count,
            TriggerKind::Manual,
        )
    }

    /// [`SnapshotStore::snapshot_create`] with an explicit trigger kind
    /// (Timed / StepCounter from M2 trigger impls).
    pub fn snapshot_create_with_trigger(
        &self,
        source_plasticity: &Path,
        doc_filename: &str,
        label: &str,
        entity_count: u64,
        trigger: TriggerKind,
    ) -> Result<SnapshotMeta, SnapshotError> {
        let label =
            sanitize_label(label).ok_or_else(|| SnapshotError::InvalidLabel(label.to_string()))?;
        let ts = rfc3339_now();
        let stem = format!("{}-{}", ts_file_component(&ts), label);
        let dir = self.snapshots_dir(doc_filename);
        fs::create_dir_all(&dir).map_err(SnapshotError::io(&dir))?;
        let snap_path = dir.join(format!("{}.plasticity", stem));
        fs::copy(source_plasticity, &snap_path).map_err(SnapshotError::io(&snap_path))?;
        let meta = SnapshotMeta {
            ts,
            label,
            entity_count,
            doc_filename: doc_filename.to_string(),
            trigger,
            id: stem.clone(),
            pinned: false,
        };
        let meta_path = dir.join(format!("{}.json", stem));
        let json = serde_json::to_string_pretty(&meta)?;
        fs::write(&meta_path, json).map_err(SnapshotError::io(&meta_path))?;
        Ok(meta)
    }

    /// List all snapshots for one document, newest first. Missing directory
    /// yields an empty vec. Backfills `id` from the filename for old
    /// metadata that predates the field.
    pub fn snapshot_list(&self, doc_filename: &str) -> Vec<SnapshotMeta> {
        let dir = self.snapshots_dir(doc_filename);
        let mut metas = Vec::new();
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => return metas,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if let Ok(text) = fs::read_to_string(&path) {
                if let Ok(mut meta) = serde_json::from_str::<SnapshotMeta>(&text) {
                    if meta.id.is_empty() {
                        meta.id = stem;
                    }
                    metas.push(meta);
                }
            }
        }
        metas.sort_by(|a, b| b.ts.cmp(&a.ts));
        metas
    }

    /// Resolve a snapshot id (file stem) to its `.plasticity` path.
    fn snapshot_path(
        &self,
        doc_filename: &str,
        snapshot_id: &str,
    ) -> Result<PathBuf, SnapshotError> {
        let dir = self.base.join(doc_filename).join("snapshots");
        let path = dir.join(format!("{}.plasticity", snapshot_id));
        if !path.exists() {
            return Err(SnapshotError::NotFound(format!(
                "{}/{}",
                doc_filename, snapshot_id
            )));
        }
        Ok(path)
    }

    /// Restore: open the snapshot file with the system default app (new
    /// window). Does NOT close the current window. Errors surface as
    /// [`SnapshotError::OpenFailed`]; an explicit "open with Plasticity"
    /// fallback path is config-level and out of scope here.
    pub fn snapshot_restore(&self, doc_filename: &str, snapshot_id: &str) -> Result<(), SnapshotError> {
        let path = self.snapshot_path(doc_filename, snapshot_id)?;
        open::that(&path).map_err(|source| SnapshotError::OpenFailed { path, source })
    }

    /// Delete a snapshot pair (`.plasticity` + `.json`). Missing snapshot
    /// file is an error.
    pub fn snapshot_delete(&self, doc_filename: &str, snapshot_id: &str) -> Result<(), SnapshotError> {
        let path = self.snapshot_path(doc_filename, snapshot_id)?;
        fs::remove_file(&path).map_err(SnapshotError::io(&path))?;
        let meta_path = path.with_extension("json");
        if meta_path.exists() {
            fs::remove_file(&meta_path).map_err(SnapshotError::io(&meta_path))?;
        }
        Ok(())
    }

    /// Set/clear the `pinned` flag on a snapshot's metadata. Pinned
    /// snapshots are exempt from [`SnapshotStore::cleanup`].
    pub fn snapshot_pin(
        &self,
        doc_filename: &str,
        snapshot_id: &str,
        pinned: bool,
    ) -> Result<SnapshotMeta, SnapshotError> {
        let dir = self.snapshots_dir(doc_filename);
        let meta_path = dir.join(format!("{}.json", snapshot_id));
        if !meta_path.exists() {
            return Err(SnapshotError::NotFound(snapshot_id.to_string()));
        }
        let text = fs::read_to_string(&meta_path).map_err(SnapshotError::io(&meta_path))?;
        let mut meta: SnapshotMeta = serde_json::from_str(&text)?;
        meta.pinned = pinned;
        if meta.id.is_empty() {
            meta.id = snapshot_id.to_string();
        }
        let json = serde_json::to_string_pretty(&meta)?;
        fs::write(&meta_path, json).map_err(SnapshotError::io(&meta_path))?;
        Ok(meta)
    }

    /// Apply `policy` to one document: delete expired unpinned snapshots,
    /// then delete oldest unpinned until at most `max_count` remain.
    /// Returns the deleted snapshot ids (oldest first).
    pub fn cleanup(&self, doc_filename: &str, policy: &CleanupPolicy) -> Result<Vec<String>, SnapshotError> {
        let mut entries: Vec<(SnapshotMeta, SystemTime)> = Vec::new();
        for meta in self.snapshot_list(doc_filename) {
            let snap = self.snapshots_dir(doc_filename).join(format!("{}.plasticity", meta.id));
            let mtime = fs::metadata(&snap)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            entries.push((meta, mtime));
        }
        let now = SystemTime::now();
        let mut deleted = Vec::new();

        // Pass 1: age expiry (pinned exempt).
        let retain: Vec<(SnapshotMeta, SystemTime)> = entries
            .into_iter()
            .filter(|(meta, mtime)| {
                let expired = now
                    .duration_since(*mtime)
                    .map(|d| d.as_secs() > policy.max_age_secs)
                    .unwrap_or(false);
                if expired && !meta.pinned {
                    deleted.push(meta.id.clone());
                    let _ = self.snapshot_delete(doc_filename, &meta.id);
                    false
                } else {
                    true
                }
            })
            .collect();

        // Pass 2: count cap (oldest first, pinned exempt).
        let mut remaining: Vec<(SnapshotMeta, SystemTime)> = retain;
        remaining.sort_by_key(|(_, mtime)| *mtime); // oldest first
        while remaining.len() > policy.max_count {
            let idx = remaining.iter().position(|(meta, _)| !meta.pinned);
            match idx {
                Some(i) => {
                    let (meta, _) = remaining.remove(i);
                    let _ = self.snapshot_delete(doc_filename, &meta.id);
                    deleted.push(meta.id.clone());
                }
                None => break, // only pinned left; stop
            }
        }
        Ok(deleted)
    }

    /// Bridge-flavoured create: derive `entity_count` best-effort from
    /// `list_all`; bridge failure must not block the save (count defaults
    /// to 0). `doc_key` is used as the doc filename (D5 isolation key).
    pub async fn snapshot_create_from_bridge(
        &self,
        client: &pla_bridge::connection::BridgeClient,
        doc_key: &str,
        source_plasticity: &Path,
        label: &str,
    ) -> Result<SnapshotMeta, SnapshotError> {
        let entity_count = match client.list_all(doc_key).await {
            Ok(value) => extract_entity_count(&value).unwrap_or(0),
            Err(_) => 0,
        };
        self.snapshot_create(source_plasticity, doc_key, label, entity_count)
    }
}

/// Sweep every doc directory under `base` with `policy`. Suitable for a
/// tokio `spawn`+`interval` periodic task (call via `spawn_blocking` or
/// accept the small blocking scan). Returns doc_filename -> deleted ids.
pub fn cleanup_all(base: &Path, policy: &CleanupPolicy) -> HashMap<String, Vec<String>> {
    let mut result = HashMap::new();
    let Ok(entries) = fs::read_dir(base) else {
        return result;
    };
    let store = SnapshotStore::new(base);
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Ok(deleted) = store.cleanup(name, policy) {
            if !deleted.is_empty() {
                result.insert(name.to_string(), deleted);
            }
        }
    }
    result
}

/// Keep `[A-Za-z0-9-_]`; map everything else to `-`. Returns `None` when the
/// result is empty (invalid label).
fn sanitize_label(label: &str) -> Option<String> {
    let sanitized: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

/// RFC3339 UTC timestamp (second precision, no external chrono dep).
fn rfc3339_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339_from_unix(secs)
}

fn rfc3339_from_unix(secs: u64) -> String {
    let days = secs / 86_400;
    let tod = secs % 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

/// File-name-safe component matching `<yyyy-mm-ddTHHMMSS>`: keep digits,
/// `-` and the `T` separator (strips `:` and the trailing `Z`).
fn ts_file_component(ts: &str) -> String {
    ts.chars()
        .filter(|c| c.is_ascii_digit() || *c == '-' || *c == 'T')
        .collect()
}

/// Howard Hinnant's civil-from-days algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Best-effort entity-count extraction from a `list_all` JSON payload:
/// any "count" integer, or the length of any "entities" array.
fn extract_entity_count(value: &serde_json::Value) -> Option<u64> {
    if let Some(count) = value.get("count").and_then(|v| v.as_u64()) {
        return Some(count);
    }
    if let Some(len) = value
        .get("entities")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
    {
        return Some(len as u64);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    fn temp_base(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "pla-snapshot-test-{}-{}-{}",
            tag,
            std::process::id(),
            rfc3339_now().replace(':', "")
        ));
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn write_source(tag: &str) -> PathBuf {
        let src = std::env::temp_dir().join(format!(
            "pla-snapshot-src-{}-{}-{}.plasticity",
            tag,
            std::process::id(),
            rfc3339_now().replace(':', "")
        ));
        fs::write(&src, b"plasticity-bytes").unwrap();
        src
    }

    /// Create `n` snapshots with distinct mtimes (spaced by `step_secs`).
    fn create_n(store: &SnapshotStore, src: &Path, doc: &str, n: usize, step_secs: u64) -> Vec<SnapshotMeta> {
        let mut metas = Vec::new();
        for i in 0..n {
            let meta = store
                .snapshot_create(src, doc, &format!("s{}", i), i as u64)
                .unwrap();
            // Distinct mtime per snapshot so ordering tests are deterministic.
            let snap = store
                .base()
                .join(doc)
                .join("snapshots")
                .join(format!("{}.plasticity", meta.id));
            let t = std::time::SystemTime::now()
                - std::time::Duration::from_secs((n - i) as u64 * step_secs);
            let f = File::options().write(true).open(&snap).unwrap();
            f.set_times(
                std::fs::FileTimes::new().set_modified(t),
            )
            .unwrap();
            metas.push(meta);
        }
        metas
    }

    #[test]
    fn create_writes_copy_and_meta() {
        let base = temp_base("create");
        let src = write_source("create");
        let store = SnapshotStore::new(&base);
        let meta = store
            .snapshot_create(&src, "doc.plasticity", "pre-refactor", 42)
            .unwrap();
        assert_eq!(meta.label, "pre-refactor");
        assert_eq!(meta.entity_count, 42);
        assert_eq!(meta.doc_filename, "doc.plasticity");
        assert_eq!(meta.trigger, TriggerKind::Manual);
        assert!(!meta.pinned);
        assert!(!meta.id.is_empty());
        let dir = base.join("doc.plasticity").join("snapshots");
        let plasticity: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().unwrap_or_default() == "plasticity")
            .collect();
        assert_eq!(plasticity.len(), 1);
        assert_eq!(
            fs::read_to_string(plasticity[0].path()).unwrap(),
            "plasticity-bytes"
        );
        let json: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().unwrap_or_default() == "json")
            .collect();
        assert_eq!(json.len(), 1);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn create_with_trigger_records_kind() {
        let base = temp_base("triggerkind");
        let src = write_source("triggerkind");
        let store = SnapshotStore::new(&base);
        let meta = store
            .snapshot_create_with_trigger(&src, "doc.plasticity", "auto", 7, TriggerKind::Timed)
            .unwrap();
        assert_eq!(meta.trigger, TriggerKind::Timed);
        let listed = store.snapshot_list("doc.plasticity");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].trigger, TriggerKind::Timed);
        assert_eq!(listed[0].id, meta.id);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn list_orders_desc_and_missing_dir_is_empty() {
        let base = temp_base("list");
        let store = SnapshotStore::new(&base);
        assert!(store.snapshot_list("nope.plasticity").is_empty());
        let src = write_source("list");
        let first = store.snapshot_create(&src, "doc.plasticity", "a", 1).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let second = store.snapshot_create(&src, "doc.plasticity", "b", 2).unwrap();
        let list = store.snapshot_list("doc.plasticity");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].ts, second.ts);
        assert_eq!(list[0].label, "b");
        assert_eq!(list[1].ts, first.ts);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn delete_removes_pair_and_errors_when_missing() {
        let base = temp_base("delete");
        let src = write_source("delete");
        let store = SnapshotStore::new(&base);
        let meta = store.snapshot_create(&src, "doc.plasticity", "gone", 0).unwrap();
        store.snapshot_delete("doc.plasticity", &meta.id).unwrap();
        assert!(store.snapshot_list("doc.plasticity").is_empty());
        let err = store.snapshot_delete("doc.plasticity", &meta.id).unwrap_err();
        assert!(matches!(err, SnapshotError::NotFound(_)));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn restore_missing_snapshot_is_not_found() {
        let base = temp_base("restore404");
        let store = SnapshotStore::new(&base);
        let err = store
            .snapshot_restore("doc.plasticity", "does-not-exist")
            .unwrap_err();
        assert!(matches!(err, SnapshotError::NotFound(_)));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn label_sanitize_maps_bad_chars() {
        let base = temp_base("sanitize");
        let src = write_source("sanitize");
        let store = SnapshotStore::new(&base);
        let meta = store
            .snapshot_create(&src, "doc.plasticity", "my label/2:0!", 0)
            .unwrap();
        assert_eq!(meta.label, "my-label-2-0-");
        let dir = base.join("doc.plasticity").join("snapshots");
        let name = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .find(|e| e.path().extension().unwrap_or_default() == "plasticity")
            .unwrap()
            .file_name()
            .to_str()
            .unwrap()
            .to_string();
        assert!(name.contains("my-label-2-0-"), "name was {}", name);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn empty_label_rejected() {
        let base = temp_base("empty");
        let src = write_source("empty");
        let store = SnapshotStore::new(&base);
        for bad in [""] {
            let err = store
                .snapshot_create(&src, "doc.plasticity", bad, 0)
                .unwrap_err();
            assert!(matches!(err, SnapshotError::InvalidLabel(_)), "label {:?}", bad);
        }
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn extract_entity_count_shapes() {
        assert_eq!(
            extract_entity_count(&serde_json::json!({"count": 7})),
            Some(7)
        );
        assert_eq!(
            extract_entity_count(&serde_json::json!({"entities": [1, 2, 3]})),
            Some(3)
        );
        assert_eq!(
            extract_entity_count(&serde_json::json!({"other": 1})),
            None
        );
    }

    // ---- M2: triggers -------------------------------------------------

    #[test]
    fn timer_trigger_fires_on_interval_with_injected_now() {
        let mut t = TimerTrigger::new(300);
        let ctx = |now: u64| TriggerContext {
            doc_key: "doc.plasticity",
            now_secs: now,
            new_version_count: 0,
        };
        // First tick fires immediately and records last_fire.
        assert!(t.tick(&ctx(1000)));
        assert_eq!(t.last_fire, Some(1000));
        // Interval not elapsed: no fire, last_fire unchanged.
        assert!(!t.tick(&ctx(1299)));
        assert_eq!(t.last_fire, Some(1000));
        // At exactly interval: fires and updates.
        assert!(t.tick(&ctx(1300)));
        assert_eq!(t.last_fire, Some(1300));
        assert!(!t.tick(&ctx(1400)));
    }

    #[test]
    fn timer_trigger_default_interval_is_5min() {
        assert_eq!(TimerTrigger::default().interval_secs, 300);
        assert_eq!(DEFAULT_TIMER_INTERVAL_SECS, 300);
    }

    #[test]
    fn step_count_trigger_threshold() {
        let mut t = StepCountTrigger::new(10);
        let ctx = |n: u64| TriggerContext {
            doc_key: "doc.plasticity",
            now_secs: 0,
            new_version_count: n,
        };
        assert!(!t.tick(&ctx(9)));
        assert!(t.tick(&ctx(10)));
        assert!(t.tick(&ctx(11)));
    }

    #[test]
    fn step_count_trigger_docs_are_independent() {
        let mut t = StepCountTrigger::new(5);
        let a = TriggerContext { doc_key: "a.plasticity", now_secs: 0, new_version_count: 4 };
        let b = TriggerContext { doc_key: "b.plasticity", now_secs: 0, new_version_count: 9 };
        assert!(!t.tick(&a));
        assert!(t.tick(&b));
    }

    // ---- M2: cleanup --------------------------------------------------

    #[test]
    fn cleanup_enforces_max_count_oldest_first() {
        let base = temp_base("cleanup-count");
        let src = write_source("cleanup-count");
        let store = SnapshotStore::new(&base);
        let metas = create_n(&store, &src, "doc.plasticity", 3, 3600);
        let policy = CleanupPolicy {
            max_count: 2,
            max_age_secs: DEFAULT_CLEANUP_MAX_AGE_SECS,
        };
        let deleted = store.cleanup("doc.plasticity", &policy).unwrap();
        assert_eq!(deleted, vec![metas[0].id.clone()]); // oldest
        let left = store.snapshot_list("doc.plasticity");
        assert_eq!(left.len(), 2);
        assert!(!left.iter().any(|m| m.id == metas[0].id));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn cleanup_respects_pinned() {
        let base = temp_base("cleanup-pin");
        let src = write_source("cleanup-pin");
        let store = SnapshotStore::new(&base);
        let metas = create_n(&store, &src, "doc.plasticity", 3, 3600);
        // Pin the oldest so it survives the count cap.
        store
            .snapshot_pin("doc.plasticity", &metas[0].id, true)
            .unwrap();
        let policy = CleanupPolicy {
            max_count: 2,
            max_age_secs: DEFAULT_CLEANUP_MAX_AGE_SECS,
        };
        let deleted = store.cleanup("doc.plasticity", &policy).unwrap();
        assert_eq!(deleted, vec![metas[1].id.clone()]); // next-oldest unpinned
        let left = store.snapshot_list("doc.plasticity");
        assert!(left.iter().any(|m| m.id == metas[0].id && m.pinned));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn cleanup_deletes_expired_by_mtime() {
        let base = temp_base("cleanup-age");
        let src = write_source("cleanup-age");
        let store = SnapshotStore::new(&base);
        let metas = create_n(&store, &src, "doc.plasticity", 2, 3600);
        // Age s0 to 8 days old so it exceeds the 7-day max_age; s1 stays fresh.
        let s0_path = store
            .base()
            .join("doc.plasticity")
            .join("snapshots")
            .join(format!("{}.plasticity", metas[0].id));
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 86_400);
        File::options()
            .write(true)
            .open(&s0_path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
        let policy = CleanupPolicy {
            max_count: DEFAULT_CLEANUP_MAX_COUNT,
            max_age_secs: 7 * 86_400,
        };
        let deleted = store.cleanup("doc.plasticity", &policy).unwrap();
        assert_eq!(deleted, vec![metas[0].id.clone()]);
        assert_eq!(store.snapshot_list("doc.plasticity").len(), 1);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn cleanup_all_sweeps_every_doc_dir() {
        let base = temp_base("cleanup-all");
        let src = write_source("cleanup-all");
        let store = SnapshotStore::new(&base);
        create_n(&store, &src, "a.plasticity", 1, 3600);
        create_n(&store, &src, "b.plasticity", 1, 3600);
        let policy = CleanupPolicy {
            max_count: DEFAULT_CLEANUP_MAX_COUNT,
            max_age_secs: 7 * 86_400,
        };
        // Fresh mtimes => nothing expired, nothing over cap => nothing swept.
        let result = cleanup_all(&base, &policy);
        assert!(result.is_empty());
        assert_eq!(store.snapshot_list("a.plasticity").len(), 1);
        assert_eq!(store.snapshot_list("b.plasticity").len(), 1);
        fs::remove_dir_all(&base).ok();
    }

    // ---- M2: pin + metadata compat ------------------------------------

    #[test]
    fn pin_roundtrip_and_missing_is_not_found() {
        let base = temp_base("pin");
        let src = write_source("pin");
        let store = SnapshotStore::new(&base);
        let meta = store.snapshot_create(&src, "doc.plasticity", "keep", 1).unwrap();
        let updated = store
            .snapshot_pin("doc.plasticity", &meta.id, true)
            .unwrap();
        assert!(updated.pinned);
        let listed = store.snapshot_list("doc.plasticity");
        assert_eq!(listed[0].pinned, true);
        let cleared = store
            .snapshot_pin("doc.plasticity", &meta.id, false)
            .unwrap();
        assert!(!cleared.pinned);
        let err = store
            .snapshot_pin("doc.plasticity", "no-such-id", true)
            .unwrap_err();
        assert!(matches!(err, SnapshotError::NotFound(_)));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn old_metadata_without_pinned_or_id_deserializes() {
        let old = serde_json::json!({
            "ts": "2025-01-01T00:00:00Z",
            "label": "legacy",
            "entity_count": 3,
            "doc_filename": "doc.plasticity",
            "trigger": "manual"
        });
        let meta: SnapshotMeta = serde_json::from_str(&old.to_string()).unwrap();
        assert!(!meta.pinned);
        assert_eq!(meta.id, "");
        assert_eq!(meta.trigger, TriggerKind::Manual);
    }

    #[test]
    fn metadata_roundtrip_includes_pinned_and_id() {
        let meta = SnapshotMeta {
            ts: "2025-01-01T00:00:00Z".into(),
            label: "roundtrip".into(),
            entity_count: 5,
            doc_filename: "doc.plasticity".into(),
            trigger: TriggerKind::StepCounter,
            id: "stem-1".into(),
            pinned: true,
        };
        let text = serde_json::to_string(&meta).unwrap();
        let back: SnapshotMeta = serde_json::from_str(&text).unwrap();
        assert!(back.pinned);
        assert_eq!(back.id, "stem-1");
        assert_eq!(back.trigger, TriggerKind::StepCounter);
    }

    // ---- M2: filename isolation ----------------------------------------

    #[test]
    fn docs_are_isolated_by_filename() {
        let base = temp_base("isolation");
        let src = write_source("isolation");
        let store = SnapshotStore::new(&base);
        store.snapshot_create(&src, "a.plasticity", "only-a", 1).unwrap();
        store.snapshot_create(&src, "b.plasticity", "only-b", 2).unwrap();
        let a = store.snapshot_list("a.plasticity");
        let b = store.snapshot_list("b.plasticity");
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_eq!(a[0].label, "only-a");
        assert_eq!(b[0].label, "only-b");
        // Deleting in a leaves b untouched; a's id invisible in b.
        let a_id = a[0].id.clone();
        store.snapshot_delete("a.plasticity", &a_id).unwrap();
        assert_eq!(store.snapshot_list("b.plasticity").len(), 1);
        let err = store.snapshot_delete("b.plasticity", &a_id).unwrap_err();
        assert!(matches!(err, SnapshotError::NotFound(_)));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn noop_event_sink_is_a_sink() {
        let sink = NoopEventSink;
        sink.snapshot_created("doc.plasticity", &SnapshotMeta {
            ts: "2025-01-01T00:00:00Z".into(),
            label: "x".into(),
            entity_count: 0,
            doc_filename: "doc.plasticity".into(),
            trigger: TriggerKind::Manual,
            id: "stem".into(),
            pinned: false,
        });
    }

    #[test]
    fn rfc3339_known_value() {
        assert_eq!(rfc3339_from_unix(0), "1970-01-01T00:00:00Z");
    }
}
