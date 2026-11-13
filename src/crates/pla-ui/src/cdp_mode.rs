//! F7: Plasticity 默认以 CDP 模式运行 — 由 pla-ui 托管(R14)。
//!
//! CDP 模式 = Plasticity 带 `--inspect=127.0.0.1:9334` 启动(需已打 fuse 补丁的 exe)
//! + 9333 代理进程(`tools/pla-cdp/pla-cdp-v8.js proxy`, 把主进程 inspector 转成
//! pla-bridge 认识的 page target)。本模块负责:
//!
//!   1. **启动配置**: 用户级快捷方式(任务栏固定 / 用户开始菜单 / 用户桌面)改为指向
//!      已打补丁的 exe 并带调试参数; `.plasticity` 双击 = 在 HKCU 覆盖同一 ProgID 的
//!      open 命令(不碰 HKLM, 无需管理员)。公用快捷方式(公用桌面 / 全体用户开始菜单)
//!      需管理员, 只报告不改。取消 = 去掉参数、删除我们写的 HKCU 覆盖。
//!   2. **代理托管**: Plasticity 的 9334 在听而 9333 无人监听时, 自动拉起代理
//!      (无窗口, 日志 `%TEMP%\pla-cdp-proxy.log`)。代理独立存活: 重启 pla-ui 不中断,
//!      端口已被占用就不重复拉起。
//!
//! 安全: 调试端口只监听 127.0.0.1(局域网不可达), 但本机任意程序都能借它在 Plasticity
//! 里执行代码 —— 个人电脑可接受, 设置面板有说明。

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub const INSPECT_PORT: u16 = 9334;
pub const PROXY_PORT: u16 = 9333;
pub const INSPECT_ARG: &str = "--inspect=127.0.0.1:9334";

/// Electron fuse 线标记(与 pla-cdp-v8.js 一致): sentinel(32) + 版本(1) + 长度(1) + 线串。
const FUSE_SENTINEL: &[u8] = b"dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX";
/// 线串第 4 位 = NodeCliInspect。
const FUSE_INDEX_INSPECT: usize = 3;

/// 本机端口是否有人监听。
pub fn port_open(port: u16) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(150),
    )
    .is_ok()
}

// ---------------------------------------------------------------- fuse / exe

/// 读 exe 的 Electron fuse: Some(true) = `--inspect` 已放开(打过补丁)。
pub fn fuse_inspect_enabled(exe: &Path) -> Option<bool> {
    use std::io::Read;
    let mut f = std::fs::File::open(exe).ok()?;
    let mut buf = vec![0u8; 4 << 20];
    let mut hay: Vec<u8> = Vec::new();
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        hay.extend_from_slice(&buf[..n]);
        if let Some(at) = find_bytes(&hay, FUSE_SENTINEL) {
            let mut tail = hay[at + FUSE_SENTINEL.len()..].to_vec();
            while tail.len() < 2 + 64 {
                let m = f.read(&mut buf).ok()?;
                if m == 0 {
                    break;
                }
                tail.extend_from_slice(&buf[..m]);
            }
            let wire_len = *tail.get(1)? as usize;
            let wire = tail.get(2..2 + wire_len)?;
            return Some(wire.get(FUSE_INDEX_INSPECT) == Some(&b'1'));
        }
        // 只保留可能跨块的尾巴
        let keep = FUSE_SENTINEL.len() - 1;
        let cut = hay.len().saturating_sub(keep);
        hay.drain(..cut);
    }
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    let first = needle[0];
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        match hay[i..hay.len() - needle.len() + 1].iter().position(|&b| b == first) {
            Some(p) => {
                let s = i + p;
                if &hay[s..s + needle.len()] == needle {
                    return Some(s);
                }
                i = s + 1;
            }
            None => return None,
        }
    }
    None
}

/// `app-26.2.3` → [26, 2, 3](用于挑最新版本)。
fn version_key(exe: &Path) -> Vec<u32> {
    exe.parent()
        .and_then(|d| d.file_name())
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("app-"))
        .map(|v| v.split('.').filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_default()
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from)
}

