//! pla-dim: P1-1 dimension-driven edit for PLA-003 (M4, v1).
//!
//! A dim = selection set (reused from pla-selection, single source of
//! truth; arch §3.3/D4) + a unit direction vector + the current numeric
//! value. Storage: `data/<doc_filename>/dims/<dim>.json`. Executing an
//! update pulls the members' facet vertices from a LIST_ALL payload
//! (TODO-PROBE shape), translates them by `delta * dir`, and writes them
//! back via PUT_SOME as a replace (delete + recreate the objects).
//!
//! Topology-typed entities (fillet/chamfer radii) are HARD-rejected with
//! [`DimError::TopologyUnsupported`] — the error text spells this out for
//! the UI. Degraded path: when PUT_SOME is unavailable the dim driver is
//! read-only and returns a `ok=false` report with a hint instead of
//! silently doing nothing.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Entity kinds whose dimensions are topology-driven (radius of a
/// fillet/chamfer) and therefore NOT movable by vertex translation.
/// TODO-PROBE: real kind literals on the wire.
pub const TOPOLOGY_KINDS: [&str; 2] = ["fillet", "chamfer"];

/// Unit direction vector of a dim.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct DimDir {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl DimDir {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// +X default direction.
    pub const fn unit_x() -> Self {
        Self::new(1.0, 0.0, 0.0)
    }

    /// Normalized copy; a zero (or non-finite) vector maps to +X so a dim
    /// always has a well-defined direction.
    pub fn normalized(&self) -> Self {
        let (x, y, z) = (self.x, self.y, self.z);
        if !x.is_finite() || !y.is_finite() || !z.is_finite() {
            return Self::unit_x();
        }
        let len = (x * x + y * y + z * z).sqrt();
        if len < f64::EPSILON {
            Self::unit_x()
        } else {
            Self::new(x / len, y / len, z / len)
        }
    }
}

/// A saved dim definition (arch D4).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DimDef {
    pub name: String,
    /// Isolation key (document filename, D5).
    pub doc_filename: String,
    /// Referenced selection-set name (weak; reuses pla-selection sets).
    pub set_name: String,
    /// Unit direction vector (stored normalized).
    pub dir: DimDir,
    /// Current numeric value.
    pub value: f64,
}

/// Result of a dim update.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DimReport {
    pub ok: bool,
    /// Number of member entities replaced.
    pub moved: usize,
    /// Missing selection members (same semantics as pla-selection).
    pub missing: Vec<String>,
    /// Human-readable detail; non-empty when `ok=false` (degraded path /
    /// topology hint is carried by the error instead).
    #[serde(default)]
    pub message: String,
}

/// Errors for the dim driver.
#[derive(Debug, thiserror::Error)]
pub enum DimError {
    #[error(
        "topology dimension not supported: entity {member:?} (kind {kind:?}) is a fillet/chamfer; \
         radius changes are structural and cannot be driven by vertex translation"
    )]
    TopologyUnsupported { member: String, kind: String },
    #[error("dim not found: {0}")]
    NotFound(String),
    #[error("invalid dim name: {0:?} (empty after sanitize)")]
    InvalidName(String),
    #[error("bridge unavailable: document must be saved first (doc_key is empty)")]
    UnsavedDoc,
    #[error("selection error: {0}")]
    Set(#[from] pla_selection::SelectionError),
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("codec error: {0}")]
    Codec(#[from] serde_json::Error),
}

impl DimError {
    fn io(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        move |source| DimError::Io { path, source }
    }
}

// ---- pure translation -------------------------------------------------------

/// Translate one vertex by `delta * dir` (pure; unit testable).
pub fn translate_vertex(v: [f64; 3], delta: f64, dir: &DimDir) -> [f64; 3] {
    let dir = dir.normalized();
    [
        v[0] + delta * dir.x,
        v[1] + delta * dir.y,
        v[2] + delta * dir.z,
    ]
}

/// Translate a batch of vertices (pure).
pub fn translate_vertices(vertices: &[[f64; 3]], delta: f64, dir: &DimDir) -> Vec<[f64; 3]> {
    vertices
        .iter()
        .map(|v| translate_vertex(*v, delta, dir))
        .collect()
}

