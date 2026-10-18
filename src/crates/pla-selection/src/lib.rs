//! pla-selection: P0-2 selection sets for PLA-003 (M3).
//!
//! Selection set = `{name, doc_filename, members: [entity ids]}` (weak
//! references; arch §3.2/D3). `apply` performs an existence check against a
//! fetched [`EntityTree`] first; missing members are reported in
//! [`ApplyReport`] (UI marks them yellow) and never silently dropped. Only
//! when all members exist does `apply` issue a PUT_SOME that creates a
//! same-named group with the members injected. Storage lives at
//! `data/<doc_filename>/selections/<set>.json`.
//!
//! The entity tree is cached in memory per `doc_key` with a version stamp
//! (arch §4.1/D5); NEW_VERSION events / reconnects invalidate the cache and
//! the next fetch replays LIST_ALL.
//!
//! All bridge payload shapes are TODO-PROBE assumptions (see pla-bridge
//! protocol header note) and kept lenient via `serde(default)`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

// ---- EntityTree model ------------------------------------------------------

/// One entity (node) in the document entity tree. TODO-PROBE: field shapes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntityNode {
    /// Stable entity id used as the weak reference in selection members.
    pub id: String,
    /// Entity kind, e.g. `solid`, `sketch`, `fillet`. TODO-PROBE literals.
    #[serde(default)]
    pub kind: String,
    /// Display name.
    #[serde(default)]
    pub name: String,
    /// Nested children (groups/components).
    #[serde(default)]
    pub children: Vec<EntityNode>,
}

/// Entity tree for one document, with a cache version stamp.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntityTree {
    /// Isolation key (document filename, D5).
    #[serde(default)]
    pub doc_key: String,
    /// Cache version stamp; bumped on every (re)fetch after invalidation.
    #[serde(default)]
    pub version: u64,
    /// Top-level entities.
    #[serde(default)]
    pub entities: Vec<EntityNode>,
}

impl EntityTree {
    /// Walk the tree depth-first, pushing every entity id.
    pub fn collect_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        fn walk(nodes: &[EntityNode], ids: &mut Vec<String>) {
            for n in nodes {
                ids.push(n.id.clone());
                walk(&n.children, ids);
            }
        }
        walk(&self.entities, &mut ids);
        ids
    }

    /// Depth-first node lookup by id.
    pub fn find(&self, id: &str) -> Option<&EntityNode> {
        fn walk<'a>(nodes: &'a [EntityNode], id: &str) -> Option<&'a EntityNode> {
            for n in nodes {
                if n.id == id {
                    return Some(n);
                }
                if let Some(found) = walk(&n.children, id) {
                    return Some(found);
                }
            }
            None
        }
        walk(&self.entities, id)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.find(id).is_some()
    }

    /// Members that no longer exist in the tree (weak-reference decay check).
    pub fn missing_members(&self, members: &[String]) -> Vec<String> {
        members
            .iter()
            .filter(|m| !self.contains(m))
            .cloned()
            .collect()
    }
}

/// Parse an EntityTree from a `LIST_ALL` response payload (lenient):
/// accepts `{entities: [...]}` or `{payload: {entities: [...]}}`, and any
/// extra fields are ignored. TODO-PROBE: real payload shape.
pub fn entity_tree_from_payload(doc_key: &str, value: &serde_json::Value) -> EntityTree {
    let root = value.get("payload").unwrap_or(value);
    let mut tree: EntityTree = root
        .get("entities")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .map(|entities: Vec<EntityNode>| EntityTree {
            doc_key: doc_key.to_string(),
            version: 0,
            entities,
        })
        .unwrap_or_else(|| EntityTree {
            doc_key: doc_key.to_string(),
            version: 0,
            entities: Vec::new(),
        });
    tree.version = root
        .get("version")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    tree
}

// ---- Per-doc_key cache with version stamps ---------------------------------

#[derive(Debug, Clone)]
struct CachedDoc {
    tree: Option<EntityTree>,
    /// Monotonic cache-generation counter; stamped into fetched trees.
    generation: u64,
}

/// In-memory entity-tree cache, namespaced by `doc_key` (D5). NEW_VERSION
/// events and reconnects call [`EntityCache::invalidate`]; the next
/// [`EntityCache::fetch`] replays LIST_ALL with a fresh version stamp.
#[derive(Debug, Default)]
pub struct EntityCache {
    docs: Mutex<HashMap<String, CachedDoc>>,
}

