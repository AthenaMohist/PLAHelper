//! pla-app: Tauri 装配 bin crate（M1-a 脚手架，M2 扩展 snapshot commands）。
//!
//! tauri 2 依赖过重，`cargo check` 在本环境不可行，故 tauri 装配置于
//! feature gate `tauri`（默认关闭，原因见 Cargo.toml 注释与 commit message）。
//! 默认构建为纯 bin，打印装配日志以验证 workspace 接线。
//!
//! M2 快照命令（架构 §5 契约）：snapshot_create / snapshot_list /
//! snapshot_delete / snapshot_restore / snapshot_pin / cleanup_now，
//! 事件 `pla://snapshot-created`。命令核心逻辑放在 `SnapshotCommands`
//! trait 后的 `AppState` 方法（无 tauri 依赖，可单测）；tauri 构建下由
//! `#[tauri::command]` 薄包装 + window emit 转发事件（feature gate 内，
//! 本环境不可编译，已按 M1-a 先例标注）。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use pla_selection::{ApplyReport, EntityCache, Selection, SelectionStore};
use pla_snapshot::{CleanupPolicy, SnapshotMeta, SnapshotStore};

/// 快照存储根目录。可用环境变量 `PLA_DATA_DIR` 覆盖（测试/多实例隔离用）。
// [交付化 2026-10-17] 配置解析顺序: 命令行 --data/--bridge > 环境变量 > 内置缺省。
// 缺省 bridge= ws://127.0.0.1:8980（Plasticity Server 官方端口）；缺省 data= exe 同级 data\（绿色单文件，免环境变量）。
static CLI_DATA: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
static CLI_BRIDGE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// 从 args 提取 --data/--bridge（在 main 最早期调用）。
fn apply_cli_config(args: &[String]) {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--data" {
            if let Some(v) = it.next() { let _ = CLI_DATA.set(PathBuf::from(v)); }
        } else if a == "--bridge" {
            if let Some(v) = it.next() { let _ = CLI_BRIDGE.set(v.clone()); }
        }
    }
}

pub fn data_dir() -> PathBuf {
    if let Some(d) = CLI_DATA.get() { return d.clone(); }
    std::env::var_os("PLA_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // exe 同级 data 目录（绿色交付：双击即用，无需任何环境变量）
            std::env::current_exe().ok()
                .and_then(|e| e.parent().map(|p| p.join("data")))
                .unwrap_or_else(|| PathBuf::from("data"))
        })
}

/// 共享应用状态：快照存储 + 清理策略 + M3/M4 选择集与尺寸存储 + 实体树缓存。
pub struct AppState {
    pub store: SnapshotStore,
    pub cleanup_policy: Mutex<CleanupPolicy>,
    pub selections: SelectionStore,
    pub dims: pla_dim::DimStore,
    pub cache: EntityCache,
}

