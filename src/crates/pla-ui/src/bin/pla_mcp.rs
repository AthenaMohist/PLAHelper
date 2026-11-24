//! pla-mcp: PLAhelper 的 MCP 服务器(stdio, JSON-RPC 2.0, 一行一条)。给 Claude Code 等外部 agent 用:
//! 看零件清单 / 某张图的设置 + 报告 + 预览图, 改出图设置(可一次改多张), 出图, 记长期规则, 在 pla-ui 里打开某个零件。
//!
//! 接入 Claude Code:  claude mcp add plahelper -- "<repo>\src\target\release\pla-mcp.exe"
//! 读直接读文件; 改 / 出图交给正在跑的 pla-ui(见 pla_ui::mcp_bridge)。

// 工具清单是一个很大的 json! 字面量(宏展开深度超过默认 128)
#![recursion_limit = "512"]

use std::io::{BufRead, Write};
use std::time::Duration;

use pla_ui::draw_agent;
use pla_ui::drawing;
use pla_ui::mcp_bridge::{self as bridge, PartInfo};
use serde_json::{json, Value};

const SUPPORTED: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

fn patch_schema() -> Value {
    let mut s = draw_agent::tools()[0]["input_schema"].clone();
    s["properties"]["parts"] = json!({"type": "array", "items": {"type": "string"}, "description": "Parts to change: key, part number or name (one sentence can change many sheets)"});
    s["properties"]["redraw"] = json!({"type": "boolean", "description": "Recompile the changed sheets afterwards (default false)"});
    s["required"] = json!(["parts", "summary"]);
    s
}

