//! 内存基准测试台(调研 16)。
//!
//! 构建 N 个控件的典型混合界面(行 = view 容器 + checkbox + 两个 text + button,
//! 每行 5 节点,行内含响应式绑定),可选渲染若干离屏帧,然后打点并驻留,
//! 供外部(PowerShell)采样进程 WorkingSet/Private。
//!
//! 用法:membench --controls 3000 [--backend cpu|vello] [--frames N 计时帧] [--no-render] [--hold-secs 6]
//! 帧率口径:预热 1 帧后连续渲染 N 帧取均值(vello 离屏含纹理回读,略高估帧成本)
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

    let backend = args
        .iter()
        .position(|a| a == "--backend")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "cpu".into());

    // 预热帧(含字体解析/管线编译)与计时帧分开:帧率只看稳态
    let mut warmup_ms = 0u128;
    let mut frame_ms = 0.0f64;
    if !no_render {
        match backend.as_str() {
            "cpu" => {
                let t = Instant::now();
                let _ = sv_shell::render_frame(&doc, 1920, 1080, 1.0);
                warmup_ms = t.elapsed().as_millis();
                let t = Instant::now();
                for _ in 0..frames {
                    let _ = sv_shell::render_frame(&doc, 1920, 1080, 1.0);
                }
                frame_ms = t.elapsed().as_secs_f64() * 1000.0 / frames.max(1) as f64;
            }
            #[cfg(feature = "backend-vello")]
            "vello" => {
                let t = Instant::now();
                let ok = sv_shell::render_frame_vello(&doc, 1920, 1080, 1.0).is_some();
                if !ok {
                    println!("BACKEND-UNAVAILABLE vello");
                    return;
                }
                warmup_ms = t.elapsed().as_millis();
                let t = Instant::now();
                for _ in 0..frames {
                    let _ = sv_shell::render_frame_vello(&doc, 1920, 1080, 1.0);
                }
                frame_ms = t.elapsed().as_secs_f64() * 1000.0 / frames.max(1) as f64;
            }
            other => {
                println!("BACKEND-UNAVAILABLE {other}");
                return;
            }
        }
    }

    let nodes = doc.read(|inner| inner.nodes.len());
    let signals = sv_reactive::debug_node_count();
    println!(
        "READY backend={backend} nodes={nodes} signals={signals} build_ms={} warmup_ms={warmup_ms} frame_ms={frame_ms:.1} fps={:.0} frames={}",
        built.as_millis(),
        if frame_ms > 0.0 { 1000.0 / frame_ms } else { 0.0 },
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