impl AppState {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        let base = base.into();
        Self {
            store: SnapshotStore::new(&base),
            cleanup_policy: Mutex::new(CleanupPolicy::default()),
            selections: SelectionStore::new(&base),
            dims: pla_dim::DimStore::new(&base),
            cache: EntityCache::new(),
        }
    }

    /// 手动快照。失败映射为结构化错误字符串（tauri invoke 序列化口径）。
    pub fn cmd_snapshot_create(
        &self,
        doc_key: &str,
        source: &Path,
        label: &str,
    ) -> Result<SnapshotMeta, String> {
        self.store
            .snapshot_create(source, doc_key, label, 0)
            .map_err(|e| e.to_string())
    }

    pub fn cmd_snapshot_list(&self, doc_key: &str) -> Vec<SnapshotMeta> {
        self.store.snapshot_list(doc_key)
    }

    pub fn cmd_snapshot_delete(&self, doc_key: &str, snapshot_id: &str) -> Result<(), String> {
        self.store
            .snapshot_delete(doc_key, snapshot_id)
            .map_err(|e| e.to_string())
    }

    /// 恢复 = 系统默认应用打开快照副本（新窗口，不关闭当前窗口）。
    pub fn cmd_snapshot_restore(&self, doc_key: &str, snapshot_id: &str) -> Result<(), String> {
        self.store
            .snapshot_restore(doc_key, snapshot_id)
            .map_err(|e| e.to_string())
    }

    pub fn cmd_snapshot_pin(
        &self,
        doc_key: &str,
        snapshot_id: &str,
        pinned: bool,
    ) -> Result<SnapshotMeta, String> {
        self.store
            .snapshot_pin(doc_key, snapshot_id, pinned)
            .map_err(|e| e.to_string())
    }

    /// 全库清理（前端"立即清理"；定时清理任务复用同一策略入口）。
    pub fn cmd_cleanup_now(&self) -> Result<Vec<(String, Vec<String>)>, String> {
        let policy = self.cleanup_policy.lock().unwrap().clone();
        let base = self.store.base().to_path_buf();
        let map = pla_snapshot::cleanup_all(&base, &policy);
        Ok(map.into_iter().collect())
    }

    // ---- M3/M4 commands（架构 §5 契约） ------------------------------------

    /// doc_list：列出选择集库中已出现过的文档（doc_key=filename，D5）。
    pub fn cmd_doc_list(&self) -> Vec<String> {
        self.selections.list_docs()
    }

    /// selection_save：命名保存选择集（弱引用成员）。
    pub fn cmd_selection_save(
        &self,
        doc_key: &str,
        name: &str,
        members: Vec<String>,
    ) -> Result<Selection, String> {
        self.selections
            .save(doc_key, name, members)
            .map_err(|e| e.to_string())
    }

    /// selection_list：列出某文档的全部选择集。
    pub fn cmd_selection_list(&self, doc_key: &str) -> Vec<Selection> {
        self.selections.list(doc_key)
    }

    /// selection_delete。
    pub fn cmd_selection_delete(&self, doc_key: &str, name: &str) -> Result<(), String> {
        self.selections.delete(doc_key, name).map_err(|e| e.to_string())
    }

    /// dim_define：选择集 + 方向向量 + 数值定义尺寸。
    pub fn cmd_dim_define(
        &self,
        doc_key: &str,
        name: &str,
        set_name: &str,
        dir: [f64; 3],
        value: f64,
    ) -> Result<pla_dim::DimDef, String> {
        self.dims
            .define(doc_key, name, set_name, pla_dim::DimDir::new(dir[0], dir[1], dir[2]), value)
            .map_err(|e| e.to_string())
    }

    /// dim_list。
    pub fn cmd_dim_list(&self, doc_key: &str) -> Vec<pla_dim::DimDef> {
        self.dims.list(doc_key)
    }
}

/// bridge WS 端点。可用 `PLA_BRIDGE_WS` 覆盖（缺省本机 mock/实机端口）。
pub fn bridge_url() -> String {
    if let Some(b) = CLI_BRIDGE.get() { return b.clone(); }
    std::env::var("PLA_BRIDGE_WS").unwrap_or_else(|_| "ws://127.0.0.1:8980".to_string())
}

/// selection_apply：经实体树缓存拉取 LIST_ALL，存在性校验后 PUT_SOME 注入
/// 同名组；缺失成员返回 ApplyReport（UI 标黄，不静默丢弃，D3）。
/// 异步（走 bridge），与同步 AppState 方法分开以免阻塞。
pub async fn selection_apply(
    state: &AppState,
    client: &pla_bridge::BridgeClient,
    doc_key: &str,
    name: &str,
) -> Result<ApplyReport, String> {
    let tree = state
        .cache
        .fetch(client, doc_key)
        .await
        .map_err(|e| e.to_string())?;
    state
        .selections
        .apply(client, doc_key, &tree, name)
        .await
        .map_err(|e| e.to_string())
}

/// dim_update：拉成员 facet 顶点平移 + PUT_SOME 替换写回（§3.3/D4）。
pub async fn dim_update(
    state: &AppState,
    client: &pla_bridge::BridgeClient,
    doc_key: &str,
    name: &str,
    new_value: f64,
) -> Result<pla_dim::DimReport, String> {
    pla_dim::dim_update(client, &state.selections, &state.dims, doc_key, name, new_value)
        .await
        .map_err(|e| e.to_string())
}

#[cfg_attr(feature = "tauri", tauri::command)]
#[cfg_attr(not(test), allow(dead_code))] // 默认(非 tauri)构建中仅测试使用
fn ping() -> String {
    "pong".to_string()
}