/// 找已打 fuse 补丁的 Plasticity.exe: 现有快捷方式目标优先, 其次各安装位置的 app-*,
/// 同为已打补丁时取版本最高者。
pub fn find_patched_exe() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    for lnk in user_shortcuts() {
        if let Ok(info) = read_lnk(&lnk) {
            cands.push(info.target);
        }
    }
    let roots = [
        env_path("USERPROFILE").map(|h| h.join("Programs")),
        env_path("LOCALAPPDATA").map(|l| l.join("Programs").join("Plasticity")),
        env_path("ProgramFiles").map(|p| p.join("Plasticity")),
    ];
    for root in roots.into_iter().flatten() {
        if let Ok(rd) = std::fs::read_dir(&root) {
            for e in rd.flatten() {
                let p = e.path();
                if p.file_name().and_then(|n| n.to_str()).map_or(false, |n| n.starts_with("app-")) {
                    cands.push(p.join("Plasticity.exe"));
                }
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    cands.retain(|p| p.is_file() && seen.insert(p.to_string_lossy().to_lowercase()));
    cands.sort_by(|a, b| version_key(b).cmp(&version_key(a)));
    cands.into_iter().find(|p| fuse_inspect_enabled(p) == Some(true))
}

// ---------------------------------------------------------------- 代理托管

/// 代理脚本: pla-ui.exe 同级或其祖先目录下的 `tools/pla-cdp/pla-cdp-v8.js`。
pub fn find_proxy_script() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent().map(Path::to_path_buf);
    for _ in 0..6 {
        let d = dir?;
        for rel in ["pla-cdp-v8.js", "tools/pla-cdp/pla-cdp-v8.js"] {
            let p = d.join(rel);
            if p.is_file() {
                return Some(p);
            }
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    None
}

/// 拉起代理(无窗口, 追加写日志)。
fn spawn_proxy() -> Result<Child, String> {
    let script = find_proxy_script().ok_or("未找到 tools/pla-cdp/pla-cdp-v8.js")?;
    let log = std::env::temp_dir().join("pla-cdp-proxy.log");
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .map_err(|e| format!("打开代理日志失败: {e}"))?;
    let err = out.try_clone().map_err(|e| format!("打开代理日志失败: {e}"))?;
    let mut cmd = Command::new("node");
    cmd.arg(&script)
        .arg("proxy")
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err);
    if let Some(dir) = script.parent() {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .map_err(|e| format!("启动 CDP 代理失败(需要 node 在 PATH 中): {e}"))
}

/// 代理守护: 每 3s 看一次端口, 需要时拉起代理。
pub struct ProxyManager {
    child: Option<Child>,
    last_tick: Option<Instant>,
    last_spawn: Option<Instant>,
    /// 给设置面板显示的状态。
    pub status: String,
}

impl Default for ProxyManager {
    fn default() -> Self {
        Self { child: None, last_tick: None, last_spawn: None, status: "检测中…".into() }
    }
}

impl ProxyManager {
    pub fn tick(&mut self) {
        if self.last_tick.map_or(false, |t| t.elapsed() < Duration::from_secs(3)) {
            return;
        }
        self.last_tick = Some(Instant::now());
        if let Some(c) = &mut self.child {
            if let Ok(Some(st)) = c.try_wait() {
                self.child = None;
                self.status = format!("代理已退出({st}), 日志见 %TEMP%\\pla-cdp-proxy.log");
            }
        }
        if port_open(PROXY_PORT) {
            self.status = if self.child.is_some() {
                "代理运行中(由 pla-ui 启动)".into()
            } else {
                "代理运行中".into()
            };
            return;
        }
        if !port_open(INSPECT_PORT) {
            self.status = "Plasticity 未以 CDP 模式运行".into();
            return;
        }
        // 刚拉起的代理需要时间监听端口, 15s 内不重复拉起。
        if self.last_spawn.map_or(false, |t| t.elapsed() < Duration::from_secs(15)) {
            return;
        }
        self.last_spawn = Some(Instant::now());
        match spawn_proxy() {
            Ok(c) => {
                self.child = Some(c);
                self.status = "已自动启动 CDP 代理…".into();
            }
            Err(e) => self.status = e,
        }
    }
}

// ---------------------------------------------------------------- 启动配置

/// 用户级 Plasticity 快捷方式(存在的才返回)。
pub fn user_shortcuts() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(app) = env_path("APPDATA") {
        v.push(app.join(r"Microsoft\Windows\Start Menu\Programs\Plasticity.lnk"));
        v.push(app.join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar\Plasticity.lnk"));
    }
    if let Some(home) = env_path("USERPROFILE") {
        v.push(home.join(r"Desktop\Plasticity.lnk"));
    }
    v.retain(|p| p.is_file());
    v
}

/// 公用快捷方式(改需管理员, 只报告)。
pub fn public_shortcuts() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(p) = env_path("PUBLIC") {
        v.push(p.join(r"Desktop\Plasticity.lnk"));
    }
    if let Some(p) = env_path("ProgramData") {
        v.push(p.join(r"Microsoft\Windows\Start Menu\Programs\Plasticity\Plasticity.lnk"));
    }
    v.retain(|p| p.is_file());
    v
}

fn shortcut_label(p: &Path) -> &'static str {
    let s = p.to_string_lossy().to_lowercase();
    if s.contains("taskbar") {
        "任务栏"
    } else if s.contains("start menu") {
        "开始菜单"
    } else {
        "桌面"
    }
}

/// 参数里加/去调试开关(其他参数原样保留)。
pub fn with_inspect_arg(args: &str, enable: bool) -> String {
    let rest: Vec<&str> = args
        .split_whitespace()
        .filter(|a| !a.starts_with("--inspect"))
        .collect();
    let mut out: Vec<&str> = Vec::new();
    if enable {
        out.push(INSPECT_ARG);
    }
    out.extend(rest);
    out.join(" ")
}

/// 是否已配置为 CDP 模式启动(任一用户快捷方式带调试参数)。
pub fn cdp_startup_enabled() -> bool {
    user_shortcuts()
        .iter()
        .filter_map(|p| read_lnk(p).ok())
        .any(|i| i.args.contains("--inspect"))
}

/// 启用/停用 CDP 模式启动。返回逐项结果(给设置面板显示)。
pub fn set_cdp_startup(enable: bool) -> Result<Vec<String>, String> {
    let mut report = Vec::new();
    let exe = if enable {
        Some(find_patched_exe().ok_or(
            "未找到已打补丁的 Plasticity.exe(先运行 node tools/pla-cdp/pla-cdp-v8.js fuse)",
        )?)
    } else {
        None
    };
    let lnks = user_shortcuts();
    if lnks.is_empty() {
        report.push("没有用户级 Plasticity 快捷方式".into());
    }
    for lnk in &lnks {
        let label = shortcut_label(lnk);
        let res = read_lnk(lnk).and_then(|info| {
            let args = with_inspect_arg(&info.args, enable);
            write_lnk(lnk, exe.as_deref(), &args)
        });
        report.push(match res {
            Ok(()) => format!("{label}快捷方式: 已{}", if enable { "改为 CDP 模式" } else { "恢复" }),
            Err(e) => format!("{label}快捷方式: 失败 {e}"),
        });
    }
    report.push(match set_file_association(exe.as_deref()) {
        Ok(()) => format!(".plasticity 双击打开: 已{}", if enable { "改为 CDP 模式" } else { "恢复" }),
        Err(e) => format!(".plasticity 双击打开: 失败 {e}"),
    });
    if enable && !public_shortcuts().is_empty() {
        report.push("公用桌面/全体开始菜单快捷方式需管理员权限, 未改动(仍为普通模式)".into());
    }
    Ok(report)
}

// ---------------------------------------------------------------- Win32: .lnk / 注册表

pub struct LnkInfo {
    pub target: PathBuf,
    pub args: String,
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn shell_link() -> Result<
    (
        windows::Win32::UI::Shell::IShellLinkW,
        windows::Win32::System::Com::IPersistFile,
    ),
    String,
> {
    use windows::core::{ComInterface, GUID};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::IShellLinkW;
    const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let sl: IShellLinkW = CoCreateInstance(&CLSID_SHELL_LINK, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("ShellLink: {e}"))?;
        let pf: IPersistFile = sl.cast().map_err(|e| format!("IPersistFile: {e}"))?;
        Ok((sl, pf))
    }
}

#[cfg(windows)]
pub fn read_lnk(path: &Path) -> Result<LnkInfo, String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Com::STGM_READ;
    let (sl, pf) = shell_link()?;
    unsafe {
        let p = wide(&path.to_string_lossy());
        pf.Load(PCWSTR(p.as_ptr()), STGM_READ).map_err(|e| format!("读取 {}: {e}", path.display()))?;
        let mut t = [0u16; 1024];
        sl.GetPath(&mut t, std::ptr::null_mut(), 0).map_err(|e| format!("GetPath: {e}"))?;
        let mut a = [0u16; 2048];
        sl.GetArguments(&mut a).map_err(|e| format!("GetArguments: {e}"))?;
        let s = |b: &[u16]| String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]);
        Ok(LnkInfo { target: PathBuf::from(s(&t)), args: s(&a) })
    }
}