/// Extract all facet vertices from one LIST_ALL entity JSON node.
/// Assumed shape: `{id, kind, facets: [{vertices: [[x,y,z], ...]}, ...]}`.
/// Lenient: missing/odd fields yield an empty vec. TODO-PROBE.
pub fn facet_vertices_from_entity(entity: &serde_json::Value) -> Vec<[f64; 3]> {
    let mut out = Vec::new();
    let Some(facets) = entity.get("facets").and_then(|f| f.as_array()) else {
        return out;
    };
    for facet in facets {
        let Some(vertices) = facet.get("vertices").and_then(|v| v.as_array()) else {
            continue;
        };
        for v in vertices {
            let coords: Vec<f64> = v
                .as_array()
                .map(|a| a.iter().filter_map(|c| c.as_f64()).collect())
                .unwrap_or_default();
            if coords.len() == 3 {
                out.push([coords[0], coords[1], coords[2]]);
            }
        }
    }
    out
}

/// Flatten a LIST_ALL payload into `(id -> entity JSON)` (recursive walk,
/// children included). TODO-PROBE shape.
fn flatten_entities(value: &serde_json::Value) -> Vec<serde_json::Value> {
    let root = value.get("payload").unwrap_or(value);
    let mut out = Vec::new();
    fn walk(nodes: Option<&Vec<serde_json::Value>>, out: &mut Vec<serde_json::Value>) {
        let Some(nodes) = nodes else { return };
        for n in nodes {
            out.push(n.clone());
            walk(n.get("children").and_then(|c| c.as_array()), out);
        }
    }
    walk(root.get("entities").and_then(|e| e.as_array()), &mut out);
    out
}

// ---- dim store ---------------------------------------------------------------

/// File-backed dim library: `data/<doc_filename>/dims/<dim>.json`.
pub struct DimStore {
    base: PathBuf,
}

impl DimStore {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    pub fn base(&self) -> &std::path::Path {
        &self.base
    }

    fn dims_dir(&self, doc_filename: &str) -> PathBuf {
        self.base.join(doc_filename).join("dims")
    }

    /// Define (create or overwrite) a dim referencing a selection set.
    /// The direction is normalized on save.
    pub fn define(
        &self,
        doc_key: &str,
        name: &str,
        set_name: &str,
        dir: DimDir,
        value: f64,
    ) -> Result<DimDef, DimError> {
        if doc_key.is_empty() {
            return Err(DimError::UnsavedDoc);
        }
        let name = sanitize_name(name).ok_or_else(|| DimError::InvalidName(name.to_string()))?;
        let def = DimDef {
            name: name.clone(),
            doc_filename: doc_key.to_string(),
            set_name: set_name.to_string(),
            dir: dir.normalized(),
            value,
        };
        let dir_path = self.dims_dir(doc_key);
        fs::create_dir_all(&dir_path).map_err(DimError::io(&dir_path))?;
        let path = dir_path.join(format!("{}.json", name));
        let json = serde_json::to_string_pretty(&def)?;
        fs::write(&path, json).map_err(DimError::io(path))?;
        Ok(def)
    }

    pub fn load(&self, doc_key: &str, name: &str) -> Result<DimDef, DimError> {
        let path = self.dims_dir(doc_key).join(format!("{}.json", name));
        if !path.exists() {
            return Err(DimError::NotFound(format!("{}/{}", doc_key, name)));
        }
        let text = fs::read_to_string(&path).map_err(DimError::io(&path))?;
        Ok(serde_json::from_str(&text)?)
    }