fn main() {
    tracing_subscriber::fmt::init();

    // [CLI 薄壳 2026-10-16, Frank C2 授权] 非 tauri 构建的验收入口。
    // 用法: pla-app <command> [args...]；环境变量 PLA_BRIDGE_WS 覆盖 bridge 端点(缺省 ws://127.0.0.1:8787)，PLA_DATA_DIR 覆盖数据目录。
    // 命令集与 tauri command 一一对应，专供 Windows 真机三点验收(快照/选择集/尺寸)。
    #[cfg(feature = "tauri")]
    {
        run_tauri();
        return;
    }
    #[allow(unreachable_code)]
    {
    let args: Vec<String> = std::env::args().skip(1).collect();
    apply_cli_config(&args);
    if args.is_empty() || args[0] == "--help" || args[0] == "help" {
        println!("pla-app CLI (验收薄壳)");
        println!("配置(优先级): --bridge/--data 参数 > 环境变量 PLA_BRIDGE_WS/PLA_DATA_DIR > 内置缺省");
        println!("缺省: bridge=ws://127.0.0.1:8980  data=<exe目录>\\data  —— 零配置即可用");
        println!("可选参数: --bridge ws://127.0.0.1:8980  --data D:\\pla_data");
        println!("命令:");
        println!("  snapshot-create <doc_key> <file> <label>       快照(读 Plasticity 文件副本)");
        println!("  snapshot-list <doc_key>");
        println!("  snapshot-restore <doc_key> <snapshot_id>       恢复=默认应用新窗口打开副本");
        println!("  snapshot-delete <doc_key> <snapshot_id>");
        println!("  selection-save <doc_key> <name> <逗号分隔成员>");
        println!("  selection-list <doc_key>");
        println!("  selection-apply <doc_key> <name>               走 bridge: LIST_ALL+PUT_SOME");
        println!("  dim-define <doc_key> <name> <set_name> <dx dy dz> <value>");
        println!("  dim-list <doc_key>");
        println!("  dim-update <doc_key> <name> <new_value>        走 bridge: 顶点平移写回");
        println!("  entities <doc_key>                            列出 bridge 实体(id/kind/name)");
        println!("  doc-list / ping");
        return;
    }
    let cmd = args[0].as_str();
    let a = &args[1..];
    let state = AppState::new(data_dir());
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let need = |n: usize| {
        if a.len() < n {
            eprintln!("参数不足: 需要 {} 个, 见 --help", n);
            std::process::exit(2);
        }
    };
    match cmd {
        "ping" => println!("pong"),
        "entities" => { need(1);
            let client = rt.block_on(async { pla_bridge::BridgeClient::connect(&bridge_url()) })
                .map_err(|e| e.to_string()).unwrap_or_else(|e| { eprintln!("BRIDGE ERROR: {e}"); std::process::exit(3); });
            match rt.block_on(state.cache.fetch(&client, &a[0])) {
                Ok(tree) => {
                    let mut out: Vec<serde_json::Value> = Vec::new();
                    fn walk(n: &pla_selection::EntityNode, out: &mut Vec<serde_json::Value>) {
                        out.push(serde_json::json!({"id": n.id, "kind": n.kind, "name": n.name}));
                        for c in &n.children { walk(c, out); }
                    }
                    for n in &tree.entities { walk(n, &mut out); }
                    println!("{}", serde_json::to_string_pretty(&out).unwrap());
                }
                Err(e) => { eprintln!("ERROR: {e}"); std::process::exit(1); }
            }
        },
        "doc-list" => println!("{}", serde_json::to_string_pretty(&state.cmd_doc_list()).unwrap()),
        "snapshot-create" => { need(3);
            match state.cmd_snapshot_create(&a[0], Path::new(&a[1]), &a[2]) {
                Ok(m) => println!("{}", serde_json::to_string_pretty(&m).unwrap()),
                Err(e) => { eprintln!("ERROR: {e}"); std::process::exit(1); }
            }
        }
        "snapshot-list" => { need(1);
            println!("{}", serde_json::to_string_pretty(&state.cmd_snapshot_list(&a[0])).unwrap());
        }
        "snapshot-restore" => { need(2);
            if let Err(e) = state.cmd_snapshot_restore(&a[0], &a[1]) { eprintln!("ERROR: {e}"); std::process::exit(1); }
            println!("OK");
        }
        "snapshot-delete" => { need(2);
            if let Err(e) = state.cmd_snapshot_delete(&a[0], &a[1]) { eprintln!("ERROR: {e}"); std::process::exit(1); }
            println!("OK");
        }
        "selection-save" => { need(3);
            let members: Vec<String> = a[2].split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            match state.cmd_selection_save(&a[0], &a[1], members) {
                Ok(s) => println!("{}", serde_json::to_string_pretty(&s).unwrap()),
                Err(e) => { eprintln!("ERROR: {e}"); std::process::exit(1); }
            }
        }
        "selection-list" => { need(1);
            println!("{}", serde_json::to_string_pretty(&state.cmd_selection_list(&a[0])).unwrap());
        }
        "selection-apply" => { need(2);
            let client = rt.block_on(async { pla_bridge::BridgeClient::connect(&bridge_url()) })
                .map_err(|e| e.to_string()).unwrap_or_else(|e| { eprintln!("BRIDGE ERROR: {e}"); std::process::exit(3); });
            match rt.block_on(selection_apply(&state, &client, &a[0], &a[1])) {
                Ok(r) => println!("{}", serde_json::to_string_pretty(&r).unwrap()),
                Err(e) => { eprintln!("ERROR: {e}"); std::process::exit(1); }
            }
        }
        "dim-define" => { need(7);
            let dir = [a[3].parse::<f64>().unwrap_or(0.0), a[4].parse::<f64>().unwrap_or(0.0), a[5].parse::<f64>().unwrap_or(0.0)];
            let value = a[6].parse::<f64>().unwrap_or_else(|_| { eprintln!("value 不是数字"); std::process::exit(2); });
            match state.cmd_dim_define(&a[0], &a[1], &a[2], dir, value) {
                Ok(d) => println!("{}", serde_json::to_string_pretty(&d).unwrap()),
                Err(e) => { eprintln!("ERROR: {e}"); std::process::exit(1); }
            }
        }
        "dim-list" => { need(1);
            println!("{}", serde_json::to_string_pretty(&state.cmd_dim_list(&a[0])).unwrap());
        }
        "dim-update" => { need(3);
            let nv = a[2].parse::<f64>().unwrap_or_else(|_| { eprintln!("new_value 不是数字"); std::process::exit(2); });
            let client = rt.block_on(async { pla_bridge::BridgeClient::connect(&bridge_url()) })
                .map_err(|e| e.to_string()).unwrap_or_else(|e| { eprintln!("BRIDGE ERROR: {e}"); std::process::exit(3); });
            match rt.block_on(dim_update(&state, &client, &a[0], &a[1], nv)) {
                Ok(r) => println!("{}", serde_json::to_string_pretty(&r).unwrap()),
                Err(e) => { eprintln!("ERROR: {e}"); std::process::exit(1); }
            }
        }
        other => { eprintln!("未知命令: {other}（--help 看列表）"); std::process::exit(2); }
    }
    }
}