#[cfg(windows)]
fn write_lnk(path: &Path, target: Option<&Path>, args: &str) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Com::STGM_READWRITE;
    let (sl, pf) = shell_link()?;
    unsafe {
        let p = wide(&path.to_string_lossy());
        pf.Load(PCWSTR(p.as_ptr()), STGM_READWRITE).map_err(|e| format!("打开: {e}"))?;
        if let Some(t) = target {
            let tw = wide(&t.to_string_lossy());
            sl.SetPath(PCWSTR(tw.as_ptr())).map_err(|e| format!("SetPath: {e}"))?;
            if let Some(dir) = t.parent() {
                let dw = wide(&dir.to_string_lossy());
                let _ = sl.SetWorkingDirectory(PCWSTR(dw.as_ptr()));
            }
        }
        let aw = wide(args);
        sl.SetArguments(PCWSTR(aw.as_ptr())).map_err(|e| format!("SetArguments: {e}"))?;
        pf.Save(PCWSTR::null(), true).map_err(|e| format!("保存: {e}"))?;
    }
    Ok(())
}

/// `.plasticity` 的 ProgID(HKCR 合并视图)。
#[cfg(windows)]
fn plasticity_progid() -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CLASSES_ROOT, RRF_RT_REG_SZ};
    let mut buf = [0u16; 256];
    let mut size = (buf.len() * 2) as u32;
    let sub = wide(".plasticity");
    unsafe {
        RegGetValueW(
            HKEY_CLASSES_ROOT,
            PCWSTR(sub.as_ptr()),
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
        .ok()?;
    }
    let s = String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)]);
    (!s.is_empty()).then_some(s)
}