fn tools() -> Value {
    let part = json!({"type": "object", "properties": {"part": {"type": "string", "description": "Part key, part number or name"}}, "required": ["part"]});
    json!([
        {"name": "status", "description": "PLAhelper status: is pla-ui running, current Plasticity document, selected part, drawing job.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "list_parts", "description": "Parts in the current BOM scope with part number, name, quantity, material, supplier and drawing status (drawn / uploaded+locked / model changed / not drawn).", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "get_drawing", "description": "One part's drawing: current settings, the last compile report (dimension names and values, hole groups, warnings) and the preview image of the sheet. Use the dimension names from here in apply_drawing_changes.", "inputSchema": part},
        {"name": "apply_drawing_changes", "description": "Apply a settings patch to one or more part drawings (orientation, section, hole positions, threads, density, hide/show dims, key dims, dim text, tolerances, technical requirements, title block, datums / geometric tolerances / surface roughness - saved in the sheet settings, so they survive redrawing; never edit <base>.spec.json by hand, PLAhelper rewrites it on every draw). Applies immediately - confirm with the user first when the request was ambiguous. Locked parts (uploaded drawing) are skipped.", "inputSchema": patch_schema()},
        {"name": "draw", "description": "Compile drawings in PLAhelper (re-exports STEP from Plasticity when needed; ~20-90 s per sheet). parts = list, or [\"undrawn\"] for every part not drawn yet or changed since. Optionally waits.", "inputSchema": {"type": "object", "properties": {
            "parts": {"type": "array", "items": {"type": "string"}},
            "export": {"type": "boolean", "description": "Force STEP re-export (after the model changed)"},
            "wait_seconds": {"type": "integer", "description": "Wait up to this long for the sheets to finish (0 = return at once, max 600)"}
        }, "required": ["parts"]}},
        {"name": "list_rules", "description": "The user's standing drawing rules (applied by the in-app assistant to every drawing).", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "remember_rule", "description": "Save a standing drawing rule for all future drawings (only when the user states a general preference).", "inputSchema": {"type": "object", "properties": {"rule": {"type": "string"}}, "required": ["rule"]}},
        {"name": "show_part", "description": "Open the part in PLAhelper's drawing tab so the user sees its sheet.", "inputSchema": part}
    ])
    .as_array()
    .cloned()
    .unwrap_or_default()
    .into_iter()
    .chain(model_tools().as_array().cloned().unwrap_or_default())
    .collect::<Vec<_>>()
    .into()
}

// ---------------------------------------------------------------- A6 建模工具(直接经 CDP 连 Plasticity, 不经 pla-ui)

/// 前台 Plasticity(用户在用的)/ 后台实例(tools/pla-cdp/pla-bg.js 起的独立隐藏实例)的 CDP 代理端口。
const FG_PORT: u16 = 9333;
const BG_PORT: u16 = 9343;
/// 本次工具调用连哪个(按参数 instance; 每次调用开头设)。
static PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(FG_PORT);
fn port() -> u16 {
    PORT.load(std::sync::atomic::Ordering::Relaxed)
}

fn model_tools() -> Value {
    let refs = "Topology ids look like '34f9439' (face) / '34e912' (edge) / '34v12' (vertex) - '34' is the solid's version id, which changes after every edit of that solid, so re-query after changes. Solids can also be given as {\"name\": \"<outliner name>\"}.";
    let ro = json!({"readOnlyHint": true});
    let mut tools = json!([
        {"name": "model_state", "annotations": ro, "description": format!("Read the open Plasticity model: undo depth, current selection, and geometry. detail: solids (default, overview) / faces / edges / all (+ vertices). Filter with of (solid ids or {{name}}), surface (Plane/Cylinder/Cone/Torus/…), curve (Line/Circle/…), normal [x,y,z] with angleTol (deg), near [x,y,z] with tol. Lengths in document units. Start with the overview, then query faces/edges of one solid. {refs}"),
         "inputSchema": {"type": "object", "properties": {
            "detail": {"type": "string", "enum": ["solids", "faces", "edges", "all"]},
            "of": {"type": "array", "items": {}}, "surface": {"type": "string"}, "curve": {"type": "string"},
            "normal": {"type": "array", "items": {"type": "number"}}, "angleTol": {"type": "number"},
            "near": {"type": "array", "items": {"type": "number"}}, "tol": {"type": "number"},
            "units": {"type": "string"}, "limit": {"type": "integer"}}}},
        {"name": "model_snaps", "annotations": ro, "description": "Snap points Plasticity itself uses (also internal ones): face centers + normals, edge midpoints + tangents, circle centers + axes, points on arcs, vertices, bbox corners. kinds filters (faceCenter|edgeMid|circleCenter|circleQuarter|vertex|bboxCorner); near/tol/of/limit as in model_state.",
         "inputSchema": {"type": "object", "properties": {"of": {"type": "array", "items": {}}, "kinds": {"type": "array", "items": {"type": "string"}}, "near": {"type": "array", "items": {"type": "number"}}, "tol": {"type": "number"}, "units": {"type": "string"}, "limit": {"type": "integer"}}}},
        {"name": "model_outline", "annotations": ro, "description": "Read Plasticity's outliner tree: folders (id, name, hidden, children) and items (kind solid/sheet/curve/instance/measurement/…, name, hidden, locked; solids carry their version id like model_state, other kinds a node key). kinds filters items (e.g. [\"solid\"]); folders are always kept. Change it with model_exec ops folder / moveTo / hide / show / ungroup / rename.",
         "inputSchema": {"type": "object", "properties": {"kinds": {"type": "array", "items": {"type": "string"}}}}},
        {"name": "model_measure", "annotations": ro, "description": format!("Measure. op=distance: a, b = [x,y,z] | face id | edge id | vertex id → distance, delta, and normalDistance (wall thickness) when both are parallel planes. op=angle: a, b = faces (normals), line edges (directions) or vectors → degrees. op=mass: of (solids) and/or faces → volume, area, centroid (display mesh, curved surfaces ~0.3% low). {refs}"),
         "inputSchema": {"type": "object", "properties": {"op": {"type": "string", "enum": ["distance", "angle", "mass"]}, "a": {}, "b": {}, "of": {"type": "array", "items": {}}, "faces": {"type": "array", "items": {"type": "string"}}, "units": {"type": "string"}}, "required": ["op"]}},
        {"name": "model_view", "annotations": ro, "description": "Screenshot the model from a standard view (front/back/left/right/top/bottom/iso), framed on 'of' solids (default: everything). The user's camera is restored afterwards. Use it to check results visually.",
         "inputSchema": {"type": "object", "properties": {"view": {"type": "string", "enum": ["front", "back", "left", "right", "top", "bottom", "iso"]}, "of": {"type": "array", "items": {}}, "width": {"type": "integer", "description": "max width px (default 960)"}}}},
        {"name": "model_exec", "annotations": {"destructiveHint": true},
         "description": format!("CHANGES THE USER'S MODEL - only when the user asked for this modeling work. Runs geometry steps through Plasticity's own factories (each step = one undo step, same as manual work). atomic (default true): if a step fails, every step of this call is undone. Cancels any command the user has in progress; the Plasticity window must be visible.\n\
ops (lengths in document units unless units is given; points [x,y,z]):\n\
- box {{p1, p2 (opposite corners of the base), height}} (+ bool, targets)\n\
- cylinder {{center (base center), axis [x,y,z] (grow direction), radius, height}} (+ bool, targets) - a hole = cylinder with bool 'subtract' and targets [the solid]\n\
- fillet {{edges, distance}} / chamfer {{edges, distance}} (45°)\n\
- offsetFace {{faces, distance}} (+ = grow outward along the face normal; on a hole wall + makes the hole smaller)\n\
- refillet {{faces (fillet faces), distance (= radius change)}}\n\
- moveFaces {{faces, move [dx,dy,dz]}} (move a hole / boss / pocket)\n\
- revolve {{origin, axis, profile [[r, z], ...], degrees?=360}} (+ bool, targets) - solid of revolution: r = distance from the axis (>= 0, never crossing it), z = position along axis from origin; the profile closes itself; points with r = 0 lie on the axis. Screws, shafts, steps, cones, grooves (subtract).\n\
- prism {{center, normal?=[0,0,1] (extrude direction), sides, radius (circumscribed) | flats (across flats), height (negative = opposite), rotate? (deg about normal)}} (+ bool, targets) - regular polygon prism: hex socket (subtract, normal pointing into the head), hex head, nut.\n\
Whole solids (solids = [solid refs]: ids, \"$step\", {{\"name\": ...}}; the result items can be referenced like any step):\n\
- duplicate {{solids}} - copy in place (then move / rotate it: copy a whole part with all its holes, chamfers and screws in one go)\n\
- move {{solids, move [dx,dy,dz]}} / rotate {{solids, pivot, axis, degrees}} (right-hand rule about the axis through pivot)\n\
- mirror {{solids, origin, normal}} - adds a mirrored copy about the plane through origin with that normal (originals stay; their ids change)\n\
- pattern {{solids, dir1, count1, spacing1, dir2?, count2?, spacing2?}} - linear / grid array; counts include the original; result = the new copies\n\
- radialPattern {{solids, center, axis, count, degrees?=360}} - circular array about the axis through center; 360 = evenly around, less = both ends included (3 over 180° = 0/90/180)\n\
- boolean {{targets, tools, operation union|subtract|intersect, keepTools?}} - between existing solids\n\
- delete {{solids}} / rename {{solids? | folders?, name}}\n\
Outliner (folders = Plasticity groups; read the tree with model_outline; folder refs = folder id, name (error if several share it), \"$step\" or \"root\" = top level):\n\
- folder {{name, parent?, solids?, folders?}} - new folder (under parent, default top level) and move those into it; result item = {{kind: folder, id, name}}\n\
- moveTo {{solids? | folders?, to}} - move into a folder (\"root\" = top level)\n\
- hide {{solids? | folders?}} / show {{…}} - same as H / unhide (a folder hides everything in it)\n\
- ungroup {{folders}} - dissolve; members go up one level\n\
bool: union | subtract | intersect with targets [solid refs]. Give a step id: \"base\" and later steps can use \"$base\" (its result). Edges / faces: topology ids, or {{\"near\": [x,y,z], \"of\": solid ref, \"tol\"?}} = the item whose bbox center is nearest (errors instead of guessing when ambiguous). {refs} After running, verify with model_state / model_measure / model_view."),
         "inputSchema": {"type": "object", "properties": {
            "ops": {"type": "array", "items": {"type": "object"}},
            "atomic": {"type": "boolean"}, "units": {"type": "string"}}, "required": ["ops"]}},
        {"name": "model_hole", "annotations": {"destructiveHint": true},
         "description": "CHANGES THE USER'S MODEL. Hole wizard in one atomic step: face (plane face id where the hole starts), type simple|counterbore|countersink, d (diameter) OR size (\"M4\": d / cbD / cbDepth / cskD / tapD come from the active hole table, see hole_table; usage clearance (default) | tap; table = another hole table; explicit numbers still win), depth (omit = through), cbD/cbDepth (counterbore), cskD (countersink, 90°), position: at [x,y,z] or offset [u,v] from the face center (u/v axes are returned), or points (sketch points; a selected straight line = a slot along it). slots [{from:[x,y,z], to:[x,y,z]}] = slot holes (width d, round ends; counterbore = slot-shaped recess; countersink not supported on slots). stack = further solid ids for a hole through several parts (tapD = diameter in the last one; for a slot the last part gets ONE tap hole at tapAt: mid (default) | from | to | [x,y,z], or at a point given in points that lies on the slot). Returns the created faces per hole (slots: wallFrom / wallTo instead of wall). All holes on one part are cut in one boolean, so many holes at once are fast.",
         "inputSchema": {"type": "object", "properties": {
            "face": {"type": "string"}, "type": {"type": "string", "enum": ["simple", "counterbore", "countersink"]}, "d": {"type": "number"},
            "depth": {"type": "number"}, "cbD": {"type": "number"}, "cbDepth": {"type": "number"}, "cskD": {"type": "number"}, "tapD": {"type": "number"},
            "at": {"type": "array", "items": {"type": "number"}}, "offset": {"type": "array", "items": {"type": "number"}},
            "points": {"type": "array", "items": {"type": "object"}}, "stack": {"type": "array", "items": {"type": "string"}}, "units": {"type": "string"},
            "slots": {"type": "array", "items": {"type": "object", "properties": {"from": {"type": "array", "items": {"type": "number"}}, "to": {"type": "array", "items": {"type": "number"}}}, "required": ["from", "to"]}},
            "tapAt": {"description": "slot + stack: where the single tap hole goes: mid | from | to | [x,y,z]"},
            "size": {"type": "string"}, "usage": {"type": "string", "enum": ["clearance", "tap"]}, "table": {"type": "string"}},
          "required": ["face"]}},
        {"name": "model_assets", "annotations": ro, "description": "List the asset packs registered in Plasticity (Assets panel) and their assets: name, folder, size (document units). Prefer the user's own assets (e.g. their screw pack) over building standard parts with model_part; hole sizes come from the GB tables (model_hole), never from assets.",
         "inputSchema": {"type": "object", "properties": {"pack": {"type": "string", "description": "path or part of it (default: all)"}}}},
        {"name": "model_asset_place", "annotations": {"destructiveHint": true},
         "description": "CHANGES THE USER'S MODEL. Place an asset from a registered pack exactly like dragging it from the Assets panel (keeps the asset reference), at a point with its +Z along `normal`. Drill the hole first (model_hole). Needs the Plasticity window visible. One undo step; a failure undoes itself.",
         "inputSchema": {"type": "object", "properties": {
            "asset": {"type": "string", "description": "asset name (e.g. M4) or id"}, "pack": {"type": "string"},
            "at": {"type": "array", "items": {"type": "number"}, "description": "where the asset's pivot goes (document units)"},
            "normal": {"type": "array", "items": {"type": "number"}, "description": "direction of the asset's +Z (for a screw: the face's outward normal, head outward); default [0,0,1]"},
            "rotate": {"type": "number", "description": "degrees about normal"}},
          "required": ["asset", "at"]}},
        {"name": "model_part_stretch", "annotations": {"destructiveHint": true},
         "description": "CHANGES THE USER'S MODEL. Change a screw's length by moving only its tip faces (end face + chamfer) along xis - never scale. Use after placing a pack screw (packs keep screws at 10 mm). One undo step.",
         "inputSchema": {"type": "object", "properties": {
            "solid": {"description": "solid id"}, "axis": {"type": "array", "items": {"type": "number"}, "description": "direction toward the tip (into the hole)"},
            "delta": {"type": "number", "description": "length change in document units (+ = longer)"}, "zone": {"type": "number", "description": "tip region length (default min(2.5, 0.3 x part length))"}},
          "required": ["solid", "axis", "delta"]}},
        {"name": "model_asset_library", "annotations": {"destructiveHint": true},
         "description": "PLAhelper's GB fastener asset library (one Plasticity asset pack per standard, shipped with PLAhelper). action=series: list buildable series and which packs exist. action=build (series, sizes?): build every size (or the given ones) per the GB tables - screws at 10 mm, pivot on the bearing face, +Z tip->head - mark them as assets, export the pack into the library folder, then roll the document back (the document must hold no other assets; needs the Plasticity window visible) and register the pack. action=install: copy the bundled packs to the user's data folder and register them in Plasticity's Assets panel.",
         "inputSchema": {"type": "object", "properties": {
            "action": {"type": "string", "enum": ["series", "build", "install"]}, "series": {"type": "string", "description": "e.g. gb70.1, gb818, gb97.1, gb6170"},
            "sizes": {"type": "array", "description": "e.g. [\"M3\", \"M4\"] (default: all)"}},
          "required": ["action"]}},
        {"name": "model_part", "annotations": {"destructiveHint": true},
         "description": format!("CHANGES THE USER'S MODEL. Build a GB standard part in one atomic call (A10 macros; threads not modelled). kind: screw (any catalog standard via std, e.g. gb818 pan cross / gb70.2 button; recesses for cross / slot not modelled) / screw-cap (GB/T 70.1 socket head cap) / screw-csk (GB/T 70.3 countersunk socket, length includes head) / screw-hex (GB/T 5783 hex bolt) / screw-set (GB/T 77 socket set screw, flat point) / nut (std gb6170 default / gb6172.1 / gb889.1) / washer (std gb97.1 default / gb848 / gb96.1 / gb93 spring) / pin (GB/T 119.2 dowel, size = diameter) / shaft (steps [[diameter, length], …], chamfer). Placement: at + axis. Screws: at = centre of the head's bearing face (countersunk: of the head top), axis = from head toward the tip (into the hole). Set screw: at = socket end. Nut / washer / pin / shaft: at = one end face centre, axis = thickness / length direction. Returns the steps' results and the spec key PLAhelper's BOM recognizer will report. Sizes: {}", pla_ui::fasteners::sizes()),
         "inputSchema": {"type": "object", "properties": {
            "kind": {"type": "string", "enum": ["screw", "screw-cap", "screw-csk", "screw-hex", "screw-set", "nut", "washer", "pin", "shaft"]}, "std": {"type": "string", "description": "catalog id (screw: required; nut / washer: optional)"},
            "size": {"description": "\"M4\" (or 4); pin = diameter"}, "length": {"type": "number"},
            "steps": {"type": "array", "items": {"type": "array", "items": {"type": "number"}}}, "chamfer": {"type": "number"},
            "at": {"type": "array", "items": {"type": "number"}}, "axis": {"type": "array", "items": {"type": "number"}}},
          "required": ["kind"]}},
        {"name": "model_build", "annotations": {"destructiveHint": true},
         "description": "CHANGES THE USER'S MODEL. Build a part from a design spec (A7/A8): params (numbers or expressions like \"w - 2*t\"; standard refs M3.clearance / M4.tap / M4.d) + features in order + checks. Everything runs in one transaction: any failing feature or check rolls the whole build back and reports which one.\n\
features (every one needs a unique id; numbers may be expressions):\n\
- box {size [x,y,z], at [x,y,z] (min corner)}\n\
- cylinder {d | r, h, at (base centre), axis?=[0,0,1]}\n\
- revolve / prism - same fields as model_exec\n\
- part {kind, size, length, at, axis} - a GB standard part (see model_part), separate solid\n\
- hole {on: \"face(<feature>.<side>)\", d, depth? (omit = through), kind? simple|counterbore|countersink, cbD, cbDepth, cskD, and at [x,y,z] | positions [[x,y,z],…] | pattern {start, grid [nu,nv], pitch [pu,pv]} (u/v = the hole tool's in-plane axes: for a +Z face u = +X, v = +Y)}\n\
- fillet / chamfer {edges: [\"edge(face(a.top), face(b.right))\", …], r}\n\
box / cylinder / revolve / prism take bool union|subtract|intersect with into: <earlier feature id> (they join that feature's body).\n\
Semantic refs: face(<feature>.<side>), side = top/bottom/left/right/front/back (±Z/±X/±Y), resolved after each step against the live geometry within that feature's own extent - so face(base.top) stays the base's top even after an upright is merged on.\n\
checks: {bbox [[min],[max]], of?, tol?=0.05} · {count {\"Cylinder\": 3}, of?} · {volume {min?, max?}, of?}.\n\
The spec, a map (bodies, solid ids, undo range) and a log are saved under the document's .pla/models/<doc>/specs/. replace: true first undoes this spec's previous build (only if nothing else was done since).",
         "inputSchema": {"type": "object", "properties": {"spec": {"type": "object"}, "replace": {"type": "boolean"}}, "required": ["spec"]}},
        {"name": "model_transaction", "annotations": {"destructiveHint": true},
         "description": "Group several model_exec / model_hole calls so they can be undone together. action=begin (remember the undo depth) → work → commit (keep) or rollback (undo everything since begin, including anything the user did in between; refused if the undo stack below begin was changed). action=undo with steps = undo the last N steps (like Ctrl+Z). action=state = undo / redo depth. The transaction survives shim updates (kept on the page).",
         "inputSchema": {"type": "object", "properties": {"action": {"type": "string", "enum": ["begin", "commit", "rollback", "undo", "state"]}, "steps": {"type": "integer", "description": "for undo: how many steps (default 1)"}}, "required": ["action"]}},
        {"name": "background", "annotations": {"destructiveHint": true},
         "description": "A separate, hidden Plasticity instance for independent jobs, so the user can keep working (browser, or their own Plasticity on another project) undisturbed: it never shows a window, never appears in the taskbar, never takes focus, and never moves the real mouse. action=start (doc?: a .plasticity file to open; uses a copy of the user's login/settings in its own data folder; ~5 s) / open (doc, discard?: drop unsaved changes) / save_as (doc) / status / stop. A document that is open in the user's foreground Plasticity is REFUSED (open / save_as / jobs) - tell the user to close it there first. Jobs on the open document: background_bom / background_draw / background_release. Then pass instance: \"background\" to any model_* tool to work in it. Native dialogs there never show: save dialogs are answered by PLAhelper (pack / STEP export), other message boxes are auto-cancelled and logged (see status).",
         "inputSchema": {"type": "object", "properties": {"action": {"type": "string", "enum": ["start", "status", "stop", "open", "save_as"]}, "doc": {"type": "string"}, "discard": {"type": "boolean"}}, "required": ["action"]}},
        {"name": "bom_table", "annotations": ro,
         "description": "The BOM of a document as PLAhelper computes it (template columns with formulas, user entries, part library, part identity tracking): columns (id, header, editable, formula, default), rows (row key, cells by column id, which cells were overridden, supplier for release, drawing status), totals, excluded rows, document defaults, available templates and suppliers. doc = foreground (default, the document open in the user's Plasticity) / background. scope = a configuration name (default: currently visible). Use it before bom_edit to see row keys and column ids.",
         "inputSchema": {"type": "object", "properties": {"doc": {"type": "string", "enum": ["foreground", "background"]}, "scope": {"type": "string"}}}},
        {"name": "bom_edit", "annotations": {"destructiveHint": true},
         "description": "Change BOM entries the way the user would in PLAhelper's BOM tab (saved per document; PLAhelper picks the change up at once). set: [{part, column, value}] (part = row key / part number / name; column = id or header, also a supplier's own columns like finish / color; value null = back to the computed / default value). assign: [{part, supplier}] = which supplier makes it in a release (supplier id or name, \"buy\" = bought, \"\" = unassigned). exclude / include: [part] (leave out of / back into the BOM). add_rows: [{column: value}] (manual rows for parts not in the model, e.g. purchased items). remove_rows: [\"manual:<id>\"]. defaults: {column: value} (document-wide defaults, e.g. sets = 5; null clears). template: BOM template name (\"\" = default; edit templates with bom_template). project: project number prefix for part numbers. Returns the updated table and what changed.",
         "inputSchema": {"type": "object", "properties": {
            "doc": {"type": "string", "enum": ["foreground", "background"]}, "scope": {"type": "string"},
            "set": {"type": "array", "items": {"type": "object", "properties": {"part": {"type": "string"}, "column": {"type": "string"}, "value": {}}, "required": ["part", "column"]}},
            "assign": {"type": "array", "items": {"type": "object", "properties": {"part": {"type": "string"}, "supplier": {"type": "string"}}, "required": ["part", "supplier"]}},
            "exclude": {"type": "array", "items": {"type": "string"}}, "include": {"type": "array", "items": {"type": "string"}},
            "add_rows": {"type": "array", "items": {"type": "object"}}, "remove_rows": {"type": "array", "items": {"type": "string"}},
            "defaults": {"type": "object"}, "template": {"type": "string"}, "project": {"type": "string"}}}},
        {"name": "bom_export", "annotations": {"destructiveHint": true},
         "description": "Export a document's BOM as CSV (next to the document, under .pla/bom). doc = foreground / background; scope = configuration name.",
         "inputSchema": {"type": "object", "properties": {"doc": {"type": "string", "enum": ["foreground", "background"]}, "scope": {"type": "string"}}}},
        {"name": "draw_parts", "annotations": {"destructiveHint": true},
         "description": "Make 2D part drawings directly (STEP export from Plasticity + PLAhelper's compiler, same settings / preferences / revision table as the drawing tab; ~20-90 s per sheet; two or more sheets are also combined into one PDF). parts = [\"undrawn\"] / [\"all\"] / keys, part numbers or names. export = force STEP re-export. doc = foreground (exports from the user's Plasticity: it briefly changes and then restores the selection) / background (the hidden instance). For foreground drawings while the user works, prefer doc=background or the queued draw tool.",
         "inputSchema": {"type": "object", "properties": {"doc": {"type": "string", "enum": ["foreground", "background"]}, "parts": {"type": "array", "items": {"type": "string"}}, "export": {"type": "boolean"}}, "required": ["parts"]}},
        {"name": "release", "annotations": {"destructiveHint": true},
         "description": "Write a release (external) package: records the version (a new release freezes an ECN of everything changed since the last one; amend = true re-sends the same batch with the current state), redraws the drawn parts the ECN touches so their revision table shows it, then writes the master BOM plus one folder (and zip) per supplier with that supplier's BOM columns and files. Reports parts not assigned to any supplier (assign them with bom_edit) and missing files. doc = foreground / background.",
         "inputSchema": {"type": "object", "properties": {"doc": {"type": "string", "enum": ["foreground", "background"]}, "amend": {"type": "boolean"}}}},
        {"name": "release_history", "annotations": ro,
         "description": "A document's release history: versions, ECNs (what changed per part), who opened them. doc = foreground / background.",
         "inputSchema": {"type": "object", "properties": {"doc": {"type": "string", "enum": ["foreground", "background"]}}}},
        {"name": "suppliers", "annotations": {"destructiveHint": true},
         "description": "Release suppliers (global, shared by all documents; default: CNC / 3D printing / silicone molding / plastic molding). action=list / add (name, id?, files?: pdf|dxf|step, columns?: master BOM column ids, extra?: [{header, id?, default?}] = the supplier's own BOM columns) / update (id or name, new_name?, files?, columns?, extra?) / remove (id or name).",
         "inputSchema": {"type": "object", "properties": {"action": {"type": "string", "enum": ["list", "add", "update", "remove"]}, "id": {"type": "string"}, "name": {"type": "string"}, "new_name": {"type": "string"},
            "files": {"type": "array", "items": {"type": "string"}}, "columns": {"type": "array", "items": {"type": "string"}}, "extra": {"type": "array", "items": {"type": "object"}}}, "required": ["action"]}},
        {"name": "configs", "annotations": {"destructiveHint": true},
         "description": "Configurations of a document (saved visibility + material assignments; also usable as BOM scope). action=list / apply (name: switch the model to it) / save (name?: capture the current visibility + materials, overwriting a configuration of that name). doc = foreground / background.",
         "inputSchema": {"type": "object", "properties": {"doc": {"type": "string", "enum": ["foreground", "background"]}, "action": {"type": "string", "enum": ["list", "apply", "save"]}, "name": {"type": "string"}}, "required": ["action"]}},
        {"name": "dims", "annotations": {"destructiveHint": true},
         "description": "The document's named dimensions and parameters (PLAhelper's records: D001 / H001 hole features ..., parameters with expressions that drive dimensions). action=list / set (changes: [{name, value}] - changes the model so each named dimension gets that value, in one transaction; a failed step rolls everything back) / set_param (name, expr - change a parameter; every dimension bound to it follows). set / set_param work on the foreground document through PLAhelper (pla-ui must be running). Values in document units.",
         "inputSchema": {"type": "object", "properties": {"doc": {"type": "string", "enum": ["foreground", "background"]}, "action": {"type": "string", "enum": ["list", "set", "set_param"]},
            "changes": {"type": "array", "items": {"type": "object", "properties": {"name": {"type": "string"}, "value": {"type": "number"}}, "required": ["name", "value"]}}, "name": {"type": "string"}, "expr": {}}, "required": ["action"]}},
        {"name": "parts_library", "annotations": {"destructiveHint": true},
         "description": "PLAhelper's parts library (global, all documents): purchase name, part number (rule: project prefix + category code + serial), BOM fields (model / origin / unit / pcs_unit / ... by BOM column id), suppliers with price / currency / MOQ / lead time / price date, preferred supplier, aliases, standard-part spec. A BOM row linked to an entry takes its name, fields and preferred price (document overrides still win). action=search (query?, category?, limit?) / get (pn; former numbers work too) / add (name, category? (guessed when omitted), pn? (auto by category + project), project?, fields?, suppliers?: [{name, price, currency, moq, lead, url}], aliases?, spec?, notes?) / update (pn, name?, fields? (null value removes), supplier?: {name, price, ...} (updates that supplier or adds it; a new price stamps today's date), suppliers? (replace all), preferred? (supplier name or index), category? (renumbers, old number kept as former), aliases?, spec?, notes?) / remove (pn) / link (doc?, part, pn; empty pn unlinks) / from_bom (doc?, part: save that BOM row as a new entry and link it) / suggest (doc?: library suggestions per BOM row).",
         "inputSchema": {"type": "object", "properties": {
            "action": {"type": "string", "enum": ["search", "get", "add", "update", "remove", "link", "from_bom", "suggest"]},
            "query": {"type": "string"}, "category": {"type": "string"}, "limit": {"type": "integer"}, "pn": {"type": "string"}, "name": {"type": "string"}, "project": {"type": "string"},
            "fields": {"type": "object"}, "supplier": {"type": "object"}, "suppliers": {"type": "array", "items": {"type": "object"}}, "preferred": {}, "aliases": {"type": "array", "items": {"type": "string"}},
            "spec": {"type": "string"}, "notes": {"type": "string"}, "doc": {"type": "string", "enum": ["foreground", "background"]}, "part": {"type": "string"}, "scope": {"type": "string"}}, "required": ["action"]}},
        {"name": "bom_template", "annotations": {"destructiveHint": true},
         "description": "BOM templates (global; each is a list of columns that every document's BOM uses; a document picks one with bom_edit template). Column: id, header, source (model data: name / qty / material / volume / group / remark / spec; \"field\" = editable, with default) or formula (e.g. \"ROUNDUP({total_qty}*{redundancy}/{pcs_unit},0)\", {id} = another column; ROUNDUP / CEILING / IF ...), sum (total row). action=list / get (name) / create (name, from?: template name or \"builtin\") / rename (name, new_name) / remove (name; not the last) / edit (name; add_columns: [{id, header, source|formula, default?, sum?, after?: column id}], update_columns: [{id, header?, source?, formula?, default?, sum?}] (null clears an item), remove_columns: [id], order: [ids first], columns: [...] replaces all). Checked before saving: unique ids, known sources, formulas only refer to existing columns, no formula loops. Removing a column keeps the documents' entries for it on file (they come back if the column is re-added with the same id). PLAhelper picks changes up within a second.",
         "inputSchema": {"type": "object", "properties": {
            "action": {"type": "string", "enum": ["list", "get", "create", "rename", "remove", "edit"]}, "name": {"type": "string"}, "new_name": {"type": "string"}, "from": {"type": "string"},
            "add_columns": {"type": "array", "items": {"type": "object"}}, "update_columns": {"type": "array", "items": {"type": "object"}},
            "remove_columns": {"type": "array", "items": {"type": "string"}}, "order": {"type": "array", "items": {"type": "string"}}, "columns": {"type": "array", "items": {"type": "object"}}}, "required": ["action"]}},
        {"name": "hole_table", "annotations": {"destructiveHint": true},
         "description": "Hole tables (global; PLAhelper's hole panel and model_hole size use the active one): per size (\"M4\") clearance (through-hole ⌀), tap (tap drill ⌀), cbD / cbDepth (counterbore), cskD (90° countersink ⌀), in document units (built-in ISO metric in mm). action=list (tables + active) / get (name?: default active) / create (name, from?: table name / \"builtin\"; default copies the active one) / rename (name, new_name) / remove (name; not the last) / select (name: make it active; the hole panel switches) / edit (name?: default active; set: [{size, clearance?, tap?, cbD?, cbDepth?, cskD?}] updates a size or adds one (a new size needs all five), remove_sizes: [size]). Values are checked (> 0) and suspicious ones reported as warnings (tap ≥ clearance, counterbore / countersink not wider than the clearance hole).",
         "inputSchema": {"type": "object", "properties": {
            "action": {"type": "string", "enum": ["list", "get", "create", "rename", "remove", "select", "edit"]}, "name": {"type": "string"}, "new_name": {"type": "string"}, "from": {"type": "string"},
            "set": {"type": "array", "items": {"type": "object", "properties": {"size": {"type": "string"}, "clearance": {"type": "number"}, "tap": {"type": "number"}, "cbD": {"type": "number"}, "cbDepth": {"type": "number"}, "cskD": {"type": "number"}}, "required": ["size"]}},
            "remove_sizes": {"type": "array", "items": {"type": "string"}}}, "required": ["action"]}},
        {"name": "knowledge_bases", "annotations": {"readOnlyHint": true},
         "description": "Professional knowledge bases (Tencent ima, the user's account): list every knowledge base (own + subscribed) with id, name, item count, kind, role. query filters names. searched_by_default = the mechanical-design ones knowledge_search uses when kb is omitted.",
         "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}}},
        {"name": "knowledge_search", "annotations": {"readOnlyHint": true},
         "description": "Search the user's ima knowledge bases (design handbooks, GB standards, material data…) BEFORE answering engineering questions from memory, and cite what you use. ima matches TITLES by keyword: give short keywords (\"联轴器\", \"键连接\", \"GB/T 1095\"); several words are searched one by one and ranked by how many the title contains. kb = knowledge base names (fragments) or ids; omitted = the mechanical-design ones. Returns hits {kb, title, type, media_id, own}. Subscribed knowledge bases only expose titles; files of the user's own knowledge base can be fetched with knowledge_get.",
         "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}, "kb": {"type": "array", "items": {"type": "string"}}, "limit": {"type": "integer", "description": "max hits (default 30)"}}, "required": ["query"]}},
        {"name": "knowledge_get", "annotations": {"readOnlyHint": true},
         "description": "Look up one knowledge-base item (media_id from knowledge_search): notes come back as text; files return type + a temporary source link (nothing is downloaded - the user only wants queries). Items of subscribed knowledge bases are refused by ima (available: false). ima's API has no question-answering endpoint: answer from the titles you found plus your own engineering calculation, and cite the titles.",
         "inputSchema": {"type": "object", "properties": {"media_id": {"type": "string"}}, "required": ["media_id"]}},
        {"name": "background_bom", "annotations": {"destructiveHint": true},
         "description": "(Same as bom_export with doc=background.) Scan the BOM of the document open in the background instance (same as PLAhelper's BOM tab: template, user entries, part library, part identity tracking) and export it as CSV next to the document. scope = a configuration name (default: currently visible). Refused when the same document is open in the foreground.",
         "inputSchema": {"type": "object", "properties": {"scope": {"type": "string"}}}},
        {"name": "background_draw", "annotations": {"destructiveHint": true},
         "description": "(Same as draw_parts with doc=background.) Make 2D drawings in the background instance: STEP export from the hidden Plasticity + compile (PLAhelper's drawing pipeline, same settings / preferences / revision table as the drawing tab; ~20-90 s per sheet; part drawings, not assembly drawings). parts = [\"undrawn\"] (not drawn or model changed), [\"all\"], or keys / part numbers / names. export = force STEP re-export. Two or more sheets are also combined into one PDF. Refused when the same document is open in the foreground.",
         "inputSchema": {"type": "object", "properties": {"parts": {"type": "array", "items": {"type": "string"}}, "export": {"type": "boolean"}}, "required": ["parts"]}},
        {"name": "background_release", "annotations": {"destructiveHint": true},
         "description": "(Same as release with doc=background.) Write a release (external) package for the background document, like PLAhelper's release tab: records the version (a new release freezes an ECN of everything changed since the last one; amend = true re-sends the same batch), redraws the drawn parts the ECN touches so their revision table shows it, then writes the master BOM plus one folder per supplier (CNC / 3D printing / ... with that supplier's BOM columns and files). Refused when the same document is open in the foreground.",
         "inputSchema": {"type": "object", "properties": {"amend": {"type": "boolean"}}}}
    ]);
    // 所有 model_* 工具都可以指向后台实例
    if let Some(arr) = tools.as_array_mut() {
        for t in arr.iter_mut().filter(|t| t["name"].as_str().is_some_and(|n| n.starts_with("model_"))) {
            t["inputSchema"]["properties"]["instance"] = json!({"type": "string", "enum": ["foreground", "background"], "description": "which Plasticity: foreground (default, the user's) or background (the hidden instance started with the background tool)"});
        }
    }
    tools
}

/// tools/pla-cdp/pla-bg.js 在哪: 从程序目录往上找。
fn bg_script() -> Option<std::path::PathBuf> {
    let mut d = std::env::current_exe().ok()?.parent()?.to_path_buf();
    for _ in 0..7 {
        let p = d.join("tools").join("pla-cdp").join("pla-bg.js");
        if p.is_file() {
            return Some(p);
        }
        d = d.parent()?.to_path_buf();
    }
    None
}

/// background 工具: 起 / 查 / 停后台隐藏实例(node pla-bg.js, 一行 JSON)。
fn background_tool(a: &Value) -> Value {
    let action = a["action"].as_str().unwrap_or("status");
    if !matches!(action, "start" | "status" | "stop" | "open" | "save_as") {
        return err(format!("unknown action {action} (start / status / stop / open / save_as)"));
    }
    let Some(script) = bg_script() else { return err(String::from("tools/pla-cdp/pla-bg.js not found next to pla-mcp")) };
    let mut cmd = std::process::Command::new("node");
    cmd.arg(&script).arg(if action == "save_as" { "save-as" } else { action });
    if let Some(doc) = a["doc"].as_str().filter(|_| matches!(action, "start" | "open" | "save_as")) {
        cmd.arg("--doc").arg(doc);
    }
    if a["discard"].as_bool() == Some(true) && action == "open" {
        cmd.arg("--discard");
    }
    if matches!(action, "open" | "save_as") && a["doc"].as_str().is_none() {
        return err(format!("{action} needs doc (a .plasticity path)"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: 不弹控制台
        // 本进程的 stdin / stdout(MCP 客户端的管道)不能被继承: 否则一路传给常驻的后台 Plasticity / 代理,
        // 客户端那头的管道永远关不掉(实测: 调用方等 EOF 就一直挂着)
        for h in [std::io::stdin().as_raw_handle(), std::io::stdout().as_raw_handle(), std::io::stderr().as_raw_handle()] {
            unsafe {
                let _ = windows::Win32::Foundation::SetHandleInformation(
                    windows::Win32::Foundation::HANDLE(h as isize),
                    windows::Win32::Foundation::HANDLE_FLAG_INHERIT.0,
                    windows::Win32::Foundation::HANDLE_FLAGS(0),
                );
            }
        }
    }
    // 输出写临时文件、只等 node 退出: 它起的 Plasticity / 代理会继承管道句柄, 用 output() 等管道关闭就永远等不到(实测)
    let out_path = std::env::temp_dir().join(format!("pla-bg-{}.json", std::process::id()));
    let Ok(out_file) = std::fs::File::create(&out_path) else { return err(String::from("cannot create temp file")) };
    cmd.stdout(out_file).stderr(std::process::Stdio::null()).stdin(std::process::Stdio::null());
    match cmd.status() {
        Ok(_) => {
            let s = std::fs::read_to_string(&out_path).unwrap_or_default();
            let _ = std::fs::remove_file(&out_path);
            let line = s.lines().rev().find(|l| l.trim_start().starts_with('{')).unwrap_or("").to_string();
            match serde_json::from_str::<Value>(&line) {
                Ok(v) if v.get("error").is_some() => err(v["error"].as_str().unwrap_or("").to_string()),
                Ok(v) => text(serde_json::to_string_pretty(&v).unwrap_or_default()),
                Err(_) => err(format!("pla-bg.js said: {}", s.trim())),
            }
        }
        Err(e) => err(format!("cannot run node: {e}")),
    }
}

fn rt() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio"))
}

/// 连哪个 Plasticity 窗口: pla-ui 在跑就用它吸附的那个, 否则 0(代理挑唯一的那个)。
fn hwnd() -> isize {
    bridge::read_state().map(|s| s.hwnd).unwrap_or(0)
}

/// 连不上才提示 CDP 模式; 垫片报的参数 / 几何错误原样给。
fn cdp_err(e: impl std::fmt::Display) -> Value {
    let m = format!("{e:#}");
    let conn = ["connect", "timeout", "refused", "json/list", "no page", "no ws url"].iter().any(|k| m.to_lowercase().contains(k));
    if conn {
        err(format!("Cannot reach Plasticity over CDP (port {}): {m}. Is Plasticity running in CDP mode (PLAhelper settings → CDP MODE) with a document open? For instance=background, start it first with the background tool.", port()))
    } else {
        err(m)
    }
}

/// 参数对象里只留这些键(不认识的直接丢, 免得原样传进垫片)。
fn pick(a: &Value, keys: &[&str]) -> Value {
    let mut m = serde_json::Map::new();
    for k in keys {
        if let Some(v) = a.get(*k).filter(|v| !v.is_null()) {
            m.insert(k.to_string(), v.clone());
        }
    }
    Value::Object(m)
}

fn call_model_tool(name: &str, a: &Value) -> Option<Value> {
    use pla_bridge::shim::api;
    // instance = background → 后台隐藏实例(代理里只有它一个窗口, hwnd 0 = 让代理挑)
    let bg = a["instance"].as_str() == Some("background");
    PORT.store(if bg { BG_PORT } else { FG_PORT }, std::sync::atomic::Ordering::Relaxed);
    let h = if bg { 0 } else { hwnd() };
    if name == "background" {
        return Some(background_tool(a));
    }
    if DOC_TOOLS.contains(&name) {
        return Some(doc_tool(name, a));
    }
    let json_text = |v: &Value| text(serde_json::to_string_pretty(v).unwrap_or_default());
    let call = |api_name: &str, args: Value| match rt().block_on(pla_bridge::query::call_once(port(), h, api_name, &args)) {
        Ok(v) if v.get("err").is_some() => err(v["err"].to_string()),
        Ok(v) => json_text(&v),
        Err(e) => cdp_err(e),
    };
    Some(match name {
        "model_state" => {
            let args = pick(a, &["detail", "of", "surface", "curve", "normal", "angleTol", "near", "tol", "units", "limit"]);
            match rt().block_on(pla_bridge::query::state_once(port(), h, &args)) {
                Ok(v) => json_text(&v),
                Err(e) => cdp_err(e),
            }
        }
        "model_outline" => call(api::OUTLINE_READ, pick(a, &["kinds"])),
        "model_snaps" => call(api::GEO_SNAPS, pick(a, &["of", "kinds", "near", "tol", "units", "limit"])),
        "model_measure" => call(api::GEO_MEASURE, pick(a, &["op", "a", "b", "of", "faces", "units"])),
        "model_view" => {
            let mut args = pick(a, &["view", "of"]);
            if let Some(w) = a["width"].as_u64() {
                args["maxWidth"] = json!(w);
            }
            match rt().block_on(pla_bridge::query::view_png_once(port(), h, &args)) {
                Ok((png, meta)) => json!({"content": [{"type": "image", "data": b64(&png), "mimeType": "image/png"}, {"type": "text", "text": meta.to_string()}]}),
                Err(e) => cdp_err(e),
            }
        }
        "model_exec" => {
            let mut script = pick(a, &["ops", "units"]);
            script["atomic"] = json!(a["atomic"].as_bool().unwrap_or(true));
            match rt().block_on(pla_bridge::exec::run_once(port(), h, &script)) {
                Ok(v) => {
                    let (ok, msg) = pla_bridge::exec::summary(&v);
                    let mut r = json_text(&v);
                    r["content"].as_array_mut().map(|c| c.insert(0, json!({"type": "text", "text": msg})));
                    if !ok {
                        r["isError"] = json!(true);
                    }
                    r
                }
                Err(e) => cdp_err(e),
            }
        }
        "model_part" => {
            let q: pla_ui::fasteners::PartReq = match serde_json::from_value(a.clone()) {
                Ok(q) => q,
                Err(e) => return Some(err(format!("bad arguments: {e}"))),
            };
            let x = match pla_ui::fasteners::expand(&q) {
                Ok(x) => x,
                Err(e) => return Some(err(e)),
            };
            let script = json!({"ops": x.ops, "atomic": true});
            match rt().block_on(pla_bridge::exec::run_once(port(), h, &script)) {
                Ok(v) => {
                    let (ok, msg) = pla_bridge::exec::summary(&v);
                    let part = v["results"].as_array().and_then(|r| r.last()).map(|r| r["items"].clone()).unwrap_or(Value::Null);
                    let mut r = json_text(&json!({"part": part, "spec": x.spec, "standard": x.text, "steps": v}));
                    r["content"].as_array_mut().map(|c| c.insert(0, json!({"type": "text", "text": format!("{} — {msg}", x.text)})));
                    if !ok {
                        r["isError"] = json!(true);
                    }
                    r
                }
                Err(e) => cdp_err(e),
            }
        }
        "model_assets" => call(api::ASSET_LIST, pick(a, &["pack", "units"])),
        "model_asset_place" => call(api::ASSET_PLACE, pick(a, &["asset", "pack", "at", "normal", "rotate", "units"])),
        "model_part_stretch" => call(api::PART_STRETCH, pick(a, &["solid", "axis", "delta", "zone", "units"])),
        "model_asset_library" => {
            let register = |paths: Vec<String>| rt().block_on(pla_bridge::query::call_once(port(), h, api::ASSET_REGISTER, &json!({ "paths": paths })));
            match a["action"].as_str().unwrap_or("series") {
                "series" => {
                    let dir = pla_ui::asset_lib::build_dir();
                    let list: Vec<Value> = pla_ui::asset_lib::series()
                        .into_iter()
                        .map(|(id, code, zh, en)| {
                            let file = pla_ui::asset_lib::pack_file(id).unwrap_or_default();
                            json!({ "series": id, "code": code, "name": zh, "en": en, "built": dir.join(&file).is_file(), "pack": file })
                        })
                        .collect();
                    json_text(&json!({ "dir": dir.display().to_string(), "series": list }))
                }
                "install" => match pla_ui::asset_lib::install() {
                    Ok(ps) if ps.is_empty() => err(String::from("no packs built yet - build a series first")),
                    Ok(ps) => match register(ps.iter().map(|p| p.display().to_string()).collect()) {
                        Ok(v) => json_text(&json!({ "registered": ps.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(), "packs": v["packs"] })),
                        Err(e) => cdp_err(e),
                    },
                    Err(e) => err(e),
                },
                "build" => {
                    let id = a["series"].as_str().unwrap_or("");
                    let sizes: Vec<f64> = a["sizes"].as_array().map(|s| s.iter().filter_map(|x| x.as_f64().or_else(|| x.as_str().and_then(|t| t.trim_start_matches(['M', 'm']).parse().ok()))).collect()).unwrap_or_default();
                    let (Some(file), Ok(items)) = (pla_ui::asset_lib::pack_file(id), pla_ui::asset_lib::build_items(id, &sizes)) else {
                        return Some(err(pla_ui::asset_lib::build_items(id, &sizes).err().unwrap_or_else(|| format!("unknown series {id}"))));
                    };
                    let path = pla_ui::asset_lib::build_dir().join(file).display().to_string();
                    match rt().block_on(pla_bridge::query::build_pack_once(port(), h, &items, &path)) {
                        Ok(v) => {
                            let reg: Result<Value, String> = pla_ui::asset_lib::install().and_then(|ps| register(ps.iter().map(|p| p.display().to_string()).collect()).map_err(|e| e.to_string()));
                            json_text(&json!({ "built": v, "registered": reg.is_ok(), "registerError": reg.err() }))
                        }
                        Err(e) => cdp_err(e),
                    }
                }
                x => err(format!("unknown action {x} (series / build / install)")),
            }
        }
        "model_build" => match build_spec(h, a) {
            Ok(v) => json_text(&v),
            Err(e) => err(e),
        },
        "model_hole" => match hole_from_table(a) {
            Ok(a) if a["d"].is_number() => call(api::HOLE_MAKE, pick(&a, &["face", "type", "d", "depth", "cbD", "cbDepth", "cskD", "tapD", "at", "offset", "points", "slots", "tapAt", "stack", "units"])),
            Ok(_) => err("model_hole needs d or size"),
            Err(e) => err(e),
        },
        "model_transaction" => match a["action"].as_str().unwrap_or("") {
            "begin" => call(api::HIST_BEGIN, Value::Null),
            "commit" => call(api::HIST_COMMIT, Value::Null),
            "rollback" => call(api::HIST_ROLLBACK, Value::Null),
            "undo" => call(api::HIST_UNDO, json!({"steps": a["steps"].as_u64().unwrap_or(1)})),
            "state" => call(api::HIST_STATE, Value::Null),
            x => err(format!("unknown action {x}")),
        },
        _ => return None,
    })
}

fn text(t: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": t.into()}]})
}

fn err(t: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": t.into()}], "isError": true})
}

/// pla-ui 的状态; 零件清单空(还没扫过 BOM)→ 让它扫一次(只读), 最多等 30 s。
fn state() -> Result<bridge::State, String> {
    let st = bridge::read_state()?;
    if !st.parts.is_empty() {
        return Ok(st);
    }
    bridge::call(&json!({"op": "scan"}), Duration::from_secs(10))?;
    let t0 = std::time::Instant::now();
    while t0.elapsed() < Duration::from_secs(30) {
        std::thread::sleep(Duration::from_millis(500));
        let s = bridge::read_state()?;
        if !s.parts.is_empty() {
            return Ok(s);
        }
    }
    Ok(bridge::read_state()?)
}

fn sheets(st: &bridge::State) -> drawing::Sheets {
    drawing::load_in(std::path::Path::new(&st.drawings_dir))
}

fn status_of(sh: &drawing::Sheets, key: &str) -> String {
    match sh.get(key) {
        Some(s) if s.locked() => "uploaded (locked)".into(),
        Some(s) if s.model_changed => "model changed, redraw".into(),
        Some(s) if !s.compiled.is_empty() => format!("drawn {}", s.compiled),
        _ => "not drawn".into(),
    }
}

fn resolve_many(st: &bridge::State, qs: &[String]) -> Result<Vec<PartInfo>, String> {
    qs.iter().map(|q| bridge::resolve(&st.parts, q).cloned()).collect()
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

fn call_tool(name: &str, a: &Value) -> Value {
    // 建模工具不需要 pla-ui(直接连 Plasticity)
    if let Some(r) = call_model_tool(name, a) {
        return r;
    }
    let st = match state() {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let ask = |req: Value, secs: u64| bridge::call(&req, Duration::from_secs(secs));
    match name {
        "status" => text(
            json!({"document": st.doc_key, "parts": st.parts.len(), "selected": st.selected, "drawing_now": st.drawing, "queued": st.queued, "drawings_dir": st.drawings_dir}).to_string(),
        ),
        "list_parts" => {
            let sh = sheets(&st);
            let rows: Vec<Value> = st
                .parts
                .iter()
                .map(|p| json!({"key": p.key, "pn": p.pn, "name": p.name, "qty": p.qty, "material": p.material, "supplier": p.supplier, "bought_or_standard": p.bought, "drawing": status_of(&sh, &p.key)}))
                .collect();
            text(serde_json::to_string_pretty(&rows).unwrap_or_default())
        }
        "get_drawing" | "show_part" => {
            let p = match bridge::resolve(&st.parts, a["part"].as_str().unwrap_or("")) {
                Ok(p) => p.clone(),
                Err(e) => return err(e),
            };
            if name == "show_part" {
                return match ask(json!({"op": "show", "key": p.key}), 10) {
                    Ok(v) if v["ok"] == true => text(format!("Opened {} {} in PLAhelper.", p.pn, p.name)),
                    Ok(v) => err(v["error"].as_str().unwrap_or("failed").to_string()),
                    Err(e) => err(e),
                };
            }
            let sh = sheets(&st);
            let dir = std::path::Path::new(&st.drawings_dir);
            let Some(s) = sh.get(&p.key) else {
                return text(format!("{} {}: no drawing yet. Use draw to create it.", p.pn, p.name));
            };
            let rep = drawing::load_report(dir, s);
            let mut title = std::collections::BTreeMap::new();
            for (k, v) in [("pn", &p.pn), ("name", &p.name), ("material", &p.material)] {
                if !v.is_empty() {
                    title.insert(k.to_string(), v.clone());
                }
            }
            title.insert("qty".into(), p.qty.to_string());
            let mut out = draw_agent::context(s, rep.as_ref(), &title, s.locked());
            out.push_str(&format!("\n[Status] {}\n[PDF] {}", status_of(&sh, &p.key), drawing::file(dir, s, "pdf").display()));
            if let Some(r) = &rep {
                let stale = drawing::stale_refs(s, r);
                if !stale.is_empty() {
                    out.push_str(&format!("\n[Stale corrections - names not in the current report] {}", stale.join(", ")));
                }
            }
            let mut content = vec![json!({"type": "text", "text": out})];
            let png = s
                .uploaded
                .as_ref()
                .filter(|u| u.locked && !u.pages.is_empty())
                .map(|u| dir.join(&u.pages[0].png))
                .unwrap_or_else(|| drawing::file(dir, s, "png"));
            if let Ok(b) = std::fs::read(&png) {
                content.push(json!({"type": "image", "data": b64(&b), "mimeType": "image/png"}));
            }
            json!({"content": content})
        }
        "apply_drawing_changes" => {
            let qs: Vec<String> = serde_json::from_value(a["parts"].clone()).unwrap_or_default();
            let parts = match resolve_many(&st, &qs) {
                Ok(p) if !p.is_empty() => p,
                Ok(_) => return err("parts is empty"),
                Err(e) => return err(e),
            };
            let mut patch = a.clone();
            if let Some(o) = patch.as_object_mut() {
                o.remove("parts");
                o.remove("redraw");
            }
            if let Err(e) = serde_json::from_value::<draw_agent::Patch>(patch.clone()) {
                return err(format!("invalid patch: {e}"));
            }
            match ask(json!({"op": "apply", "keys": parts.iter().map(|p| &p.key).collect::<Vec<_>>(), "patch": patch, "redraw": a["redraw"].as_bool().unwrap_or(false)}), 20) {
                Ok(v) => text(serde_json::to_string_pretty(&v["results"]).unwrap_or_default()),
                Err(e) => err(e),
            }
        }
        "draw" => {
            let qs: Vec<String> = serde_json::from_value(a["parts"].clone()).unwrap_or_default();
            let sh = sheets(&st);
            let keys: Vec<String> = if qs.iter().any(|q| q == "undrawn") {
                st.parts.iter().filter(|p| !p.bought && sh.get(&p.key).map_or(true, |s| !s.locked() && (s.compiled.is_empty() || s.model_changed))).map(|p| p.key.clone()).collect()
            } else {
                match resolve_many(&st, &qs) {
                    Ok(p) => p.into_iter().map(|p| p.key).collect(),
                    Err(e) => return err(e),
                }
            };
            if keys.is_empty() {
                return text("Nothing to draw.");
            }
            let dir = std::path::PathBuf::from(&st.drawings_dir);
            let before: Vec<Option<std::time::SystemTime>> = keys.iter().map(|k| report_mtime(&dir, &sh, k)).collect();
            let r = match ask(json!({"op": "draw", "keys": keys, "export": a["export"].as_bool().unwrap_or(false)}), 20) {
                Ok(v) => v,
                Err(e) => return err(e),
            };
            if r["ok"] != true {
                return err(r["error"].as_str().unwrap_or("failed").to_string());
            }
            let wait = a["wait_seconds"].as_u64().unwrap_or(0).min(600);
            let t0 = std::time::Instant::now();
            let done = loop {
                let sh2 = drawing::load_in(&dir);
                let done: Vec<String> = keys.iter().zip(&before).filter(|(k, b)| report_mtime(&dir, &sh2, k) != **b).map(|(k, _)| k.clone()).collect();
                if done.len() == keys.len() || t0.elapsed().as_secs() >= wait {
                    break done;
                }
                std::thread::sleep(Duration::from_secs(2));
            };
            text(format!(
                "Queued {} sheet(s); {} finished{}. Skipped: {}",
                keys.len(),
                done.len(),
                if done.len() < keys.len() { " (the rest are still compiling; call get_drawing later)" } else { "" },
                r["skipped"]
            ))
        }
        "list_rules" => {
            let p = drawing::Prefs::load();
            text(if p.agent_rules.is_empty() { "(no rules yet)".to_string() } else { p.agent_rules.iter().map(|r| format!("- {r}")).collect::<Vec<_>>().join("\n") })
        }
        "remember_rule" => match ask(json!({"op": "rule", "rule": a["rule"]}), 10) {
            Ok(v) if v["ok"] == true => text("Saved."),
            Ok(v) => err(v["error"].as_str().unwrap_or("failed").to_string()),
            Err(e) => err(e),
        },
        _ => err(format!("unknown tool {name}")),
    }
}

// ---------------------------------------------------------------- A7 / A8: 按规格建模(逐特征执行 + 语义引用 + 验收 + 回滚 + 存档)

fn shim(h: isize, name: &str, args: Value) -> Result<Value, String> {
    let v = rt().block_on(pla_bridge::query::call_once(port(), h, name, &args)).map_err(|e| format!("{e:#}"))?;
    if let Some(e) = v.get("err") {
        return Err(e.to_string());
    }
    Ok(v)
}

fn specs_dir(doc_key: &str) -> std::path::PathBuf {
    pla_ui::dims::STORE.dir(doc_key).join("specs")
}

fn build_spec(h: isize, a: &Value) -> Result<Value, String> {
    use pla_bridge::shim::api;
    use pla_ui::spec::{self, Action, Params, State};
    let sp: spec::Spec = serde_json::from_value(a["spec"].clone()).map_err(|e| format!("bad spec: {e}"))?;
    if sp.name.trim().is_empty() || sp.features.is_empty() {
        return Err("spec needs a name and at least one feature".into());
    }
    let safe: String = sp.name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let doc = bridge::read_state().ok().map(|s| s.doc_key);
    let dir = doc.as_deref().map(specs_dir);
    let units = sp.units.clone();
    let with_units = |mut v: Value| {
        if let Some(u) = &units {
            v["units"] = json!(u);
        }
        v
    };
    // replace: 上一次按这份规格建的东西还在撤销栈顶 → 先撤掉
    if a["replace"].as_bool() == Some(true) {
        let map: Option<Value> = dir.as_ref().and_then(|d| std::fs::read_to_string(d.join(format!("{safe}.map.json"))).ok()).and_then(|t| serde_json::from_str(&t).ok());
        if let Some(m) = map {
            let (s0, s1) = (m["undo_start"].as_u64().unwrap_or(0), m["undo_end"].as_u64().unwrap_or(0));
            let now = shim(h, api::HIST_STATE, Value::Null)?["undo"].as_u64().unwrap_or(0);
            if now != s1 {
                return Err(format!("replace: the previous build of {} ended at undo depth {s1}, now {now} - something else happened since; undo it yourself or build without replace", sp.name));
            }
            if s1 > s0 {
                shim(h, api::HIST_UNDO, json!({"steps": s1 - s0}))?;
            }
        }
    }
    let start = shim(h, api::HIST_BEGIN, Value::Null)?["depth"].as_u64().unwrap_or(0);
    let p = Params::new(&sp.params);
    let mut st = State::default();
    let mut log: Vec<Value> = Vec::new();
    let query = |st: &State, detail: &str| -> Result<Value, String> {
        let of: Vec<i64> = st.bodies.values().copied().collect();
        if of.is_empty() {
            return Ok(json!({}));
        }
        shim(h, api::GEO_QUERY, with_units(json!({"detail": detail, "of": of})))
    };
    let fail = |msg: String| -> Result<Value, String> {
        let rb = shim(h, api::HIST_ROLLBACK, Value::Null).map(|v| format!("rolled back {} step(s)", v["undone"])).unwrap_or_else(|e| format!("ROLLBACK FAILED: {e}"));
        Err(format!("{msg} — {rb}"))
    };
    let mut last_body = String::new();
    for f in &sp.features {
        let fid = f["id"].as_str().unwrap_or("?").to_string();
        let ty = f["type"].as_str().unwrap_or("");
        let geo = if matches!(ty, "fillet" | "chamfer") {
            match query(&st, "all") {
                Ok(g) => g,
                Err(e) => return fail(format!("{fid}: query failed: {e}")),
            }
        } else {
            json!({})
        };
        let c = match spec::compile(f, &p, &st, &geo) {
            Ok(c) => c,
            Err(e) => return fail(e),
        };
        st.features.insert(fid.clone(), (c.body.clone(), c.bbox));
        for act in c.actions {
            match act {
                Action::Exec { ops, body } => {
                    let script = with_units(json!({"ops": ops}));
                    let v = match rt().block_on(pla_bridge::exec::run_once(port(), h, &script)) {
                        Ok(v) => v,
                        Err(e) => return fail(format!("{fid}: {e:#}")),
                    };
                    if !v["failed"].is_null() {
                        return fail(format!("{fid}: {}", v["failed"]["err"]));
                    }
                    let solid = v["results"].as_array().and_then(|r| r.last()).and_then(|r| r["items"].as_array()).and_then(|i| i.last()).and_then(|i| i["id"].as_i64());
                    match solid {
                        Some(s) => {
                            st.bodies.insert(body.clone(), s);
                        }
                        None => return fail(format!("{fid}: the step produced no solid")),
                    }
                    log.push(json!({"feature": fid, "ops": ops, "solid": solid}));
                    last_body = body;
                }
                Action::Hole { face, mut args } => {
                    let geo = match query(&st, "faces") {
                        Ok(g) => g,
                        Err(e) => return fail(format!("{fid}: query failed: {e}")),
                    };
                    let (face_id, body) = match spec::resolve_face(&face, &st, &geo) {
                        Ok(x) => x,
                        Err(e) => return fail(format!("{fid}: {e}")),
                    };
                    args["face"] = json!(face_id);
                    let v = match shim(h, api::HOLE_MAKE, with_units(args.clone())) {
                        Ok(v) => v,
                        Err(e) => return fail(format!("{fid}: hole failed: {e}")),
                    };
                    if let Some(new) = v["solids"].as_object().and_then(|m| m.values().last()).and_then(|x| x.as_i64()) {
                        st.bodies.insert(body.clone(), new);
                    }
                    log.push(json!({"feature": fid, "hole": args}));
                    last_body = body;
                }
            }
        }
    }
    // 验收
    let geo = match query(&st, "faces") {
        Ok(g) => g,
        Err(e) => return fail(format!("checks: query failed: {e}")),
    };
    let mass = |solid: i64| -> Option<f64> { shim(h, api::GEO_MEASURE, with_units(json!({"op": "mass", "of": [solid]}))).ok().and_then(|v| v["solids"][0]["volume"].as_f64()) };
    let fails = spec::check(&sp.checks, &p, &st, &geo, &mass, &last_body);
    if !fails.is_empty() {
        return fail(format!("checks failed: {}", fails.join("; ")));
    }
    shim(h, api::HIST_COMMIT, Value::Null)?;
    let end = shim(h, api::HIST_STATE, Value::Null)?["undo"].as_u64().unwrap_or(0);
    let map = json!({"name": sp.name, "bodies": st.bodies, "features": st.features, "undo_start": start, "undo_end": end, "date": pla_ui::parts_lib::today()});
    let mut saved = Value::Null;
    if let Some(d) = &dir {
        let _ = std::fs::create_dir_all(d);
        let w = |n: &str, v: &Value| std::fs::write(d.join(n), serde_json::to_vec_pretty(v).unwrap_or_default());
        let ok = w(&format!("{safe}.spec.json"), &serde_json::to_value(&sp).unwrap_or_default()).is_ok() && w(&format!("{safe}.map.json"), &map).is_ok();
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(d.join(format!("{safe}.log.jsonl"))) {
            use std::io::Write;
            let _ = writeln!(f, "{}", json!({"date": pla_ui::parts_lib::today(), "steps": log, "map": map}));
        }
        saved = json!(if ok { d.display().to_string() } else { "save failed".into() });
    }
    Ok(json!({"built": sp.name, "bodies": st.bodies, "steps": end.saturating_sub(start), "checks": format!("{} passed", sp.checks.len()), "saved": saved}))
}

fn report_mtime(dir: &std::path::Path, sh: &drawing::Sheets, key: &str) -> Option<std::time::SystemTime> {
    let s = sh.get(key)?;
    std::fs::metadata(dir.join(format!("{}.report.json", s.base))).and_then(|m| m.modified()).ok()
}

fn handle(msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg["method"].as_str().unwrap_or("");
    let result = match method {
        "initialize" => {
            let want = msg["params"]["protocolVersion"].as_str().unwrap_or("");
            let v = if SUPPORTED.contains(&want) { want } else { SUPPORTED[0] };
            json!({"protocolVersion": v, "capabilities": {"tools": {}}, "serverInfo": {"name": "plahelper", "version": env!("CARGO_PKG_VERSION")},
                   "instructions": "PLAhelper drawing tools for the user's Plasticity model (Chinese GB part drawings). Start with list_parts / get_drawing; use dimension names from get_drawing when changing settings. pla-ui must be running."})
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => call_tool(msg["params"]["name"].as_str().unwrap_or(""), &msg["params"]["arguments"]),
        _ if id.is_none() => return None, // 通知(notifications/initialized 等)不回
        _ => return Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("method not found: {method}")}})),
    };
    id.map(|id| json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(line.trim_start_matches('\u{feff}')) {
            Ok(m) => handle(&m),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}})),
        };
        if let Some(r) = reply {
            let _ = writeln!(out, "{r}");
            let _ = out.flush();
        }
    }
}

