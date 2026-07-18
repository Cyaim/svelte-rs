//! 内存基准测试台(调研 16)。
//!
//! 构建 N 个控件的典型混合界面(行 = view 容器 + checkbox + 两个 text + button,
//! 每行 5 节点,行内含响应式绑定),可选渲染若干离屏帧,然后打点并驻留,
//! 供外部(PowerShell)采样进程 WorkingSet/Private。
//!
//! 用法:membench --controls 3000 [--frames 3] [--no-render] [--hold-secs 6]
//! 输出:`READY nodes=<场景树节点数> signals=<响应式节点数>` 后驻留。

use std::time::Instant;

use sv_reactive::{create_root, state};
use sv_ui::{Color, Direction, Doc, bind_text};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |name: &str, default: usize| -> usize {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let controls = get("--controls", 3000);
    let frames = get("--frames", 3);
    let hold = get("--hold-secs", 6);
    let no_render = args.iter().any(|a| a == "--no-render");

    let rows = controls / 5; // 每行 5 个控件
    let doc = Doc::new();
    let d = doc.clone();
    let t0 = Instant::now();
    let (_, _scope) = create_root(move || build(&d, rows));
    let built = t0.elapsed();

    let t1 = Instant::now();
    if !no_render {
        for _ in 0..frames {
            let (_pixmap, _placed) = sv_shell::render_frame(&doc, 1920, 1080, 1.0);
        }
    }
    let rendered = t1.elapsed();

    let nodes = doc.read(|inner| inner.nodes.len());
    let signals = sv_reactive::debug_node_count();
    println!(
        "READY nodes={nodes} signals={signals} build_ms={} render_ms={} frames={}",
        built.as_millis(),
        rendered.as_millis(),
        if no_render { 0 } else { frames }
    );
    // 驻留供外部采样
    std::thread::sleep(std::time::Duration::from_secs(hold as u64));
}

/// 典型混合行:标题文本(响应式)+ 静态文本 + 按钮(带点击)+ 复选框,外包一个行容器
fn build(doc: &Doc, rows: usize) {
    let root = doc.root();
    doc.update_style(root, |s| {
        s.padding = 8.0.into();
        s.gap = 2.0;
    });
    for i in 0..rows {
        let row = doc.create_view();
        doc.append(root, row);
        doc.update_style(row, |s| {
            s.direction = Direction::Row;
            s.gap = 4.0;
        });

        let count = state(i as i32);

        let cb = doc.create_checkbox();
        doc.append(row, cb);

        let label = doc.create_text("");
        doc.append(row, label);
        bind_text(doc, label, move || format!("条目 {}", count.get()));

        let tag = doc.create_text("静态标签");
        doc.append(row, tag);
        doc.update_style(tag, |s| s.fg = Some(Color::rgb(120, 120, 136)));

        let btn = doc.create_button("操作");
        doc.append(row, btn);
        doc.update_style(btn, |s| {
            s.padding = 4.0.into();
            s.bg = Some(Color::rgb(255, 62, 0));
        });
        doc.set_on_click(btn, move || count.update(|c| *c += 1));
    }
}