/// 标记: 该 HKCU ProgID 键是 pla-ui 新建的(停用时整键删除)。
#[cfg(windows)]
const PLA_MARKER: &str = "PlaHelperCreated";
/// 启用前 HKCU 里原有的 open 命令(Plasticity 安装器会写一份用户级关联), 停用时写回。
#[cfg(windows)]
const PLA_PREV: &str = "PlaHelperPrevCommand";

/// 读 HKCU 下某键的字符串值(name=None 为默认值)。
#[cfg(windows)]
fn hkcu_get_sz(sub: &str, name: Option<&str>) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let mut buf = vec![0u16; 2048];
    let mut size = (buf.len() * 2) as u32;
    let s = wide(sub);
    let n = name.map(wide);
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(s.as_ptr()),
            n.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
        .ok()?;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// exe = Some → 写 HKCU 覆盖(带调试参数); None → 删除我们写的覆盖。
#[cfg(windows)]
fn set_file_association(exe: Option<&Path>) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegDeleteTreeW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_WRITE, REG_CREATED_NEW_KEY, REG_CREATE_KEY_DISPOSITION,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };
    use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
    let progid = plasticity_progid().ok_or("系统里没有 .plasticity 文件关联")?;
    let base = format!(r"Software\Classes\{progid}");
    let cmd_path = format!(r"{base}\shell\open\command");
    let prev_cmd = hkcu_get_sz(&cmd_path, None);
    let set_sz = |key: HKEY, name: Option<&str>, val: &str| -> Result<(), String> {
        let v = wide(val);
        let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), v.len() * 2) };
        let n = name.map(wide);
        unsafe {
            RegSetValueExW(
                key,
                n.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
                0,
                REG_SZ,
                Some(bytes),
            )
        }
        .map_err(|e| format!("写注册表: {e}"))
    };
    unsafe {
        match exe {
            Some(exe) => {
                // ProgID 根键: 记录是否由我们新建
                let mut root = HKEY::default();
                let mut disp = REG_CREATE_KEY_DISPOSITION::default();
                let bw = wide(&base);
                RegCreateKeyExW(
                    HKEY_CURRENT_USER, PCWSTR(bw.as_ptr()), 0, PCWSTR::null(),
                    REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut root, Some(&mut disp),
                )
                .map_err(|e| format!("建注册表键: {e}"))?;
                let mut r = if disp == REG_CREATED_NEW_KEY { set_sz(root, Some(PLA_MARKER), "1") } else { Ok(()) };
                // 备份原有命令(只备份一次, 且不备份我们自己写的带调试参数的版本)
                if r.is_ok() && hkcu_get_sz(&base, Some(PLA_PREV)).is_none() {
                    if let Some(prev) = prev_cmd.as_deref().filter(|p| !p.contains("--inspect")) {
                        r = set_sz(root, Some(PLA_PREV), prev);
                    }
                }
                let _ = RegCloseKey(root);
                r?;
                let mut cmd = HKEY::default();
                let cw = wide(&cmd_path);
                RegCreateKeyExW(
                    HKEY_CURRENT_USER, PCWSTR(cw.as_ptr()), 0, PCWSTR::null(),
                    REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut cmd, None,
                )
                .map_err(|e| format!("建注册表键: {e}"))?;
                let r = set_sz(cmd, None, &format!("\"{}\" {INSPECT_ARG} \"%1\"", exe.display()));
                let _ = RegCloseKey(cmd);
                r?;
            }
            None => {
                let bw = wide(&base);
                if hkcu_get_sz(&base, Some(PLA_MARKER)).is_some() {
                    // 整个 ProgID 键是我们建的 → 整键删除
                    let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(bw.as_ptr()));
                } else if let Some(prev) = hkcu_get_sz(&base, Some(PLA_PREV)) {
                    // 写回启用前的原命令(Plasticity 安装器写的用户级关联)
                    let mut cmd = HKEY::default();
                    let cw = wide(&cmd_path);
                    RegCreateKeyExW(
                        HKEY_CURRENT_USER, PCWSTR(cw.as_ptr()), 0, PCWSTR::null(),
                        REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut cmd, None,
                    )
                    .map_err(|e| format!("打开注册表键: {e}"))?;
                    let r = set_sz(cmd, None, &prev);
                    let _ = RegCloseKey(cmd);
                    r?;
                    let pw = wide(PLA_PREV);
                    let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(bw.as_ptr()), PCWSTR(pw.as_ptr()));
                } else if prev_cmd.as_deref().map_or(false, |c| c.contains("--inspect")) {
                    // 无备份(启用前就没有用户级命令) → 只删我们写的 command 键
                    let cw = wide(&cmd_path);
                    let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(cw.as_ptr()));
                    let _ = windows::Win32::System::Registry::RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(cw.as_ptr()));
                }
            }
        }
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn read_lnk(_path: &Path) -> Result<LnkInfo, String> {
    Err("仅 Windows".into())
}
#[cfg(not(windows))]
fn write_lnk(_path: &Path, _target: Option<&Path>, _args: &str) -> Result<(), String> {
    Err("仅 Windows".into())
}
#[cfg(not(windows))]
fn set_file_association(_exe: Option<&Path>) -> Result<(), String> {
    Err("仅 Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspect_arg_add_remove_keeps_other_args() {
        assert_eq!(with_inspect_arg("", true), INSPECT_ARG);
        assert_eq!(with_inspect_arg("--foo bar", true), format!("{INSPECT_ARG} --foo bar"));
        assert_eq!(with_inspect_arg(&format!("{INSPECT_ARG} --foo"), true), format!("{INSPECT_ARG} --foo"));
        assert_eq!(with_inspect_arg(&format!("{INSPECT_ARG} --foo"), false), "--foo");
        assert_eq!(with_inspect_arg("--inspect=127.0.0.1:1234", false), "");
    }

    #[test]
    fn finds_sentinel_across_chunk_boundary() {
        let mut data = vec![b'x'; 100];
        data.extend_from_slice(FUSE_SENTINEL);
        assert_eq!(find_bytes(&data, FUSE_SENTINEL), Some(100));
        assert_eq!(find_bytes(b"short", FUSE_SENTINEL), None);
    }

    #[test]
    fn reads_fuse_wire_from_file() {
        let dir = std::env::temp_dir().join("pla-fuse-test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("fake.exe");
        let mut data = vec![0u8; 5 << 20]; // 跨 4MB 块
        let at = (4 << 20) - 10;
        data[at..at + FUSE_SENTINEL.len()].copy_from_slice(FUSE_SENTINEL);
        let w = at + FUSE_SENTINEL.len();
        data[w] = 1; // 版本
        data[w + 1] = 9; // 线长
        data[w + 2..w + 11].copy_from_slice(b"000111011");
        std::fs::write(&p, &data).unwrap();
        assert_eq!(fuse_inspect_enabled(&p), Some(true));
        data[w + 2 + FUSE_INDEX_INSPECT] = b'0';
        std::fs::write(&p, &data).unwrap();
        assert_eq!(fuse_inspect_enabled(&p), Some(false));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn version_key_orders_app_dirs() {
        let a = Path::new(r"C:\x\app-26.2.10\Plasticity.exe");
        let b = Path::new(r"C:\x\app-26.2.3\Plasticity.exe");
        assert!(version_key(a) > version_key(b));
    }
}