// ---------------------------------------------------------------- 文档作业(前台 / 后台; 用户 2026-11-18: 用语言描述就能完成任务 —— BOM 读改、出图、外发包、配置、尺寸)
// doc = "foreground"(默认, 用户正在用的文档)/ "background"(后台实例里开着的已保存文档)。
// 后台: 每次先确认前台没开同名文档(同名就报警拒绝)。逻辑与 pla-ui 共用 pla_ui::doc_jobs; 数据写进该文档的 .pla/… 存档,
// pla-ui 按修改时间重读 BOM 填写 / 供应商, 所以前台改了界面里马上看得到。

/// 后台实例的状态文件(pla-bg.js 写)。
fn bg_state() -> Value {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let p = std::path::Path::new(&base).join("PLAhelper").join("bg-plasticity").join("state.json");
    std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

fn doc_stem(p: &str) -> String {
    let b = std::path::Path::new(p).file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
    let b = b.trim_start_matches('*').trim();
    let lower = b.to_lowercase();
    lower.strip_suffix(".plasticity").unwrap_or(&lower).trim().to_string()
}

/// 前台 Plasticity 各窗口的标题(前台没开 = 空)。
fn foreground_titles() -> Vec<String> {
    let js = "(() => JSON.stringify(process.mainModule.require('electron').BrowserWindow.getAllWindows().map((w) => w.getTitle())))()";
    match rt().block_on(pla_bridge::export::main_eval_once(pla_bridge::export::INSPECT_PORT, js)) {
        Ok(v) => serde_json::from_str::<Vec<String>>(v.as_str().unwrap_or("[]")).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn title_key(t: &str) -> String {
    let base = t.split(" - ").next().unwrap_or(t).trim().trim_start_matches('*').trim();
    base.strip_suffix(".plasticity").unwrap_or(base).to_string()
}

/// 作业对象。
struct Doc {
    key: String,
    port: u16,
    hwnd: isize,
    bg: bool,
    label: String,
}

/// 后台文档: (doc_key, 路径)。没开文档 / 前台开着同名 → 错误(后者是报警)。
fn bg_doc() -> Result<(String, std::path::PathBuf), String> {
    let st = bg_state();
    let Some(doc) = st["doc"].as_str().filter(|d| !d.is_empty()) else {
        return Err("the background instance has no saved document open - background {action: open, doc} (or start with doc) first".into());
    };
    let js = "(() => { const e = process.mainModule.require('electron'); return JSON.stringify((e.BaseWindow || e.BrowserWindow).getAllWindows().filter((w) => w.webContents).map((w) => w.webContents.getTitle())); })()";
    let titles: Vec<String> = rt()
        .block_on(pla_bridge::export::main_eval_once(pla_bridge::export::inspect_port_for(BG_PORT), js))
        .ok()
        .and_then(|v| serde_json::from_str(v.as_str().unwrap_or("[]")).ok())
        .ok_or("the background instance is not running - background {action: start} first")?;
    let stem = doc_stem(doc);
    if !titles.iter().any(|t| doc_stem(&title_key(t)) == stem) {
        return Err(format!("the background instance no longer shows {doc} (titles: {titles:?}) - open it again"));
    }
    if foreground_titles().iter().any(|t| doc_stem(&title_key(t)) == stem) {
        return Err(format!(
            "REFUSED: \"{}\" is also open in your foreground Plasticity - background jobs must not work on a document that is open in the foreground (both would write the same files). Close it in the foreground first. / 拒绝执行: 前台 Plasticity 也开着这个文档, 后台不能同时处理同一个文档。请先在前台关掉它。",
            std::path::Path::new(doc).file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default()
        ));
    }
    let key = std::path::Path::new(doc).file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
    let path = std::path::PathBuf::from(doc);
    pla_ui::snapshot::pin_doc_path(&key, &path);
    Ok((key, path))
}

fn doc_target(a: &Value) -> Result<Doc, String> {
    if a["doc"].as_str() == Some("background") {
        let (key, path) = bg_doc()?;
        return Ok(Doc { key, port: BG_PORT, hwnd: 0, bg: true, label: path.display().to_string() });
    }
    // 前台: pla-ui 在跑就用它绑定的文档 / 窗口; 否则看前台窗口标题
    if let Ok(st) = bridge::read_state() {
        if !st.doc_key.is_empty() && st.doc_key != "untitled" && bridge::now().saturating_sub(st.ts) < 30 {
            return Ok(Doc { label: st.doc_key.clone(), key: st.doc_key, port: FG_PORT, hwnd: st.hwnd, bg: false });
        }
    }
    let titles: Vec<String> = foreground_titles().into_iter().filter(|t| !matches!(doc_stem(&title_key(t)).as_str(), "" | "untitled" | "plasticity")).collect();
    match titles.as_slice() {
        [] => Err("no saved document open in the foreground Plasticity (or it is not running in CDP mode)".into()),
        [t] => Ok(Doc { key: title_key(t), port: FG_PORT, hwnd: 0, bg: false, label: title_key(t) }),
        _ => Err(format!("several foreground documents are open ({}); start PLAhelper (pla-ui) so it knows which one is current", titles.join(", "))),
    }
}

/// 扫 BOM + 零件身份迁移。scope = 配置名(None = 当前可见)。
fn doc_scan(d: &Doc, scope: Option<&str>) -> Result<(Value, pla_ui::doc_jobs::Migrated), String> {
    let args = match scope {
        None => json!({}),
        Some(cf) => {
            let st = pla_ui::configs::STORE.load(&d.key, cf).ok_or_else(|| format!("configuration {cf} not found for {}", d.key))?;
            json!({ "members": pla_ui::configs::members(&st).into_iter().map(|m| m.0).collect::<Vec<_>>() })
        }
    };
    let scan = rt().block_on(pla_bridge::query::call_once(d.port, d.hwnd, pla_bridge::shim::api::BOM_SCAN, &args)).map_err(|e| format!("BOM scan: {e:#}"))?;
    let m = pla_ui::doc_jobs::identity_migrate(&d.key, &scan)?;
    Ok((scan, m))
}

const DOC_TOOLS: &[&str] = &[
    "bom_table", "bom_edit", "bom_export", "draw_parts", "release", "release_history", "suppliers", "configs", "dims", "parts_library",
    "bom_template", "hole_table", "background_bom", "background_draw", "background_release",
    "knowledge_bases", "knowledge_search", "knowledge_get",
];

/// K1 专业知识库(腾讯 ima, 只读)
fn knowledge_tool(name: &str, a: &Value) -> Result<Value, String> {
    use pla_ui::knowledge as k;
    let list = |v: &Value| -> Vec<String> {
        match v {
            Value::String(s) => vec![s.clone()],
            Value::Array(xs) => xs.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
            _ => vec![],
        }
    };
    match name {
        "knowledge_bases" => {
            let all = k::bases(a["query"].as_str().unwrap_or(""))?;
            let default: Vec<Value> = k::pick_bases(&all, &[]).iter().map(|b| b["name"].clone()).collect();
            Ok(json!({"bases": all, "searched_by_default": default}))
        }
        "knowledge_search" => k::search(a["query"].as_str().unwrap_or(""), &list(&a["kb"]), a["limit"].as_u64().unwrap_or(30) as usize),
        "knowledge_get" => k::get(a["media_id"].as_str().unwrap_or("")),
        _ => Err(format!("unknown tool {name}")),
    }
}

fn doc_tool(name: &str, a: &Value) -> Value {
    let r = (|| -> Result<Value, String> {
        // 旧名字 = 后台
        let (name, a) = match name.strip_prefix("background_") {
            Some(rest) => {
                let mut a2 = a.clone();
                a2["doc"] = json!("background");
                (match rest { "bom" => "bom_export", "draw" => "draw_parts", x => x }.to_string(), a2)
            }
            None => (name.to_string(), a.clone()),
        };
        if name == "suppliers" {
            return suppliers_tool(&a);
        }
        if name == "parts_library" {
            return parts_library_tool(&a);
        }
        if name == "bom_template" {
            return bom_template_tool(&a);
        }
        if name == "hole_table" {
            return hole_table_tool(&a);
        }
        if name.starts_with("knowledge_") {
            return knowledge_tool(&name, &a);
        }
        let d = doc_target(&a)?;
        match name.as_str() {
            "bom_table" => bom_table_tool(&d, &a),
            "bom_edit" => bom_edit_tool(&d, &a),
            "bom_export" => bom_export_tool(&d, &a),
            "draw_parts" => draw_tool(&d, &a),
            "release" => release_tool(&d, &a),
            "release_history" => release_history_tool(&d),
            "configs" => configs_tool(&d, &a),
            "dims" => dims_tool(&d, &a),
            x => Err(format!("unknown tool {x}")),
        }
    })();
    match r {
        Ok(v) => text(serde_json::to_string_pretty(&v).unwrap_or_default()),
        Err(e) => err(e),
    }
}

// ---- BOM

fn table_json(d: &Doc, scan: &Value, st: &pla_ui::doc_jobs::DocState) -> Value {
    let (rows, totals) = pla_ui::bom_table::build(scan, &st.tpl, &st.edits, &st.lib);
    let cols: Vec<Value> = st
        .tpl
        .columns
        .iter()
        .map(|c| json!({ "id": c.id, "header": c.header, "editable": c.source.as_deref() == Some("field") || c.source.is_none() && c.formula.is_none(), "source": c.source, "formula": c.formula, "default": c.default }))
        .collect();
    let parts = pla_ui::doc_jobs::bom_parts(scan, &st.tpl, &st.edits, &st.lib, &st.sheets.purchased);
    let rows_j: Vec<Value> = rows
        .iter()
        .map(|r| {
            let cells: serde_json::Map<String, Value> = st.tpl.columns.iter().zip(&r.cells).map(|(c, v)| (c.id.clone(), json!(v))).collect();
            let over: Vec<&str> = st.tpl.columns.iter().zip(&r.overridden).filter(|(_, o)| **o).map(|(c, _)| c.id.as_str()).collect();
            let sup = pla_ui::release::process_of(&st.edits, &r.key);
            let sh = st.sheets.get(&r.key);
            json!({
                "row": r.key, "manual": r.manual, "cells": cells, "overridden": over,
                "supplier": sup, "bought": parts.iter().any(|p| p.2.key == r.key && p.2.bought),
                "drawing": sh.map(|s| if s.locked() { "uploaded (locked)".to_string() } else if s.model_changed { "model changed".into() } else if !s.compiled.is_empty() { format!("drawn {}", s.compiled) } else { "not drawn".into() }).unwrap_or_else(|| "not drawn".into()),
            })
        })
        .collect();
    json!({
        "document": d.label, "template": if st.edits.template.is_empty() { "(default)".to_string() } else { st.edits.template.clone() },
        "templates": pla_ui::bom_table::list_in(&pla_ui::bom_table::dir()), "columns": cols, "rows": rows_j,
        "totals": st.tpl.columns.iter().zip(&totals).filter(|(_, t)| !t.is_empty()).map(|(c, t)| json!({ c.id.clone(): t })).collect::<Vec<_>>(),
        "excluded": st.edits.excluded, "defaults": st.edits.defaults, "project": st.edits.project,
        "suppliers": st.suppliers.list.iter().map(|s| json!({ "id": s.id, "name": s.name })).collect::<Vec<_>>(),
        "processColumn": pla_ui::release::PROCESS_COL,
    })
}

fn bom_table_tool(d: &Doc, a: &Value) -> Result<Value, String> {
    let (scan, _) = doc_scan(d, a["scope"].as_str().filter(|s| !s.is_empty()))?;
    let st = pla_ui::doc_jobs::DocState::load(&d.key);
    Ok(table_json(d, &scan, &st))
}

/// 行: key / "manual:<id>" / 料号 / 名字(名字按当前表里算好的 name 列)。
fn find_row(rows: &[pla_ui::bom_table::Row], tpl: &pla_ui::bom_table::Template, q: &str) -> Result<String, String> {
    let col = |id: &str| tpl.columns.iter().position(|c| c.id == id);
    let (pn, name) = (col("pn"), col("name"));
    let hit: Vec<&pla_ui::bom_table::Row> = rows
        .iter()
        .filter(|r| r.key == q || pn.is_some_and(|i| !r.cells[i].is_empty() && r.cells[i] == q) || name.is_some_and(|i| r.cells[i] == q))
        .collect();
    match hit.as_slice() {
        [r] => Ok(r.key.clone()),
        [] => Err(format!("no BOM row matches {q:?} (use the row key, part number or name from bom_table)")),
        _ => Err(format!("{q:?} matches {} rows - use the row key", hit.len())),
    }
}

fn find_col(tpl: &pla_ui::bom_table::Template, extra: &[pla_ui::bom_table::Col], q: &str) -> Result<String, String> {
    if q == pla_ui::release::PROCESS_COL {
        return Ok(q.to_string());
    }
    let low = q.to_lowercase();
    tpl.columns
        .iter()
        .chain(extra.iter())
        .find(|c| c.id == q || c.header.to_lowercase() == low)
        .map(|c| c.id.clone())
        .ok_or_else(|| format!("no column {q:?} (columns: {})", tpl.columns.iter().map(|c| format!("{}={}", c.id, c.header)).collect::<Vec<_>>().join(", ")))
}

fn bom_edit_tool(d: &Doc, a: &Value) -> Result<Value, String> {
    let (scan, _) = doc_scan(d, a["scope"].as_str().filter(|s| !s.is_empty()))?;
    let mut st = pla_ui::doc_jobs::DocState::load(&d.key);
    let mut log: Vec<String> = Vec::new();
    // 模板
    if let Some(t) = a["template"].as_str() {
        // "" = 回到默认(列表第一个)
        if !t.is_empty() && !pla_ui::bom_table::list_in(&pla_ui::bom_table::dir()).iter().any(|x| x == t) {
            return Err(format!("no BOM template {t}"));
        }
        st.edits.template = t.to_string();
        log.push(format!("template = {t}"));
        st = pla_ui::doc_jobs::DocState { edits: st.edits.clone(), ..pla_ui::doc_jobs::DocState::load(&d.key) };
        let dir = pla_ui::bom_table::dir();
        let name = if t.is_empty() { pla_ui::bom_table::list_in(&dir).first().cloned().unwrap_or_default() } else { t.to_string() };
        st.tpl = pla_ui::bom_table::load_in(&dir, &name).unwrap_or_else(|_| pla_ui::bom_table::builtin());
    }
    if let Some(p) = a["project"].as_str() {
        st.edits.project = p.to_string();
        log.push(format!("project = {p}"));
    }
    // 供应商自有列也能当列名用(填表面处理 / 颜色 …)
    let extra: Vec<pla_ui::bom_table::Col> = st.suppliers.list.iter().flat_map(|s| s.extra.clone()).collect();
    if let Some(m) = a["defaults"].as_object() {
        for (k, v) in m {
            let id = find_col(&st.tpl, &extra, k)?;
            match v.as_str().map(String::from).or_else(|| v.as_f64().map(|x| x.to_string())) {
                Some(val) => {
                    st.edits.defaults.insert(id.clone(), val.clone());
                    log.push(format!("default {id} = {val}"));
                }
                None => {
                    st.edits.defaults.remove(&id);
                    log.push(format!("default {id} cleared"));
                }
            }
        }
    }
    // 手动行
    for r in a["add_rows"].as_array().into_iter().flatten() {
        let id = st.edits.add_manual();
        for (k, v) in r.as_object().into_iter().flatten() {
            let col = find_col(&st.tpl, &extra, k)?;
            let val = v.as_str().map(String::from).unwrap_or_else(|| v.to_string());
            st.edits.set_cell(&format!("manual:{id}"), &col, &val, "\u{0}");
        }
        log.push(format!("added row manual:{id}"));
    }
    for r in a["remove_rows"].as_array().into_iter().flatten().filter_map(|x| x.as_str()) {
        let id = r.trim_start_matches("manual:");
        let n = st.edits.manual.len();
        st.edits.manual.retain(|m| m.id != id);
        if st.edits.manual.len() == n {
            return Err(format!("no manual row {r}"));
        }
        log.push(format!("removed row manual:{id}"));
    }
    let (rows, _) = pla_ui::bom_table::build(&scan, &st.tpl, &st.edits, &st.lib);
    // 排除 / 恢复
    for q in a["exclude"].as_array().into_iter().flatten().filter_map(|x| x.as_str()) {
        let k = find_row(&rows, &st.tpl, q)?;
        if !st.edits.excluded.contains(&k) {
            st.edits.excluded.push(k.clone());
        }
        log.push(format!("excluded {q}"));
    }
    for q in a["include"].as_array().into_iter().flatten().filter_map(|x| x.as_str()) {
        let before = st.edits.excluded.len();
        st.edits.excluded.retain(|k| k != q && find_row(&rows, &st.tpl, q).ok().as_deref() != Some(k.as_str()));
        if st.edits.excluded.len() == before {
            return Err(format!("{q} is not excluded"));
        }
        log.push(format!("included {q}"));
    }
    // 单元格
    for c in a["set"].as_array().into_iter().flatten() {
        let q = c["part"].as_str().ok_or("each set entry needs part")?;
        let k = find_row(&rows, &st.tpl, q)?;
        let col = find_col(&st.tpl, &extra, c["column"].as_str().ok_or("each set entry needs column")?)?;
        if c["value"].is_null() {
            if let Some(mid) = k.strip_prefix("manual:") {
                if let Some(m) = st.edits.manual.iter_mut().find(|m| m.id == mid) {
                    m.fields.remove(&col);
                }
            } else if let Some(m) = st.edits.parts.get_mut(&k) {
                m.remove(&col);
                if m.is_empty() {
                    st.edits.parts.remove(&k);
                }
            }
            log.push(format!("{q}.{col} reset"));
        } else {
            let val = c["value"].as_str().map(String::from).unwrap_or_else(|| c["value"].to_string());
            st.edits.set_cell(&k, &col, &val, "\u{0}");
            log.push(format!("{q}.{col} = {val}"));
        }
    }
    // 归哪家(外发): 供应商 id 或名字 / "buy" = 外购 / "" = 不分配
    for c in a["assign"].as_array().into_iter().flatten() {
        let q = c["part"].as_str().ok_or("each assign entry needs part")?;
        let k = find_row(&rows, &st.tpl, q)?;
        let s = c["supplier"].as_str().unwrap_or("");
        let id = if s.is_empty() || s == pla_ui::release::BUY {
            s.to_string()
        } else {
            st.suppliers.list.iter().find(|x| x.id == s || x.name.eq_ignore_ascii_case(s)).map(|x| x.id.clone()).ok_or_else(|| {
                format!("no supplier {s} (have: {}, or buy)", st.suppliers.list.iter().map(|x| format!("{}={}", x.id, x.name)).collect::<Vec<_>>().join(", "))
            })?
        };
        if id.is_empty() {
            if let Some(m) = st.edits.parts.get_mut(&k) {
                m.remove(pla_ui::release::PROCESS_COL);
            }
        } else {
            st.edits.set_cell(&k, pla_ui::release::PROCESS_COL, &id, "\u{0}");
        }
        log.push(format!("{q} → {}", if id.is_empty() { "unassigned" } else { &id }));
    }
    if log.is_empty() {
        return Err("nothing to change (set / assign / exclude / include / add_rows / remove_rows / defaults / template / project)".into());
    }
    pla_ui::bom_table::save_edits(&d.key, &st.edits)?;
    let mut out = table_json(d, &scan, &st);
    out["changed"] = json!(log);
    Ok(out)
}

fn bom_export_tool(d: &Doc, a: &Value) -> Result<Value, String> {
    let scope = a["scope"].as_str().filter(|s| !s.is_empty());
    let (scan, m) = doc_scan(d, scope)?;
    let st = pla_ui::doc_jobs::DocState::load(&d.key);
    let (rows, totals) = pla_ui::bom_table::build(&scan, &st.tpl, &st.edits, &st.lib);
    let label = scope.map_or_else(|| "当前可见".to_string(), |s| format!("配置 {s}"));
    let csv = pla_ui::bom::export_text(&d.key, &label, &pla_ui::bom_table::to_csv(&st.tpl, &rows, &totals))?;
    let parts: Vec<Value> = pla_ui::doc_jobs::bom_parts(&scan, &st.tpl, &st.edits, &st.lib, &st.sheets.purchased).into_iter().map(|p| json!(p.2)).collect();
    Ok(json!({ "document": d.label, "scope": label, "parts": parts, "csv": csv.display().to_string(), "identityRenames": m.renames.len() }))
}

// ---- 出图

/// 出一张零件图(导 STEP → 编译)。返回 (图名, PDF 路径)。
fn draw_one(d: &Doc, st: &mut pla_ui::doc_jobs::DocState, part: &pla_ui::doc_jobs::Part, export: bool, prefs: &pla_ui::drawing::Prefs, tools: &std::path::Path) -> Result<(String, String), String> {
    let (title, member, info) = part;
    if pla_ui::doc_jobs::ensure_sheet(&mut st.sheets, &info.key, title, member) {
        st.save_sheets()?;
    }
    let s = st.sheets.get(&info.key).cloned().ok_or("sheet vanished")?;
    if s.locked() {
        return Err(format!("{} is locked to an uploaded drawing", s.base));
    }
    let dir = pla_ui::drawing::dir(&d.key);
    let step = pla_ui::drawing::file(&dir, &s, "step");
    if export || !step.exists() {
        if s.member.is_empty() {
            return Err(format!("{}: no model solid (rescan the BOM)", s.base));
        }
        let v = rt()
            .block_on(pla_bridge::query::export_step_once(d.port, d.hwnd, &step.display().to_string(), std::slice::from_ref(&s.member), Some("AP214")))
            .map_err(|e| format!("{}: STEP export: {e:#}", s.base))?;
        let mut s2 = s.clone();
        if let Some(p) = v["parts"].as_array().filter(|a| a.len() == 1).and_then(|a| a.first()) {
            s2.volume = p["volume"].as_f64();
            s2.area = p["area"].as_f64();
        }
        s2.exported = pla_ui::parts_lib::today();
        st.sheets.upsert(s2);
        st.save_sheets()?;
    }
    let mut t = title.clone();
    t.extend(s.title.clone());
    let spec = pla_ui::doc_jobs::write_part_spec(&d.key, &info.key, &st.sheets, prefs, &t)?;
    pla_ui::drawing::compile(tools, &spec).map_err(|e| format!("{}: {e}", s.base))?;
    let mut s3 = st.sheets.get(&info.key).cloned().ok_or("sheet vanished")?;
    s3.compiled = pla_ui::parts_lib::today();
    s3.model_changed = false;
    let pdf = pla_ui::drawing::file(&dir, &s3, "pdf").display().to_string();
    let base = s3.base.clone();
    st.sheets.upsert(s3);
    st.save_sheets()?;
    Ok((base, pdf))
}

/// 选零件: ["all"] = 全部自制件, ["undrawn"] = 没出过 / 模型改过的, 否则按 key / 料号 / 名字。
fn pick_parts<'p>(parts: &'p [pla_ui::doc_jobs::Part], sheets: &pla_ui::drawing::Sheets, want: &[String]) -> (Vec<&'p pla_ui::doc_jobs::Part>, Vec<String>) {
    let mut out = Vec::new();
    let mut missing = Vec::new();
    let own = |p: &&pla_ui::doc_jobs::Part| !p.2.bought;
    if want.iter().any(|w| w == "all") {
        out.extend(parts.iter().filter(own));
    } else if want.iter().any(|w| w == "undrawn") {
        out.extend(parts.iter().filter(own).filter(|p| sheets.get(&p.2.key).is_none_or(|s| !s.locked() && (s.compiled.is_empty() || s.model_changed))));
    } else {
        for w in want {
            match parts.iter().find(|p| p.2.key == *w || (!p.2.pn.is_empty() && p.2.pn == *w) || p.2.name == *w) {
                Some(p) => out.push(p),
                None => missing.push(w.clone()),
            }
        }
    }
    (out, missing)
}

fn draw_tool(d: &Doc, a: &Value) -> Result<Value, String> {
    let want: Vec<String> = a["parts"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
    if want.is_empty() {
        return Err("parts is required: [\"undrawn\"], [\"all\"], or keys / part numbers / names".into());
    }
    let tools = pla_ui::drawing::tools_dir().ok_or("drawing needs the tools/drawing compiler")?;
    if pla_ui::drawing::freecad_bin().is_none() {
        return Err("drawing needs FreeCAD 1.1".into());
    }
    let (scan, m) = doc_scan(d, None)?;
    let mut st = pla_ui::doc_jobs::DocState::load(&d.key);
    let parts = pla_ui::doc_jobs::bom_parts(&scan, &st.tpl, &st.edits, &st.lib, &st.sheets.purchased);
    let (picked, missing) = pick_parts(&parts, &st.sheets, &want);
    let picked: Vec<pla_ui::doc_jobs::Part> = picked.into_iter().cloned().collect();
    let prefs = pla_ui::drawing::Prefs::load();
    let export = a["export"].as_bool().unwrap_or(false);
    let mut done = Vec::new();
    let mut failed = Vec::new();
    for p in &picked {
        match draw_one(d, &mut st, p, export, &prefs, &tools) {
            Ok((base, pdf)) => done.push(json!({ "part": base, "pdf": pdf })),
            Err(e) => failed.push(e),
        }
    }
    let mut book = Value::Null;
    if done.len() >= 2 {
        let dir = pla_ui::drawing::dir(&d.key);
        let pages: Vec<String> = picked.iter().filter_map(|p| st.sheets.get(&p.2.key)).flat_map(|s| pla_ui::drawing::book_pages(&dir, s)).collect();
        let out = pla_ui::drawing::book_file(&dir, &d.key);
        book = match pla_ui::drawing::book(&tools, &out, &pages) {
            Ok(_) => json!(out.display().to_string()),
            Err(e) => json!(format!("combine failed: {e}")),
        };
    }
    Ok(json!({ "document": d.label, "drawn": done, "failed": failed, "notFound": missing, "book": book, "identityRenames": m.renames.len(), "folder": pla_ui::drawing::dir(&d.key).display().to_string() }))
}

// ---- 外发

fn release_tool(d: &Doc, a: &Value) -> Result<Value, String> {
    use pla_ui::release as rel;
    let (scan, _) = doc_scan(d, None)?;
    let mut st = pla_ui::doc_jobs::DocState::load(&d.key);
    let parts = pla_ui::doc_jobs::bom_parts(&scan, &st.tpl, &st.edits, &st.lib, &st.sheets.purchased);
    let prefs = pla_ui::drawing::Prefs::load();
    let root = rel::STORE.dir(&d.key);
    let mut hist = rel::History::load(&root);
    if hist.open_by.trim().is_empty() {
        hist.open_by = prefs.title.get("designer").cloned().unwrap_or_default();
    }
    let amend = a["amend"].as_bool().unwrap_or(false) && hist.last().is_some();
    let (ver, no) = hist.record(pla_ui::doc_jobs::rel_snapshot(&d.key, &parts, &st.sheets), &pla_ui::parts_lib::today(), amend);
    hist.save(&root).map_err(|e| format!("saving the version history: {e}"))?;
    let ecn = no.as_ref().and_then(|n| hist.ecns.iter().find(|e| &e.no == n).cloned());
    let redraw: Vec<String> = ecn
        .as_ref()
        .map(|e| {
            e.items
                .iter()
                .filter(|i| !i.kinds.iter().any(|k| k == "removed"))
                .filter(|i| st.sheets.get(&i.key).is_some_and(|s| !s.locked() && (!s.compiled.is_empty() || s.model_changed)))
                .map(|i| i.key.clone())
                .collect()
        })
        .unwrap_or_default();
    let mut redrawn = Vec::new();
    let mut failed = Vec::new();
    if !redraw.is_empty() {
        let tools = pla_ui::drawing::tools_dir().ok_or("redrawing needs the tools/drawing compiler")?;
        for k in &redraw {
            if let Some(p) = parts.iter().find(|p| &p.2.key == k).cloned() {
                match draw_one(d, &mut st, &p, false, &prefs, &tools) {
                    Ok((base, _)) => redrawn.push(base),
                    Err(e) => failed.push(e),
                }
            }
        }
    }
    let (rows, totals) = pla_ui::bom_table::build(&scan, &st.tpl, &st.edits, &st.lib);
    let (packs, total) = pla_ui::doc_jobs::rel_packs(&d.key, &st.suppliers, &st.tpl, &rows, &totals, &st.edits, &st.sheets);
    let out = rel::write(&root, &ver, &pla_ui::parts_lib::today(), &total, &packs, ecn.as_ref())?;
    let unassigned: Vec<String> = parts.iter().filter(|p| !p.2.bought && p.2.supplier.is_empty()).map(|p| p.2.name.clone()).collect();
    Ok(json!({
        "document": d.label, "version": ver, "ecn": no, "amend": amend,
        "package": out.display().to_string(), "suppliers": packs.iter().map(|p| p.supplier.name.clone()).collect::<Vec<_>>(),
        "missingFiles": packs.iter().flat_map(|p| p.missing.iter().map(move |(part, m)| format!("{}: {} ({})", p.supplier.name, part, m.join(", ")))).collect::<Vec<_>>(),
        "unassignedParts": unassigned, "redrawn": redrawn, "redrawFailed": failed,
    }))
}

fn release_history_tool(d: &Doc) -> Result<Value, String> {
    let root = pla_ui::release::STORE.dir(&d.key);
    let hist = pla_ui::release::History::load(&root);
    Ok(json!({ "document": d.label, "folder": root.display().to_string(), "history": serde_json::to_value(&hist).unwrap_or(Value::Null) }))
}

/// 供应商(全局): list / add / update / remove。
fn suppliers_tool(a: &Value) -> Result<Value, String> {
    use pla_ui::release as rel;
    let mut s = rel::Suppliers::load();
    let action = a["action"].as_str().unwrap_or("list");
    let cols = |v: &Value| -> Option<Vec<pla_ui::bom_table::Col>> {
        v.as_array().map(|arr| {
            arr.iter()
                .filter_map(|c| {
                    let header = c["header"].as_str().or_else(|| c.as_str())?.to_string();
                    let id = c["id"].as_str().map(String::from).unwrap_or_else(|| header.to_lowercase().chars().map(|ch| if ch.is_alphanumeric() { ch } else { '_' }).collect());
                    Some(pla_ui::bom_table::Col { id, header, source: Some("field".into()), formula: None, default: c["default"].as_str().map(String::from), sum: false })
                })
                .collect()
        })
    };
    let strs = |v: &Value| v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect::<Vec<_>>());
    match action {
        "list" => {}
        "add" => {
            let name = a["name"].as_str().ok_or("add needs name")?;
            let id = a["id"].as_str().map(String::from).unwrap_or_else(|| s.next_id());
            if s.get(&id).is_some() {
                return Err(format!("supplier id {id} exists"));
            }
            s.list.push(rel::Supplier {
                id,
                name: name.into(),
                files: strs(&a["files"]).unwrap_or_else(|| vec!["pdf".into(), "step".into()]),
                columns: strs(&a["columns"]).unwrap_or_else(|| vec!["pn".into(), "name".into(), "total_qty".into()]),
                extra: cols(&a["extra"]).unwrap_or_default(),
            });
            s.save()?;
        }
        "update" | "remove" => {
            let q = a["id"].as_str().or(a["name"].as_str()).ok_or("needs id or name")?;
            let i = s.list.iter().position(|x| x.id == q || x.name.eq_ignore_ascii_case(q)).ok_or_else(|| format!("no supplier {q}"))?;
            if action == "remove" {
                s.list.remove(i);
            } else {
                let x = &mut s.list[i];
                if let Some(n) = a["new_name"].as_str() {
                    x.name = n.into();
                }
                if let Some(f) = strs(&a["files"]) {
                    if let Some(bad) = f.iter().find(|k| !rel::FILE_KINDS.contains(&k.as_str())) {
                        return Err(format!("file kind {bad} (pdf / dxf / step)"));
                    }
                    x.files = f;
                }
                if let Some(c) = strs(&a["columns"]) {
                    x.columns = c;
                }
                if let Some(e) = cols(&a["extra"]) {
                    x.extra = e;
                }
            }
            s.save()?;
        }
        x => return Err(format!("unknown action {x} (list / add / update / remove)")),
    }
    Ok(json!({ "suppliers": serde_json::to_value(&s.list).unwrap_or(Value::Null), "file": rel::Suppliers::path().display().to_string() }))
}

// ---- 配置(可见性 + 材质指派)

fn configs_tool(d: &Doc, a: &Value) -> Result<Value, String> {
    let store = pla_ui::configs::STORE;
    match a["action"].as_str().unwrap_or("list") {
        "list" => {
            let list: Vec<Value> = store
                .list(&d.key)
                .into_iter()
                .map(|m| {
                    let brief = store.load(&d.key, &m.name).map(|s| pla_ui::configs::brief(&s)).unwrap_or_default();
                    json!({ "name": m.name, "summary": brief })
                })
                .collect();
            Ok(json!({ "document": d.label, "configs": list }))
        }
        "apply" => {
            let name = a["name"].as_str().ok_or("apply needs name")?;
            let st = store.load(&d.key, name).ok_or_else(|| format!("no configuration {name}"))?;
            let args: Value = serde_json::from_str(&st).map_err(|e| format!("configuration {name} is unreadable: {e}"))?;
            let r = rt().block_on(pla_bridge::query::call_once(d.port, d.hwnd, pla_bridge::shim::api::CFG_RESTORE, &args)).map_err(|e| format!("{e:#}"))?;
            Ok(json!({ "document": d.label, "applied": name, "result": r }))
        }
        "save" => {
            let v = rt().block_on(pla_bridge::query::call_once(d.port, d.hwnd, pla_bridge::shim::api::CFG_SAVE, &Value::Null)).map_err(|e| format!("{e:#}"))?;
            let state = v.to_string();
            let name = match a["name"].as_str() {
                Some(n) if store.load(&d.key, n).is_some() => {
                    store.update(&d.key, n, &state, None)?;
                    n.to_string()
                }
                Some(n) => {
                    let got = store.save(&d.key, &state, None)?;
                    if got != n {
                        store.rename(&d.key, &got, n)?;
                    }
                    n.to_string()
                }
                None => store.save(&d.key, &state, None)?,
            };
            Ok(json!({ "document": d.label, "saved": name, "summary": pla_ui::configs::brief(&state) }))
        }
        x => Err(format!("unknown action {x} (list / apply / save)")),
    }
}

// ---- 尺寸记录 / 参数

fn dims_tool(d: &Doc, a: &Value) -> Result<Value, String> {
    let f = pla_ui::dims::load_file(&d.key);
    let list = |f: &pla_ui::dims::DimsFile| {
        json!({
            "document": d.label,
            "dims": f.dims.iter().map(|r| json!({ "name": r.name, "kind": r.kind, "value": r.value, "units": r.units, "expr": r.expr })).collect::<Vec<_>>(),
            "params": f.params.iter().map(|p| json!({ "name": p.name, "kind": p.kind, "expr": p.expr, "note": p.note })).collect::<Vec<_>>(),
        })
    };
    match a["action"].as_str().unwrap_or("list") {
        "list" => Ok(list(&f)),
        act @ ("set" | "set_param") => {
            if d.bg {
                return Err("changing dimensions runs through PLAhelper (pla-ui) on the foreground document only".into());
            }
            let req = if act == "set" {
                let ch = a["changes"].as_array().filter(|c| !c.is_empty()).ok_or("set needs changes: [{name, value}]")?;
                json!({ "op": "dims_set", "changes": ch })
            } else {
                json!({ "op": "param_set", "name": a["name"], "expr": a["expr"].as_str().map(String::from).or_else(|| a["expr"].as_f64().map(|x| x.to_string())) })
            };
            let r = bridge::call(&req, Duration::from_secs(10))?;
            if r["ok"] != true {
                return Err(r["error"].as_str().unwrap_or("failed").to_string());
            }
            // 等改完(记录里的值到位, 或没有绑定可改)
            let want: Vec<(String, f64)> = a["changes"].as_array().into_iter().flatten().filter_map(|c| Some((c["name"].as_str()?.to_string(), c["value"].as_f64()?))).collect();
            let t0 = std::time::Instant::now();
            let mut last = pla_ui::dims::load_file(&d.key);
            while t0.elapsed() < Duration::from_secs(60) {
                std::thread::sleep(Duration::from_millis(700));
                last = pla_ui::dims::load_file(&d.key);
                let done = if act == "set" {
                    want.iter().all(|(n, v)| last.dims.iter().any(|r| &r.name == n && (r.value - v).abs() < 1e-3))
                } else {
                    t0.elapsed() > Duration::from_secs(5)
                };
                if done {
                    break;
                }
            }
            let mut out = list(&last);
            out["note"] = json!("check PLAhelper's message if a value did not change (a failed step rolls the whole change back)");
            Ok(out)
        }
        x => Err(format!("unknown action {x} (list / set / set_param)")),
    }
}

// ---- 零件库(全局; 用户 2026-11-18: 要能访问和改动)

fn lib_entry_json(e: &pla_ui::parts_lib::Entry, full: bool) -> Value {
    let pref = e.suppliers.get(e.preferred).or_else(|| e.suppliers.first());
    let mut v = json!({
        "pn": e.pn, "name": e.name, "category": e.category, "fields": e.fields,
        "price": pref.map(|s| s.price.clone()), "supplier": pref.map(|s| s.name.clone()), "spec": e.spec,
    });
    if full {
        v["suppliers"] = json!(e.suppliers.iter().map(|s| json!({ "name": s.name, "url": s.url, "price": s.price, "currency": s.currency, "moq": s.moq, "lead": s.lead, "date": s.date })).collect::<Vec<_>>());
        v["preferred"] = json!(e.preferred);
        v["aliases"] = json!(e.aliases);
        v["keys"] = json!(e.keys.len());
        v["notes"] = json!(e.notes);
        v["former"] = json!(e.former);
    }
    v
}

fn lib_supplier(v: &Value, old: Option<&pla_ui::parts_lib::Supplier>) -> pla_ui::parts_lib::Supplier {
    let mut s = old.cloned().unwrap_or_default();
    let get = |k: &str| v[k].as_str().map(String::from).or_else(|| v[k].as_f64().map(|x| x.to_string()));
    if let Some(x) = get("name") { s.name = x; }
    if let Some(x) = get("url") { s.url = x; }
    if let Some(x) = get("price") {
        if x != s.price {
            s.date = pla_ui::parts_lib::today();
        }
        s.price = x;
    }
    if let Some(x) = get("currency") { s.currency = x; }
    if let Some(x) = get("moq") { s.moq = x; }
    if let Some(x) = get("lead") { s.lead = x; }
    s
}

fn strs_of(v: &Value) -> Option<Vec<String>> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
}

fn parts_library_tool(a: &Value) -> Result<Value, String> {
    use pla_ui::parts_lib as pl;
    let dir = pl::dir();
    let mut lib = pl::load_in(&dir);
    let action = a["action"].as_str().unwrap_or("search");
    let need_pn = |lib: &pl::Library| -> Result<String, String> {
        let q = a["pn"].as_str().ok_or("needs pn")?;
        lib.find_any(q).map(|e| e.pn.clone()).ok_or_else(|| format!("no library entry {q}"))
    };
    match action {
        "search" => {
            let q = a["query"].as_str().unwrap_or("").to_lowercase();
            let cat = a["category"].as_str();
            let limit = a["limit"].as_u64().unwrap_or(50) as usize;
            let hits: Vec<Value> = lib
                .entries
                .iter()
                .filter(|e| cat.is_none_or(|c| e.category == c))
                .filter(|e| {
                    q.is_empty()
                        || e.pn.to_lowercase().contains(&q)
                        || e.name.to_lowercase().contains(&q)
                        || e.spec.to_lowercase().contains(&q)
                        || e.aliases.iter().any(|x| x.to_lowercase().contains(&q))
                        || e.former.iter().any(|x| x.to_lowercase().contains(&q))
                        || e.fields.values().any(|x| x.to_lowercase().contains(&q))
                })
                .take(limit)
                .map(|e| lib_entry_json(e, false))
                .collect();
            Ok(json!({ "file": dir.join("library.json").display().to_string(), "count": lib.entries.len(), "hits": hits, "categories": lib.pn_rule.categories, "prefix": lib.pn_rule.prefix }))
        }
        "get" => {
            let pn = need_pn(&lib)?;
            Ok(lib_entry_json(lib.find(&pn).unwrap(), true))
        }
        "add" => {
            let name = a["name"].as_str().ok_or("add needs name")?.to_string();
            let fields: std::collections::BTreeMap<String, String> =
                a["fields"].as_object().map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str().map(String::from).or_else(|| v.as_f64().map(|x| x.to_string()))?))).collect()).unwrap_or_default();
            let g = |k: &str| fields.get(k).cloned().unwrap_or_default();
            let category = a["category"].as_str().map(String::from).unwrap_or_else(|| pl::guess_category(&name, &g("model"), &g("material"), &g("origin")).unwrap_or("009").to_string());
            if !lib.pn_rule.categories.contains_key(&category) {
                return Err(format!("unknown category {category} (have: {})", lib.pn_rule.categories.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(", ")));
            }
            let pn = match a["pn"].as_str() {
                Some(p) if lib.find_any(p).is_some() => return Err(format!("part number {p} is taken")),
                Some(p) => p.to_string(),
                None => lib.next_pn_for(a["project"].as_str().unwrap_or(""), &category),
            };
            let suppliers: Vec<pl::Supplier> = a["suppliers"].as_array().into_iter().flatten().map(|s| lib_supplier(s, None)).collect();
            lib.upsert(pl::Entry {
                pn: pn.clone(), name, category, fields, suppliers, preferred: 0,
                aliases: strs_of(&a["aliases"]).unwrap_or_default(), spec: a["spec"].as_str().unwrap_or("").to_string(),
                notes: a["notes"].as_str().unwrap_or("").to_string(), ..Default::default()
            });
            pl::save_in(&dir, &lib)?;
            Ok(json!({ "added": lib_entry_json(lib.find(&pn).unwrap(), true) }))
        }
        "update" => {
            let mut pn = need_pn(&lib)?;
            // 改分类 = 按新分类重新编号(旧号记进 former)
            if let Some(c) = a["category"].as_str() {
                if !lib.pn_rule.categories.contains_key(c) {
                    return Err(format!("unknown category {c}"));
                }
                if let Some(n) = lib.reclassify(&pn, c) {
                    pn = n;
                }
            }
            let e = lib.find_mut(&pn).unwrap();
            if let Some(n) = a["name"].as_str() {
                e.name = n.into();
            }
            for (k, v) in a["fields"].as_object().into_iter().flatten() {
                match v.as_str().map(String::from).or_else(|| v.as_f64().map(|x| x.to_string())) {
                    Some(x) => {
                        e.fields.insert(k.clone(), x);
                    }
                    None => {
                        e.fields.remove(k);
                    }
                }
            }
            if let Some(s) = a["suppliers"].as_array() {
                e.suppliers = s.iter().map(|x| lib_supplier(x, None)).collect();
                e.preferred = 0;
            }
            // 改 / 加一家: supplier = {name, price, …}(按名字找, 没有就加)
            if a["supplier"].is_object() {
                let nm = a["supplier"]["name"].as_str().unwrap_or("");
                match e.suppliers.iter().position(|s| !nm.is_empty() && s.name == nm) {
                    Some(i) => e.suppliers[i] = lib_supplier(&a["supplier"], Some(&e.suppliers[i])),
                    None => e.suppliers.push(lib_supplier(&a["supplier"], None)),
                }
            }
            if let Some(p) = a["preferred"].as_str() {
                e.preferred = e.suppliers.iter().position(|s| s.name == p).ok_or_else(|| format!("no supplier {p} on {pn}"))?;
            } else if let Some(i) = a["preferred"].as_u64() {
                e.preferred = (i as usize).min(e.suppliers.len().saturating_sub(1));
            }
            if let Some(x) = strs_of(&a["aliases"]) {
                e.aliases = x;
            }
            if let Some(x) = a["spec"].as_str() {
                e.spec = x.into();
            }
            if let Some(x) = a["notes"].as_str() {
                e.notes = x.into();
            }
            pl::save_in(&dir, &lib)?;
            Ok(json!({ "updated": lib_entry_json(lib.find(&pn).unwrap(), true) }))
        }
        "remove" => {
            let pn = need_pn(&lib)?;
            lib.remove(&pn);
            pl::save_in(&dir, &lib)?;
            Ok(json!({ "removed": pn }))
        }
        // BOM 行 ↔ 库: link(pn; pn 为空 = 取消链接)/ from_bom(把这一行存成新条目并链接)/ suggest(库给的建议)
        "link" | "from_bom" | "suggest" => {
            let d = doc_target(a)?;
            let (scan, _) = doc_scan(&d, a["scope"].as_str().filter(|s| !s.is_empty()))?;
            let mut st = pla_ui::doc_jobs::DocState::load(&d.key);
            st.lib = lib;
            let (rows, _) = pla_ui::bom_table::build(&scan, &st.tpl, &st.edits, &st.lib);
            if action == "suggest" {
                let out: Vec<Value> = rows.iter().map(|r| json!({ "row": r.key, "linked": r.link, "suggest": r.suggest })).collect();
                return Ok(json!({ "document": d.label, "rows": out }));
            }
            let q = a["part"].as_str().ok_or("needs part (row key / part number / name)")?;
            let k = find_row(&rows, &st.tpl, q)?;
            let row = rows.iter().find(|r| r.key == k).unwrap();
            let pn = if action == "link" {
                match a["pn"].as_str().filter(|p| !p.is_empty()) {
                    Some(p) => {
                        let pn = st.lib.find_any(p).map(|e| e.pn.clone()).ok_or_else(|| format!("no library entry {p}"))?;
                        st.edits.links.insert(k.clone(), pn.clone());
                        Some(pn)
                    }
                    None => {
                        st.edits.links.remove(&k);
                        None
                    }
                }
            } else {
                let parts = pla_ui::doc_jobs::bom_parts(&scan, &st.tpl, &st.edits, &st.lib, &st.sheets.purchased);
                let model_name = parts.iter().find(|p| p.2.key == k).map(|p| p.1 .1.clone()).unwrap_or_default();
                let pn = pla_ui::doc_jobs::lib_add_from_row(&mut st.lib, &mut st.edits, &st.tpl, row, &model_name);
                pl::save_in(&dir, &st.lib)?;
                Some(pn)
            };
            pla_ui::bom_table::save_edits(&d.key, &st.edits)?;
            let mut out = table_json(&d, &scan, &st);
            out["changed"] = json!([format!("{q} → {}", pn.clone().unwrap_or_else(|| "unlinked".into()))]);
            if let Some(p) = pn.as_ref().and_then(|p| st.lib.find(p)) {
                out["entry"] = lib_entry_json(p, true);
            }
            Ok(out)
        }
        x => Err(format!("unknown action {x} (search / get / add / update / remove / link / from_bom / suggest)")),
    }
}

