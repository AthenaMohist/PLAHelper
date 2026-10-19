//! 手动验证: 抓取真实 Plasticity 主窗口 → PNG 落盘(供 Python 解码对拍)。
//! cargo run -p pla-ui --release --example cap_test
fn main() {
    let wins = pla_ui::window_finder::find_plasticity_windows();
    if wins.is_empty() {
        println!("NO_PLASTICITY_WINDOW");
        return;
    }
    let w = &wins[0];
    println!("target: {:?} {}x{}", w.title, w.right - w.left, w.bottom - w.top);
    let out = std::path::Path::new("pla_cap_test.png");
    match pla_ui::wincap::capture_window_png(w.hwnd, out) {
        Ok(_) => println!("OK {}", out.display()),
        Err(e) => println!("ERR {e}"),
    }
}