impl EntityCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cached tree for `doc_key`, if present and not invalidated.
    pub fn get(&self, doc_key: &str) -> Option<EntityTree> {
        self.docs
            .lock()
            .ok()?
            .get(doc_key)
            .and_then(|d| d.tree.clone())
    }

    /// Insert/refresh a cached tree (bumps the generation stamp).
    pub fn put(&self, tree: EntityTree) {
        if let Ok(mut docs) = self.docs.lock() {
            let entry = docs.entry(tree.doc_key.clone()).or_insert(CachedDoc {
                tree: None,
                generation: 0,
            });
            entry.generation += 1;
            let mut tree = tree;
            tree.version = entry.generation;
            entry.tree = Some(tree);
        }
    }

    /// Drop the cached tree (NEW_VERSION / reconnect); keeps the generation
    /// counter monotonic so the refetched tree gets a strictly newer stamp.
    pub fn invalidate(&self, doc_key: &str) {
        if let Ok(mut docs) = self.docs.lock() {
            if let Some(entry) = docs.get_mut(doc_key) {
                entry.tree = None;
            }
        }
    }

    /// Invalidate every doc (full reconnect).
    pub fn invalidate_all(&self) {
        if let Ok(mut docs) = self.docs.lock() {
            for entry in docs.values_mut() {
                entry.tree = None;
            }
        }
    }

    /// Fetch through the cache: serve the cached tree when valid, otherwise
    /// LIST_VISIBLE (code 22, full-scene tree) via the bridge and cache the
    /// parsed result. Falls back to LIST_ALL (top-level only) if the device
    /// rejects 22.
    ///
    /// Wire-verified 2026-10-20: code 22 returns nested records (tag=6 name
    /// blocks) for every scene object; code 20 only returns top-level rows.
    pub async fn fetch(
        &self,
        client: &pla_bridge::BridgeClient,
        doc_key: &str,
    ) -> Result<EntityTree, SelectionError> {
        if let Some(tree) = self.get(doc_key) {
            return Ok(tree);
        }
        let value = match client.list_visible(doc_key).await {
            Ok(v) => v,
            Err(_e) => {
                // No tracing dep in this crate; the fallback itself is the signal.
                client.list_all(doc_key).await?
            }
        };
        let tree = entity_tree_from_payload(doc_key, &value);
        self.put(tree.clone());
        // put() stamps the version on its own copy; serve that one so the
        // caller sees the cache generation, not the raw parsed tree.
        Ok(self.get(doc_key).unwrap_or(tree))
    }
}

// ---- Selection model + store -----------------------------------------------

/// A saved selection set (weak entity references; D3).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Selection {
    pub name: String,
    /// Isolation key (document filename, D5).
    pub doc_filename: String,
    /// Weak entity ids.
    pub members: Vec<String>,
}

/// Result of `apply`: existence check outcome. `ok=false` carries the
/// missing member ids; the UI marks them yellow (no silent drop, D3).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApplyReport {
    pub ok: bool,
    pub missing: Vec<String>,
}

