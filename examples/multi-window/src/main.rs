//! 多窗体示例:**两个窗口共享一个计数器**。任一窗点 +1,两窗同步刷新。
//!
//! 运行(需显示器,开两个窗):`cargo run -p multi-window`
//! 离屏验证一帧:`cargo run -p multi-window -- --png out.png`(渲染"窗 A")
//!
//! 卖点:两窗共享同一**线程内**响应式运行时——一处写、处处精准更新,零窗口间
//! 通信/序列化代码。核心已由 sv-shell 的 `shared_signal_drives_multiple_docs`
//! 测试离屏证明;本例把它搬到真窗口上(`run_multi`)。

use sv_macro::view;
use sv_reactive::{Signal, state};
use sv_ui::{Color, Direction, Doc, Style, ViewId};

/// 一个窗口的 UI:标题 + 共享计数 + 加减按钮。`count` 是**跨窗共享**的 signal。
fn pane(doc: &Doc, root: ViewId, title: &str, accent: Color, count: Signal<i32>) {
    doc.update_style(root, |s| {
        s.padding = 24.0.into();
        s.gap = 12.0;
    });
    let btn = move |s: &mut Style| {
        s.padding = 10.0.into();
        s.corner_radius = 8.0;
        s.bg = Some(accent);
        s.fg = Some(Color::WHITE);
    };
    let title = title.to_string();
    view! { doc, root =>
        <text style(|s| s.font_size = 24.0)>{title.clone()}</text>
        <text style(|s| s.font_size = 20.0)>"共享 Count: " {count.get()}</text>
        <view style(|s| { s.direction = Direction::Row; s.gap = 8.0; })>
            <button style(btn) on_click(move || count.update(|c| *c -= 1))>"-1"</button>
            <button style(btn) on_click(move || count.update(|c| *c += 1))>"+1"</button>
        </view>
        <text style(|s| s.fg = Some(Color::rgb(120, 120, 130)))>"另一个窗会同步变化"</text>
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // 跨窗共享的计数器:两个 build 闭包都捕获它(Signal 是 Copy)
    let count = state(0i32);

    if let Some(i) = args.iter().position(|a| a == "--png") {
        let path = args
            .get(i + 1)
            .cloned()
            .unwrap_or_else(|| "multi-window.png".into());
        // 离屏只渲一个 Doc(窗口化的双窗联动需显示器):渲"窗 A"一帧
        sv_shell::render_to_png(
            move |doc, root| pane(doc, root, "窗 A", Color::rgb(255, 62, 0), count),
            960,
            800,
            2.0,
            &path,
        )
        .expect("离屏渲染失败");
        println!("已渲染到 {path}");
        return;
    }

    sv_shell::run_multi(vec![
        (
            "窗 A".into(),
            Box::new(move |doc: &Doc, root: ViewId| {
                pane(doc, root, "窗 A", Color::rgb(255, 62, 0), count)
            }),
        ),
        (
            "窗 B".into(),
            Box::new(move |doc: &Doc, root: ViewId| {
                pane(doc, root, "窗 B", Color::rgb(60, 120, 255), count)
            }),
        ),
    ])
    .expect("运行失败");
}