#[cfg_attr(not(feature = "tauri"), allow(dead_code))]
fn run_tauri() {
    #[cfg(feature = "tauri")]
    {
        // tauri 装配（M2）：state 注入 + 快照命令注册 + 事件 pla://snapshot-created。
        // 事件转发：snapshot_create 成功后 `app.emit("pla://snapshot-created",
        // json!({"doc_key": ..., "meta": ...}))`（架构 §5）；tauri crate 本地
        // 不可编译，此路径按 M1-a 先例以注释固化契约，启用 feature 后补 emit 调用。
        tauri::Builder::default()
            .manage(AppState::new(data_dir()))
            .invoke_handler(tauri::generate_handler![
                ping,
                snapshot_create,
                snapshot_list,
                snapshot_delete,
                snapshot_restore,
                snapshot_pin,
                cleanup_now,
                doc_list,
                entities_fetch,
                selection_save,
                selection_apply,
                dim_define,
                dim_update
            ])
            .run(tauri::generate_context!())
            .expect("error while running tauri application");
    }

    #[cfg(feature = "tauri")]
    #[tauri::command]
    fn snapshot_create(
        state: tauri::State<AppState>,
        doc_key: String,
        source_plasticity: String,
        label: String,
    ) -> Result<SnapshotMeta, String> {
        state.cmd_snapshot_create(&doc_key, Path::new(&source_plasticity), &label)
        // TODO(tauri build env): app.emit("pla://snapshot-created", {...})
    }

    #[cfg(feature = "tauri")]
    #[tauri::command]
    fn snapshot_list(state: tauri::State<AppState>, doc_key: String) -> Vec<SnapshotMeta> {
        state.cmd_snapshot_list(&doc_key)
    }

    #[cfg(feature = "tauri")]
    #[tauri::command]
    fn snapshot_delete(
        state: tauri::State<AppState>,
        doc_key: String,
        snapshot_id: String,
    ) -> Result<(), String> {
        state.cmd_snapshot_delete(&doc_key, &snapshot_id)
    }

    #[cfg(feature = "tauri")]
    #[tauri::command]
    fn snapshot_restore(
        state: tauri::State<AppState>,
        doc_key: String,
        snapshot_id: String,
    ) -> Result<(), String> {
        state.cmd_snapshot_restore(&doc_key, &snapshot_id)
    }

    #[cfg(feature = "tauri")]
    #[tauri::command]
    fn snapshot_pin(
        state: tauri::State<AppState>,
        doc_key: String,
        snapshot_id: String,
        pinned: bool,
    ) -> Result<SnapshotMeta, String> {
        state.cmd_snapshot_pin(&doc_key, &snapshot_id, pinned)
    }

    #[cfg(feature = "tauri")]
    #[tauri::command]
    fn cleanup_now(state: tauri::State<AppState>) -> Result<Vec<(String, Vec<String>)>, String> {
        state.cmd_cleanup_now()
    }

    // ---- M3/M4 commands（架构 §5；feature gate 内，M2 先例） ----------------
    // 异步命令 selection_apply / dim_update 需 tauri async runtime；此环境
    // 不可编译，按 M1-a 先例以注释固化契约，启用 feature 后补 await+emit：
    //   selection_apply 成功后 app.emit("pla://doc-updated", {"doc_key", "kind": "selection"})
    //   dim_update 成功后 app.emit("pla://doc-updated", {"doc_key", "kind": "dim"})

    #[cfg(feature = "tauri")]
    #[tauri::command]
    fn doc_list(state: tauri::State<AppState>) -> Vec<String> {
        state.cmd_doc_list()
    }

    #[cfg(feature = "tauri")]
    #[tauri::command]
    fn entities_fetch(
        state: tauri::State<AppState>,
        app: tauri::AppHandle,
        doc_key: String,
    ) -> Result<pla_selection::EntityTree, String> {
        let client = rt.block_on(async { pla_bridge::BridgeClient::connect(&bridge_url()) })
            .map_err(|e| e.to_string())?;
        let tree = tokio::runtime::Handle::current().block_on(state.cache.fetch(&client, &doc_key))
            .map_err(|e| e.to_string())?;
        // TODO(tauri build env): app.emit("pla://doc-updated", json!({"doc_key": doc_key, "kind": "entities"}))
        let _ = app;
        Ok(tree)
    }

    #[cfg(feature = "tauri")]
    #[tauri::command]
    fn selection_save(
        state: tauri::State<AppState>,
        doc_key: String,
        name: String,
        members: Vec<String>,
    ) -> Result<pla_selection::Selection, String> {
        state.cmd_selection_save(&doc_key, &name, members)
    }

    #[cfg(feature = "tauri")]
    #[tauri::command]
    fn selection_apply(
        state: tauri::State<AppState>,
        doc_key: String,
        name: String,
    ) -> Result<pla_selection::ApplyReport, String> {
        let client = pla_bridge::BridgeClient::connect(&bridge_url()).map_err(|e| e.to_string())?;
        tokio::runtime::Handle::current()
            .block_on(super::selection_apply(&state, &client, &doc_key, &name))
    }

    #[cfg(feature = "tauri")]
    #[tauri::command]
    fn dim_define(
        state: tauri::State<AppState>,
        doc_key: String,
        name: String,
        set_name: String,
        dir: [f64; 3],
        value: f64,
    ) -> Result<pla_dim::DimDef, String> {
        state.cmd_dim_define(&doc_key, &name, &set_name, dir, value)
    }

    #[cfg(feature = "tauri")]
    #[tauri::command]
    fn dim_update(
        state: tauri::State<AppState>,
        doc_key: String,
        name: String,
        new_value: f64,
    ) -> Result<pla_dim::DimReport, String> {
        let client = pla_bridge::BridgeClient::connect(&bridge_url()).map_err(|e| e.to_string())?;
        tokio::runtime::Handle::current()
            .block_on(super::dim_update(&state, &client, &doc_key, &name, new_value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_returns_pong() {
        assert_eq!(super::ping(), "pong");
    }

    #[test]
    fn snapshot_command_loop_create_list_pin_delete() {
        let dir = std::env::temp_dir().join(format!(
            "pla-app-cmd-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let state = AppState::new(dir.clone());
        let src = dir.join("src.plasticity");
        std::fs::write(&src, "entities: []").unwrap();

        let meta = state
            .cmd_snapshot_create("doc.plasticity", &src, "t1")
            .unwrap();
        assert_eq!(state.cmd_snapshot_list("doc.plasticity").len(), 1);

        let pinned = state
            .cmd_snapshot_pin("doc.plasticity", &meta.id, true)
            .unwrap();
        assert!(pinned.pinned);

        state
            .cmd_snapshot_delete("doc.plasticity", &meta.id)
            .unwrap();
        assert!(state.cmd_snapshot_list("doc.plasticity").is_empty());
        assert!(state.cmd_snapshot_delete("doc.plasticity", &meta.id).is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn restore_missing_snapshot_is_structured_error() {
        let dir = std::env::temp_dir().join(format!(
            "pla-app-restore-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = AppState::new(&dir);
        assert!(state.cmd_snapshot_restore("none.plasticity", "x").is_err());
    }

    #[test]
    fn cleanup_now_runs_with_default_policy() {
        let dir = std::env::temp_dir().join(format!(
            "pla-app-clean-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let state = AppState::new(dir.clone());
        let src = dir.join("src.plasticity");
        std::fs::write(&src, "x").unwrap();
        state
            .cmd_snapshot_create("d.plasticity", &src, "a")
            .unwrap();
        let swept = state.cmd_cleanup_now().unwrap();
        assert!(swept.is_empty(), "fresh snapshot survives default policy");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn data_dir_env_override() {
        // 用子进程无关的方式：直接改 env 再读回（单线程测试进程内安全）。
        std::env::set_var("PLA_DATA_DIR", "/tmp/pla-custom");
        assert_eq!(data_dir(), PathBuf::from("/tmp/pla-custom"));
        std::env::remove_var("PLA_DATA_DIR");
    }

    #[test]
    fn m34_commands_selection_and_dim_crud() {
        let dir = std::env::temp_dir().join(format!(
            "pla-app-m34-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let state = AppState::new(dir.clone());

        // selection_save + list + doc_list (D5 isolation)
        let sel = state
            .cmd_selection_save("doc.plasticity", "arms", vec!["1".into(), "2".into()])
            .unwrap();
        assert_eq!(sel.members.len(), 2);
        assert_eq!(state.cmd_selection_list("doc.plasticity").len(), 1);
        assert_eq!(state.cmd_doc_list(), vec!["doc.plasticity".to_string()]);

        // dim_define + list
        let def = state
            .cmd_dim_define("doc.plasticity", "len", "arms", [0.0, 3.0, 4.0], 10.0)
            .unwrap();
        assert_eq!(def.value, 10.0);
        assert_eq!(state.cmd_dim_list("doc.plasticity").len(), 1);

        // selection_delete cleans up; doc disappears from doc_list
        state.cmd_selection_delete("doc.plasticity", "arms").unwrap();
        assert!(state.cmd_doc_list().is_empty());
        assert!(state.cmd_selection_delete("doc.plasticity", "arms").is_err());

        // invalid names surface as structured errors
        assert!(state
            .cmd_selection_save("doc.plasticity", "///", vec![])
            .is_err());
        assert!(state.cmd_dim_define("", "x", "s", [1.0, 0.0, 0.0], 1.0).is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// End-to-end over the mock bridge: selection_apply injects a group via
    /// one PUT_SOME; missing members yield a yellow-mark report and no write.
    /// Same mock-server pattern as crates/pla-selection/tests/mock_ws.rs.
    mod mock_ws_e2e {
        use super::super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::Duration;

        use futures_util::{SinkExt, StreamExt};
        use serde_json::{json, Value};
        use tokio::net::{TcpListener, TcpStream};
        use tokio_tungstenite::tungstenite::Message;
        use tokio_tungstenite::{accept_async, WebSocketStream};

        struct BridgeUrlGuard;
        impl Drop for BridgeUrlGuard {
            fn drop(&mut self) {
                std::env::remove_var("PLA_BRIDGE_WS");
            }
        }

        async fn spawn_mock() -> (String, BridgeUrlGuard, Arc<AtomicUsize>) {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}", listener.local_addr().unwrap());
            std::env::set_var("PLA_BRIDGE_WS", &url);
            let counter = Arc::new(AtomicUsize::new(0));
            tokio::spawn(mock_server(listener, Arc::clone(&counter)));
            (url, BridgeUrlGuard, counter)
        }

        async fn mock_server(listener: TcpListener, put_some_count: Arc<AtomicUsize>) {
            loop {
                let Ok((sock, _)) = listener.accept().await else { return };
                let value = Arc::clone(&put_some_count);
                tokio::spawn(async move {
                    let ws: WebSocketStream<TcpStream> = match accept_async(sock).await {
                        Ok(ws) => ws,
                        Err(_) => return,
                    };
                    let (mut sink, mut stream) = ws.split();
                    // [FIX 2026-10-18 OC 直修] 二进制协议 mock(原 JSON mock 对不上二进制客户端 → E2E timeout)。
                    // facets: 客户端 G4 解码仅到 id/kind/name(raw_len), facet 顶点解码为已知缺口(真机 G4 复跑覆盖)。
                    let hs_resp: Vec<u8> = vec![
                        0x64,0,0,0, 0,0,0,0, 0x0e,0,0,0, 0,0,0,0, 1,0,0,0, 2,0,0,0, 3,0,0,0,
                        0x0a,0,0,0, 0x0b,0,0,0, 0x14,0,0,0, 0x16,0,0,0, 0x17,0,0,0, 0x18,0,0,0,
                        0x19,0,0,0, 0x1a,0,0,0, 0x1f,0,0,0, 0x64,0,0,0,
                    ];
                    fn obj_record(id: u32, kind: u32, name: &str) -> Vec<u8> {
                // [真机布局 DPL-218 逆向]: flags|kind_raw|type_code|index|0|0|parent|tag6|len|name+pad
                let kind_raw = 4u32;                                   // 实体 kind_raw=4(Extruded 实锤)
                let type_code = match kind { 2 => 2u32, _ => 5 };      // 5=solid, 2=sheet
                let mut v = Vec::new();
                v.extend_from_slice(&1u32.to_le_bytes());
                v.extend_from_slice(&kind_raw.to_le_bytes());
                v.extend_from_slice(&type_code.to_le_bytes());
                v.extend_from_slice(&id.to_le_bytes());               // index = 对外 id
                v.extend_from_slice(&0u32.to_le_bytes());
                v.extend_from_slice(&0u32.to_le_bytes());
                v.extend_from_slice(&0xffffffffu32.to_le_bytes());    // parent=root
                v.extend_from_slice(&6u32.to_le_bytes());             // tag=6 name
                v.extend_from_slice(&(name.len() as u32).to_le_bytes());
                v.extend_from_slice(name.as_bytes());
                let pad = (4 - (name.len() % 4)) % 4;
                v.extend_from_slice(&vec![0u8; pad]);
                // facets 段(mock 语义, 供 dim 顶点平移测试; 真机块布局待探针):
                v.extend_from_slice(&1u32.to_le_bytes());
                v.extend_from_slice(&2u32.to_le_bytes());
                for pt in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0]] {
                    for c in pt { v.extend_from_slice(&c.to_le_bytes()); }
                }
                v
            }
                    fn list_all_resp(mid: u32) -> Vec<u8> {
                        let mut v = 20u32.to_le_bytes().to_vec();
                        v.extend_from_slice(&mid.to_le_bytes());
                        v.extend_from_slice(&200u32.to_le_bytes());
                        let fname = b"doc.plasticity";
                        v.extend_from_slice(&(fname.len() as u32).to_le_bytes());
                        v.extend_from_slice(fname);
                // 4 字节对齐 padding(protocol::Reader::string 读后对齐, 真机帧 17B+3pad 实锤)
                let pad = (4 - (fname.len() % 4)) % 4;
                v.extend_from_slice(&vec![0u8; pad]);
                        v.extend_from_slice(&0u32.to_le_bytes());
                        let recs = vec![obj_record(1, 1, "Body"), obj_record(2, 1, "Arm")];
                        v.extend_from_slice(&(recs.len() as u32).to_le_bytes());
                        for r in recs { v.extend_from_slice(&(r.len() as u32).to_le_bytes()); v.extend_from_slice(&r); }
                        v
                    }
                    while let Some(Ok(msg)) = stream.next().await {
                        let Message::Binary(b) = msg else { continue };
                        if b.len() < 8 { continue; }
                        let mt = u32::from_le_bytes([b[0],b[1],b[2],b[3]]);
                        let mid = u32::from_le_bytes([b[4],b[5],b[6],b[7]]);
                        let out = match mt {
                            100 => { let mut r = hs_resp.clone(); r[4..8].copy_from_slice(&mid.to_le_bytes()); r }
                            23 => continue,
                            20 => list_all_resp(mid),
                            31 => { value.fetch_add(1, Ordering::SeqCst); let mut v = 31u32.to_le_bytes().to_vec(); v.extend_from_slice(&mid.to_le_bytes()); v.extend_from_slice(&200u32.to_le_bytes()); v.extend_from_slice(&0u32.to_le_bytes()); v }
                            other => { let mut v = other.to_le_bytes().to_vec(); v.extend_from_slice(&mid.to_le_bytes()); v.extend_from_slice(&200u32.to_le_bytes()); v.extend_from_slice(&0u32.to_le_bytes()); v }
                        };
                        if sink.send(Message::Binary(out)).await.is_err() { break; }
                    }
                });
            }
        }

        #[tokio::test]
        async fn selection_apply_and_dim_update_over_bridge() {
            let (_url, _guard, counter) = spawn_mock().await;
            let dir = std::env::temp_dir().join(format!(
                "pla-app-e2e-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let state = AppState::new(dir.clone());

            state
                .cmd_selection_save("doc.plasticity", "arms", vec!["1".into(), "2".into()])
                .unwrap();
            state
                .cmd_dim_define("doc.plasticity", "len", "arms", [0.0, 0.0, 1.0], 5.0)
                .unwrap();

            let client = pla_bridge::BridgeClient::connect(&bridge_url()).unwrap();
            client.wait_alive(Duration::from_secs(2)).await.unwrap();

            let report = selection_apply(&state, &client, "doc.plasticity", "arms")
                .await
                .unwrap();
            assert!(report.ok && report.missing.is_empty());
            assert_eq!(counter.load(Ordering::SeqCst), 1);

            let dim = dim_update(&state, &client, "doc.plasticity", "len", 8.0)
                .await
                .unwrap();
            assert!(dim.ok && dim.moved == 2, "both members translated");
            assert_eq!(
                state.dims.load("doc.plasticity", "len").unwrap().value,
                8.0,
                "dim value persisted after update"
            );

            // Stale selection: missing surfaced, no write (D3).
            state
                .cmd_selection_save("doc.plasticity", "stale", vec!["gone".into()])
                .unwrap();
            // dim_update 的 PUT_SOME 替换写回已计入 counter；stale apply 必须
            // 不再额外发起 PUT_SOME → 比较前后计数而非写死绝对值。
            let before_stale = counter.load(Ordering::SeqCst);
            let report = selection_apply(&state, &client, "doc.plasticity", "stale")
                .await
                .unwrap();
            assert!(!report.ok && report.missing == vec!["gone".to_string()]);
            assert_eq!(
                counter.load(Ordering::SeqCst),
                before_stale,
                "no extra PUT_SOME"
            );

            client.shutdown();
            std::fs::remove_dir_all(&dir).ok();
        }
    }
}