// ---- BOM 模板(全局; 用户 2026-11-18)

fn val_str(v: &Value) -> Option<String> {
    v.as_str().map(String::from).or_else(|| v.as_f64().map(|x| x.to_string()))
}

/// 一列的 JSON → Col(在 old 上改; null = 去掉该项)。
fn tpl_col(v: &Value, old: Option<&pla_ui::bom_table::Col>) -> Result<pla_ui::bom_table::Col, String> {
    let mut c = old.cloned().unwrap_or(pla_ui::bom_table::Col { id: String::new(), header: String::new(), source: None, formula: None, default: None, sum: false });
    if let Some(x) = v["id"].as_str() {
        c.id = x.trim().to_string();
    }
    if let Some(x) = v["header"].as_str() {
        c.header = x.to_string();
    }
    let opt = |k: &str, cur: &mut Option<String>| {
        if let Some(o) = v.as_object().and_then(|m| m.get(k)) {
            *cur = val_str(o).filter(|s| !s.is_empty());
        }
    };
    opt("source", &mut c.source);
    opt("formula", &mut c.formula);
    opt("default", &mut c.default);
    // 给了公式就是公式列; 改成 source 就不再是公式列
    if v.get("formula").is_some_and(|f| f.is_string()) && v.get("source").is_none() {
        c.source = None;
    }
    if v.get("source").is_some_and(|s| s.is_string()) && v.get("formula").is_none() {
        c.formula = None;
    }
    if let Some(b) = v["sum"].as_bool() {
        c.sum = b;
    }
    if c.header.is_empty() {
        c.header = c.id.clone();
    }
    if c.source.is_none() && c.formula.is_none() {
        c.source = Some("field".into());
    }
    Ok(c)
}