/// Errors for the selection store.
#[derive(Debug, thiserror::Error)]
pub enum SelectionError {
    #[error("selection not found: {0}")]
    NotFound(String),
    #[error("invalid selection name: {0:?} (empty after sanitize)")]
    InvalidName(String),
    #[error("bridge unavailable: document must be saved first (doc_key is empty)")]
    UnsavedDoc,
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("codec error: {0}")]
    Codec(#[from] serde_json::Error),
    #[error("bridge error: {0}")]
    Bridge(#[from] pla_bridge::BridgeError),
}

impl SelectionError {
    fn io(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        move |source| SelectionError::Io { path, source }
    }
}

/// File-backed selection-set library rooted at the `data/` dir:
/// `data/<doc_filename>/selections/<set>.json`.
pub struct SelectionStore {
    base: PathBuf,
}

impl SelectionStore {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    pub fn base(&self) -> &std::path::Path {
        &self.base
    }

    fn selections_dir(&self, doc_filename: &str) -> PathBuf {
        self.base.join(doc_filename).join("selections")
    }

    /// List all doc_keys (filenames) that have at least one saved selection.
    /// Backs the `doc_list` command (架构 §5); empty when nothing stored.
    pub fn list_docs(&self) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(&self.base) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.join("selections").is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    out.push(name.to_string());
                }
            }
        }
        out.sort();
        out
    }

    /// Save (create or overwrite) a selection set. Returns the stored value.
    pub fn save(
        &self,
        doc_key: &str,
        name: &str,
        members: Vec<String>,
    ) -> Result<Selection, SelectionError> {
        if doc_key.is_empty() {
            return Err(SelectionError::UnsavedDoc);
        }
        let name = sanitize_name(name).ok_or_else(|| SelectionError::InvalidName(name.to_string()))?;
        let sel = Selection {
            name: name.clone(),
            doc_filename: doc_key.to_string(),
            members,
        };
        let dir = self.selections_dir(doc_key);
        fs::create_dir_all(&dir).map_err(SelectionError::io(&dir))?;
        let path = dir.join(format!("{}.json", name));
        let json = serde_json::to_string_pretty(&sel)?;
        fs::write(&path, json).map_err(SelectionError::io(path))?;
        Ok(sel)
    }

    /// Load one selection set by name.
    pub fn load(&self, doc_key: &str, name: &str) -> Result<Selection, SelectionError> {
        let path = self.selections_dir(doc_key).join(format!("{}.json", name));
        if !path.exists() {
            return Err(SelectionError::NotFound(format!(
                "{}/{}",
                doc_key, name
            )));
        }
        let text = fs::read_to_string(&path).map_err(SelectionError::io(&path))?;
        Ok(serde_json::from_str(&text)?)
    }

    /// List all saved selection sets for one document (sorted by name).
    pub fn list(&self, doc_key: &str) -> Vec<Selection> {
        let dir = self.selections_dir(doc_key);
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(&dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(text) = fs::read_to_string(&path) {
                if let Ok(sel) = serde_json::from_str::<Selection>(&text) {
                    out.push(sel);
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Delete one selection set. Missing file is an error.
    pub fn delete(&self, doc_key: &str, name: &str) -> Result<(), SelectionError> {
        let path = self.selections_dir(doc_key).join(format!("{}.json", name));
        if !path.exists() {
            return Err(SelectionError::NotFound(format!(
                "{}/{}",
                doc_key, name
            )));
        }
        fs::remove_file(&path).map_err(SelectionError::io(&path))?;
        // Prune the selections dir (and the doc dir when empty) so doc_list
        // reflects only docs that still own at least one selection.
        let dir = self.selections_dir(doc_key);
        if fs::read_dir(&dir).map(|mut d| d.next().is_none()).unwrap_or(false) {
            fs::remove_dir(&dir).ok();
            if let Some(doc_dir) = dir.parent() {
                if fs::read_dir(doc_dir).map(|mut d| d.next().is_none()).unwrap_or(false) {
                    fs::remove_dir(doc_dir).ok();
                }
            }
        }
        Ok(())
    }

    /// Apply a saved selection set (arch D3): existence-check all members
    /// against `tree`; if any are missing, return `ok=false` with the
    /// missing ids and DO NOT touch the document. Otherwise PUT_SOME with a
    /// create-group payload (TODO-PROBE shape) injecting the members.
    pub async fn apply(
        &self,
        client: &pla_bridge::BridgeClient,
        doc_key: &str,
        tree: &EntityTree,
        name: &str,
    ) -> Result<ApplyReport, SelectionError> {
        let sel = self.load(doc_key, name)?;
        let missing = tree.missing_members(&sel.members);
        if !missing.is_empty() {
            // Weak-reference decay: surface missing ids, never silently drop.
            return Ok(ApplyReport {
                ok: false,
                missing,
            });
        }
        // TODO-PROBE: PUT_SOME create-group payload shape.
        let data = serde_json::json!({
            "op": "CREATE_GROUP",
            "group": sel.name,
            "members": sel.members,
        });
        client.put_some(doc_key, data).await?;
        Ok(ApplyReport {
            ok: true,
            missing: Vec::new(),
        })
    }
}

/// Keep `[A-Za-z0-9-_]`; map everything else to `-` (same policy as
/// pla-snapshot labels). `None` when the result is empty.
fn sanitize_name(name: &str) -> Option<String> {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    // Require at least one alphanumeric char so pure-invalid input ("///",
    // "   ") doesn't sanitize into a legal all-dash filename.
    if sanitized.is_empty() || !sanitized.chars().any(|c| c.is_ascii_alphanumeric()) {
        None
    } else {
        Some(sanitized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_walk_find_and_missing() {
        let tree = EntityTree {
            doc_key: "doc.plasticity".into(),
            version: 1,
            entities: vec![
                EntityNode {
                    id: "e1".into(),
                    kind: "solid".into(),
                    name: "Body".into(),
                    children: vec![EntityNode {
                        id: "e1-1".into(),
                        kind: "face".into(),
                        name: "F".into(),
                        children: vec![],
                    }],
                },
                EntityNode {
                    id: "e2".into(),
                    kind: "sketch".into(),
                    name: "S".into(),
                    children: vec![],
                },
            ],
        };
        assert_eq!(tree.collect_ids(), vec!["e1", "e1-1", "e2"]);
        assert!(tree.contains("e1-1"));
        assert_eq!(tree.find("e2").unwrap().kind, "sketch");
        assert_eq!(
            tree.missing_members(&["e1".into(), "gone".into(), "e2".into()]),
            vec!["gone".to_string()]
        );
    }

    #[test]
    fn parse_payload_lenient_shapes() {
        let a = serde_json::json!({
            "entities": [{"id": "a", "kind": "solid", "name": "A"}],
            "version": 3
        });
        let tree = entity_tree_from_payload("d.pla", &a);
        assert_eq!(tree.collect_ids(), vec!["a"]);
        assert_eq!(tree.version, 3);
        // nested under "payload" + missing fields tolerated
        let b = serde_json::json!({"payload": {"entities": [{"id": "b"}]}});
        let tree = entity_tree_from_payload("d.pla", &b);
        assert_eq!(tree.collect_ids(), vec!["b"]);
        assert_eq!(tree.entities[0].kind, "");
        // unknown shape -> empty tree, not a panic
        let c = serde_json::json!({"unexpected": true});
        assert!(entity_tree_from_payload("d.pla", &c).entities.is_empty());
    }

    #[test]
    fn cache_invalidate_bumps_generation() {
        let cache = EntityCache::new();
        let tree = EntityTree {
            doc_key: "d.pla".into(),
            version: 0,
            entities: vec![],
        };
        cache.put(tree);
        assert_eq!(cache.get("d.pla").unwrap().version, 1);
        cache.invalidate("d.pla");
        assert!(cache.get("d.pla").is_none());
        cache.put(EntityTree {
            doc_key: "d.pla".into(),
            version: 0,
            entities: vec![],
        });
        assert_eq!(cache.get("d.pla").unwrap().version, 2);
        cache.invalidate_all();
        assert!(cache.get("d.pla").is_none());
    }

    #[test]
    fn store_crud_roundtrip_and_isolation() {
        let base = std::env::temp_dir().join(format!(
            "pla-sel-crud-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = SelectionStore::new(&base);
        let saved = store
            .save("a.plasticity", "my set", vec!["e1".into(), "e2".into()])
            .unwrap();
        assert_eq!(saved.name, "my-set");
        assert_eq!(saved.doc_filename, "a.plasticity");
        assert_eq!(store.list("a.plasticity").len(), 1);
        // overwrite same name
        store.save("a.plasticity", "my set", vec!["e1".into()]).unwrap();
        assert_eq!(store.list("a.plasticity").len(), 1);
        assert_eq!(store.load("a.plasticity", "my-set").unwrap().members, vec!["e1"]);
        // doc isolation (D5)
        assert!(store.list("b.plasticity").is_empty());
        let err = store.load("b.plasticity", "my-set").unwrap_err();
        assert!(matches!(err, SelectionError::NotFound(_)));
        // delete
        store.delete("a.plasticity", "my-set").unwrap();
        assert!(store.list("a.plasticity").is_empty());
        // invalid names / unsaved doc
        assert!(matches!(
            store.save("a.plasticity", "///", vec![]).unwrap_err(),
            SelectionError::InvalidName(_)
        ));
        assert!(matches!(
            store.save("", "x", vec![]).unwrap_err(),
            SelectionError::UnsavedDoc
        ));
        std::fs::remove_dir_all(&base).ok();
    }
}