    /// List all dims for one document (sorted by name).
    pub fn list(&self, doc_key: &str) -> Vec<DimDef> {
        let dir = self.dims_dir(doc_key);
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
                if let Ok(def) = serde_json::from_str::<DimDef>(&text) {
                    out.push(def);
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Persist an updated dim definition (new current value after a
    /// successful update).
    pub fn save(&self, def: &DimDef) -> Result<(), DimError> {
        self.define(
            &def.doc_filename,
            &def.name,
            &def.set_name,
            def.dir,
            def.value,
        )
        .map(|_| ())
    }

    pub fn delete(&self, doc_key: &str, name: &str) -> Result<(), DimError> {
        let path = self.dims_dir(doc_key).join(format!("{}.json", name));
        if !path.exists() {
            return Err(DimError::NotFound(format!("{}/{}", doc_key, name)));
        }
        fs::remove_file(&path).map_err(DimError::io(&path))
    }
}

/// Execute a dim update (arch D4): pull member facet vertices from LIST_ALL,
/// translate by `(new_value - current) * dir`, PUT_SOME a replace payload
/// (original objects deleted and recreated server-side).
///
/// Order of checks: unsaved doc -> dim exists -> set exists -> members exist
/// (missing report) -> topology kinds (hard reject) -> translate & write.
/// Degraded path: PUT_SOME unavailable -> `ok=false` report with a
/// read-only hint (no partial write).
pub async fn dim_update(
    client: &pla_bridge::BridgeClient,
    selections: &pla_selection::SelectionStore,
    dims: &DimStore,
    doc_key: &str,
    name: &str,
    new_value: f64,
) -> Result<DimReport, DimError> {
    if doc_key.is_empty() {
        return Err(DimError::UnsavedDoc);
    }
    let mut def = dims.load(doc_key, name)?;
    let set = selections.load(doc_key, &def.set_name)?;

    // One LIST_ALL drives both the existence check and the vertex pull.
    // TODO-PROBE: real entity/facet payload shape.
    let payload = client
        .list_all(doc_key)
        .await
        .map_err(pla_selection::SelectionError::from)?;
    let tree = pla_selection::entity_tree_from_payload(doc_key, &payload);
    let missing = tree.missing_members(&set.members);
    if !missing.is_empty() {
        return Ok(DimReport {
            ok: false,
            moved: 0,
            missing,
            message: String::new(),
        });
    }

    // Topology-typed members are hard-rejected (structured error, explicit
    // copy for the UI).
    for member in &set.members {
        if let Some(node) = tree.find(member) {
            if TOPOLOGY_KINDS.contains(&node.kind.to_lowercase().as_str()) {
                return Err(DimError::TopologyUnsupported {
                    member: member.clone(),
                    kind: node.kind.clone(),
                });
            }
        }
    }

    // Build the replace payload: every member re-emitted with translated
    // facet vertices (originals are deleted and recreated server-side).
    let entities_raw = flatten_entities(&payload);
    let delta = new_value - def.value;
    let mut replaced = Vec::new();
    let mut moved = 0usize;
    for member in &set.members {
        let Some(entity) = entities_raw.iter().find(|e| {
            e.get("id").and_then(|i| i.as_str()) == Some(member.as_str())
        }) else {
            continue;
        };
        let vertices = facet_vertices_from_entity(entity);
        if vertices.is_empty() {
            continue;
        }
        let translated = translate_vertices(&vertices, delta, &def.dir);
        let mut obj = entity.clone();
        obj["facets"] = serde_json::json!([{ "vertices": translated }]);
        replaced.push(obj);
        moved += 1;
    }

    // TODO-PROBE: PUT_SOME replace semantics (delete + recreate by id).
    let data = serde_json::json!({ "op": "REPLACE_ENTITIES", "entities": replaced });
    match client.put_some(doc_key, data).await {
        Ok(_) => {
            def.value = new_value;
            dims.save(&def)?;
            Ok(DimReport {
                ok: true,
                moved,
                missing: Vec::new(),
                message: String::new(),
            })
        }
        // Degraded path: dim driver stays read-only with an explicit hint.
        Err(e) => Ok(DimReport {
            ok: false,
            moved: 0,
            missing: Vec::new(),
            message: format!("read-only: PUT_SOME unavailable ({e}); document unchanged"),
        }),
    }
}

/// Keep `[A-Za-z0-9-_]`; map everything else to `-` (same policy as
/// pla-snapshot / pla-selection). `None` when empty.
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
    fn dir_normalized_handles_zero_and_non_finite() {
        assert_eq!(DimDir::new(0.0, 0.0, 0.0).normalized(), DimDir::unit_x());
        assert_eq!(
            DimDir::new(f64::NAN, 1.0, 0.0).normalized(),
            DimDir::unit_x()
        );
        assert_eq!(
            DimDir::new(3.0, 4.0, 0.0).normalized(),
            DimDir::new(0.6, 0.8, 0.0)
        );
    }

    #[test]
    fn translate_vertex_applies_delta_along_unit_dir() {
        let v = [1.0, 2.0, 3.0];
        // +X, delta 5 -> only x moves.
        assert_eq!(translate_vertex(v, 5.0, &DimDir::unit_x()), [6.0, 2.0, 3.0]);
        // Non-unit dir is normalized first: (0,3,4) -> (0,0.6,0.8), delta 10.
        assert_eq!(
            translate_vertex(v, 10.0, &DimDir::new(0.0, 3.0, 4.0)),
            [1.0, 8.0, 11.0]
        );
        // Negative delta pulls back: dir (0,0,2) normalizes to (0,0,1),
        // so z moves by exactly -1.
        assert_eq!(
            translate_vertex(v, -1.0, &DimDir::new(0.0, 0.0, 2.0)),
            [1.0, 2.0, 2.0]
        );
    }

    #[test]
    fn translate_vertices_maps_batch() {
        let out = translate_vertices(&[[0.0; 3], [1.0, 0.0, 0.0]], 2.0, &DimDir::unit_x());
        assert_eq!(out, vec![[2.0, 0.0, 0.0], [3.0, 0.0, 0.0]]);
    }

    #[test]
    fn facet_vertices_parses_assumed_shape_leniently() {
        let e = serde_json::json!({
            "id": "e1",
            "facets": [
                {"vertices": [[0,0,0], [1,0,0]]},
                {"vertices": [[0,1,0]]},
            ]
        });
        assert_eq!(
            facet_vertices_from_entity(&e),
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        );
        // odd shapes tolerated
        assert!(facet_vertices_from_entity(&serde_json::json!({"id": "e"})).is_empty());
        assert!(facet_vertices_from_entity(&serde_json::json!({"facets": [{"vertices": [[1,2]]}]})).is_empty());
    }

    #[test]
    fn flatten_walks_children() {
        let payload = serde_json::json!({"entities": [
            {"id": "a", "children": [{"id": "a1"}]},
            {"id": "b"},
        ]});
        let ids: Vec<String> = flatten_entities(&payload)
            .iter()
            .filter_map(|e| e.get("id").and_then(|i| i.as_str()).map(String::from))
            .collect();
        assert_eq!(ids, vec!["a", "a1", "b"]);
    }

    fn temp_base(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "pla-dim-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn dim_store_crud_and_normalization() {
        let base = temp_base("crud");
        let store = DimStore::new(&base);
        let def = store
            .define(
                "doc.plasticity",
                "arm length",
                "arms",
                DimDir::new(0.0, 3.0, 4.0),
                42.0,
            )
            .unwrap();
        assert_eq!(def.name, "arm-length");
        assert_eq!(def.dir, DimDir::new(0.0, 0.6, 0.8));
        assert_eq!(store.list("doc.plasticity").len(), 1);
        let loaded = store.load("doc.plasticity", "arm-length").unwrap();
        assert_eq!(loaded, def);
        // doc isolation (D5)
        assert!(store.list("other.plasticity").is_empty());
        assert!(matches!(
            store.load("other.plasticity", "arm-length").unwrap_err(),
            DimError::NotFound(_)
        ));
        // save updated value
        let mut updated = loaded;
        updated.value = 50.0;
        store.save(&updated).unwrap();
        assert_eq!(store.load("doc.plasticity", "arm-length").unwrap().value, 50.0);
        // delete + guards
        store.delete("doc.plasticity", "arm-length").unwrap();
        assert!(store.list("doc.plasticity").is_empty());
        assert!(matches!(
            store.define("", "x", "s", DimDir::unit_x(), 1.0).unwrap_err(),
            DimError::UnsavedDoc
        ));
        assert!(matches!(
            store.define("doc.plasticity", "///", "s", DimDir::unit_x(), 1.0).unwrap_err(),
            DimError::InvalidName(_)
        ));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn topology_error_message_is_explicit() {
        let err = DimError::TopologyUnsupported {
            member: "e9".into(),
            kind: "fillet".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("e9"));
        assert!(msg.contains("fillet"));
        assert!(msg.contains("not supported"));
    }
}