fn tpl_json(name: &str, t: &pla_ui::bom_table::Template) -> Value {
    json!({
        "name": name,
        "columns": t.columns.iter().map(|c| json!({ "id": c.id, "header": c.header, "source": c.source, "formula": c.formula, "default": c.default, "sum": c.sum })).collect::<Vec<_>>(),
    })
}

fn bom_template_tool(a: &Value) -> Result<Value, String> {
    use pla_ui::bom_table as bt;
    let dir = bt::dir();
    let list = bt::list_in(&dir);
    let action = a["action"].as_str().unwrap_or("list");
    let need = |k: &str| -> Result<String, String> {
        let n = a[k].as_str().ok_or_else(|| format!("needs {k}"))?;
        list.iter().find(|x| x.as_str() == n).cloned().ok_or_else(|| format!("no BOM template {n} (have: {})", list.join(", ")))
    };
    let note = "Documents pick a template by name (bom_edit template); a document whose template is gone falls back to the first one.";
    match action {
        "list" => Ok(json!({ "dir": dir.display().to_string(), "templates": list, "builtin": bt::BUILTIN_NAME, "sources": bt::SOURCES })),
        "get" => {
            let n = need("name")?;
            Ok(tpl_json(&n, &bt::load_in(&dir, &n)?))
        }
        "create" => {
            let n = a["name"].as_str().ok_or("needs name")?.trim().to_string();
            if !bt::valid_name(&n) || list.contains(&n) {
                return Err(format!("bad or taken template name {n:?}"));
            }
            let t = match a["from"].as_str() {
                Some(f) if f == "builtin" => bt::builtin(),
                Some(_) => bt::load_in(&dir, &need("from")?)?,
                None => bt::builtin(),
            };
            bt::write_in(&dir, &n, &t)?;
            let first = bt::list_in(&dir).first() == Some(&n);
            Ok(json!({ "created": tpl_json(&n, &t), "note": if first { "This name sorts first, so documents that never picked a template now use it (the default = first by name)." } else { "" } }))
        }
        "rename" => {
            let n = need("name")?;
            let nn = a["new_name"].as_str().ok_or("needs new_name")?.trim().to_string();
            if !bt::valid_name(&nn) || list.contains(&nn) {
                return Err(format!("bad or taken template name {nn:?}"));
            }
            std::fs::rename(bt::path_in(&dir, &n), bt::path_in(&dir, &nn)).map_err(|e| e.to_string())?;
            Ok(json!({ "renamed": format!("{n} → {nn}"), "note": note }))
        }
        "remove" => {
            let n = need("name")?;
            bt::remove_in(&dir, &n)?;
            Ok(json!({ "removed": n, "note": note }))
        }
        "edit" => {
            let n = need("name")?;
            let mut t = bt::load_in(&dir, &n)?;
            let mut log = Vec::new();
            if let Some(cols) = a["columns"].as_array() {
                t.columns = cols.iter().map(|c| tpl_col(c, None)).collect::<Result<_, _>>()?;
                log.push(format!("columns replaced ({})", t.columns.len()));
            }
            for c in a["remove_columns"].as_array().into_iter().flatten().filter_map(|x| x.as_str()) {
                let before = t.columns.len();
                t.columns.retain(|x| x.id != c);
                if t.columns.len() == before {
                    return Err(format!("no column {c}"));
                }
                log.push(format!("removed {c}"));
            }
            for u in a["update_columns"].as_array().into_iter().flatten() {
                let id = u["id"].as_str().ok_or("update_columns items need id")?;
                let i = t.columns.iter().position(|c| c.id == id).ok_or_else(|| format!("no column {id}"))?;
                let mut u2 = u.clone();
                if let Some(o) = u2.as_object_mut() {
                    o.remove("id");
                }
                t.columns[i] = tpl_col(&u2, Some(&t.columns[i]))?;
                log.push(format!("updated {id}"));
            }
            for c in a["add_columns"].as_array().into_iter().flatten() {
                let col = tpl_col(c, None)?;
                let at = match c["after"].as_str() {
                    Some(x) => t.columns.iter().position(|c| c.id == x).map(|i| i + 1).ok_or_else(|| format!("no column {x} (after)"))?,
                    None => t.columns.len(),
                };
                log.push(format!("added {}", col.id));
                t.columns.insert(at, col);
            }
            if let Some(order) = a["order"].as_array() {
                let ids: Vec<&str> = order.iter().filter_map(|x| x.as_str()).collect();
                let mut out = Vec::new();
                for id in &ids {
                    let i = t.columns.iter().position(|c| c.id == *id).ok_or_else(|| format!("no column {id} (order)"))?;
                    out.push(t.columns.remove(i));
                }
                out.append(&mut t.columns); // 没列到的保持原顺序排在后面
                t.columns = out;
                log.push("reordered".into());
            }
            bt::check(&t)?;
            bt::write_in(&dir, &n, &t)?;
            Ok(json!({ "template": tpl_json(&n, &t), "changed": log, "note": "PLAhelper picks the change up within a second; see the result with bom_table." }))
        }
        x => Err(format!("unknown action {x} (list / get / create / rename / remove / edit)")),
    }
}

