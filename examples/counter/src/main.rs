//! 计数器示例(proc-macro 路线:`view!` 内嵌模板)。
//!
//! 运行:`cargo run -p counter`
//! 离屏验证:`cargo run -p counter -- --png out.png`

use sv_macro::view;
use sv_reactive::state;
use sv_ui::{Color, Direction, Doc, Style, ViewId};

fn build(doc: &Doc, root: ViewId) {
    let count = state(0i32);

    doc.update_style(root, |s| {
        s.padding = 24.0;
        s.gap = 12.0;
    });

    let btn = |s: &mut Style| {
        s.padding = 10.0;
        s.corner_radius = 8.0;
        s.bg = Some(Color::rgb(255, 62, 0));
        s.fg = Some(Color::WHITE);
    };

    view! { doc, root =>
        <text style(|s| s.font_size = 28.0)>"sv 计数器(view! 宏)"</text>
        <text style(|s| s.font_size = 20.0)>"Count: " {count.get()}</text>
        <view style(|s| { s.direction = Direction::Row; s.gap = 8.0; })>
            <button style(btn) on_click(move || count.update(|c| *c -= 1))>"-1"</button>
            <button style(btn) on_click(move || count.update(|c| *c += 1))>"+1"</button>
            <button style(btn) on_click(move || count.set(0))>"归零"</button>
        </view>
        if count.get() > 5 {
            <text style(|s| s.fg = Some(Color::rgb(255, 62, 0)))>"超过 5 了!"</text>
        } else if count.get() < 0 {
            <text style(|s| s.fg = Some(Color::rgb(60, 120, 255)))>"负数啦"</text>
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--png") {
        let path = args.get(i + 1).cloned().unwrap_or_else(|| "counter.png".into());
        sv_shell::render_to_png(build, 960, 800, 2.0, &path).expect("离屏渲染失败");
        println!("已渲染到 {path}");
        return;
    }
    sv_shell::run_app("sv 计数器", build).expect("运行失败");
}