// ---- 孔规格方案(全局; 用户 2026-11-18)

fn hole_size_json(s: &pla_ui::hole_presets::HoleSize) -> Value {
    serde_json::to_value(s).unwrap_or_default()
}

fn hole_table_tool(a: &Value) -> Result<Value, String> {
    use pla_ui::hole_presets as hp;
    let dir = hp::dir();
    let list = hp::list_in(&dir);
    let active = hp::active_in(&dir);
    let action = a["action"].as_str().unwrap_or("list");
    let pick = |k: &str| -> Result<String, String> {
        match a[k].as_str() {
            None if k == "name" => Ok(active.clone()),
            None => Err(format!("needs {k}")),
            Some(n) => list.iter().find(|x| x.as_str() == n).cloned().ok_or_else(|| format!("no hole table {n} (have: {})", list.join(", "))),
        }
    };
    let bad_name = |n: &str| n.trim().is_empty() || n.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) || list.iter().any(|x| x == n);
    match action {
        "list" => Ok(json!({ "dir": dir.display().to_string(), "tables": list, "active": active, "builtin": hp::BUILTIN_NAME })),
        "get" => {
            let n = pick("name")?;
            Ok(json!({ "name": n, "active": n == active, "sizes": hp::load_in(&dir, &n)?.iter().map(hole_size_json).collect::<Vec<_>>() }))
        }
        "create" => {
            let n = a["name"].as_str().ok_or("needs name")?.trim().to_string();
            if bad_name(&n) {
                return Err(format!("bad or taken hole table name {n:?}"));
            }
            // from: 省略 = 复制当前方案; "builtin" = 内置 ISO; 其它 = 那个方案
            let sizes = match a["from"].as_str() {
                None => hp::load_in(&dir, &active).unwrap_or_else(|_| hp::builtin()),
                Some("builtin") => hp::builtin(),
                Some(_) => hp::load_in(&dir, &pick("from")?)?,
            };
            hp::write_in(&dir, &n, &sizes)?;
            Ok(json!({ "created": n, "sizes": sizes.len() }))
        }
        "rename" => {
            let n = pick("name")?;
            let nn = a["new_name"].as_str().ok_or("needs new_name")?.trim().to_string();
            hp::rename_in(&dir, &n, &nn)?;
            if n == active {
                pla_ui::prefs::save_value("hole_preset", json!(nn))?;
            }
            Ok(json!({ "renamed": format!("{n} → {nn}") }))
        }
        "remove" => {
            let n = pick("name")?;
            if a["name"].is_null() {
                return Err("remove needs an explicit name".into());
            }
            hp::remove_in(&dir, &n)?;
            Ok(json!({ "removed": n, "active": hp::active_in(&dir) }))
        }
        "select" => {
            let n = pick("name")?;
            pla_ui::prefs::save_value("hole_preset", json!(n))?;
            Ok(json!({ "active": n, "note": "PLAhelper's hole panel switches within a second." }))
        }
        "edit" => {
            let n = pick("name")?;
            let mut sizes = hp::load_in(&dir, &n)?;
            let mut log = Vec::new();
            let num = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.trim().parse().ok())));
            for v in a["set"].as_array().into_iter().flatten() {
                let size = v["size"].as_str().ok_or("set items need size")?.trim().to_string();
                let i = sizes.iter().position(|s| s.size.eq_ignore_ascii_case(&size));
                let mut s = match i {
                    Some(i) => sizes[i].clone(),
                    None => {
                        let all = ["clearance", "tap", "cbD", "cbDepth", "cskD"].iter().all(|k| num(v, k).is_some());
                        if !all {
                            return Err(format!("new size {size} needs clearance, tap, cbD, cbDepth and cskD"));
                        }
                        hp::HoleSize { size: size.clone(), clearance: 0.0, tap: 0.0, cb_d: 0.0, cb_depth: 0.0, csk_d: 0.0 }
                    }
                };
                for (k, f) in [("clearance", &mut s.clearance), ("tap", &mut s.tap), ("cbD", &mut s.cb_d), ("cbDepth", &mut s.cb_depth), ("cskD", &mut s.csk_d)] {
                    if let Some(x) = num(v, k) {
                        *f = x;
                    }
                }
                match i {
                    Some(i) => {
                        log.push(format!("updated {size}"));
                        sizes[i] = s;
                    }
                    None => {
                        log.push(format!("added {size}"));
                        sizes.push(s);
                    }
                }
            }
            for r in a["remove_sizes"].as_array().into_iter().flatten().filter_map(|x| x.as_str()) {
                let before = sizes.len();
                sizes.retain(|s| !s.size.eq_ignore_ascii_case(r));
                if sizes.len() == before {
                    return Err(format!("no size {r} in {n}"));
                }
                log.push(format!("removed {r}"));
            }
            // 按公称直径排(M2 < M2.5 < M10), 认不出的放后面
            let dia = |s: &str| s.trim_start_matches(['M', 'm']).split(['x', 'X', '×']).next().and_then(|x| x.parse::<f64>().ok()).unwrap_or(f64::MAX);
            sizes.sort_by(|x, y| dia(&x.size).total_cmp(&dia(&y.size)));
            let warn = hp::check(&sizes)?;
            hp::write_in(&dir, &n, &sizes)?;
            Ok(json!({ "name": n, "changed": log, "warnings": warn, "sizes": sizes.iter().map(hole_size_json).collect::<Vec<_>>() }))
        }
        x => Err(format!("unknown action {x} (list / get / create / rename / remove / select / edit)")),
    }
}

/// model_hole 的 size → 按当前孔规格方案填 d / cbD / cbDepth / cskD / tapD(显式给的数值优先)。
fn hole_from_table(a: &Value) -> Result<Value, String> {
    let Some(size) = a["size"].as_str().map(|s| s.trim().to_string()).or_else(|| a["size"].as_f64().map(|d| format!("M{d}"))) else {
        return Ok(a.clone());
    };
    use pla_ui::hole_presets as hp;
    let dir = hp::dir();
    let name = a["table"].as_str().map(String::from).unwrap_or_else(|| hp::active_in(&dir));
    let sizes = hp::load_in(&dir, &name)?;
    hole_params(a, &size, &name, &sizes)
}

fn hole_params(a: &Value, size: &str, name: &str, sizes: &[pla_ui::hole_presets::HoleSize]) -> Result<Value, String> {
    let s = sizes.iter().find(|s| s.size.eq_ignore_ascii_case(&size)).ok_or_else(|| format!("no size {size} in hole table {name} (have: {})", sizes.iter().map(|s| s.size.as_str()).collect::<Vec<_>>().join(", ")))?;
    let tap = a["usage"].as_str() == Some("tap");
    let mut out = a.clone();
    let mut fill = |k: &str, v: f64| {
        if out.get(k).is_none_or(|x| x.is_null()) {
            out[k] = json!(v);
        }
    };
    let has_stack = a["stack"].as_array().is_some_and(|x| !x.is_empty());
    fill("d", if tap && !has_stack { s.tap } else { s.clearance });
    match a["type"].as_str() {
        Some("counterbore") => {
            fill("cbD", s.cb_d);
            fill("cbDepth", s.cb_depth);
        }
        Some("countersink") => fill("cskD", s.csk_d),
        _ => {}
    }
    if has_stack {
        fill("tapD", s.tap);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hole_size_from_table() {
        let sizes = pla_ui::hole_presets::builtin();
        let p = |a: Value| hole_params(&a, a["size"].as_str().unwrap(), "ISO metric", &sizes);
        let cb = p(json!({"face": "f", "size": "M4", "type": "counterbore"})).unwrap();
        assert_eq!((cb["d"].as_f64(), cb["cbD"].as_f64(), cb["cbDepth"].as_f64()), (Some(4.5), Some(8.0), Some(4.4)));
        assert!(cb.get("cskD").is_none());
        let tap = p(json!({"face": "f", "size": "m4", "usage": "tap"})).unwrap();
        assert_eq!(tap["d"].as_f64(), Some(3.3));
        // 组合孔: d = 过孔, 最后一件 tapD = 底孔
        let st = p(json!({"face": "f", "size": "M5", "usage": "tap", "stack": ["7"]})).unwrap();
        assert_eq!((st["d"].as_f64(), st["tapD"].as_f64()), (Some(5.5), Some(4.2)));
        // 显式数值优先
        let ex = p(json!({"face": "f", "size": "M3", "type": "countersink", "d": 3.2})).unwrap();
        assert_eq!((ex["d"].as_f64(), ex["cskD"].as_f64()), (Some(3.2), Some(6.7)));
        assert!(p(json!({"face": "f", "size": "M7"})).unwrap_err().contains("M7"));
    }
}
