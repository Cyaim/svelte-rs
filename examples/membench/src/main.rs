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
    let mutate = args.iter().any(|a| a == "--mutate");
    let virtual_mode = args.iter().any(|a| a == "--virtual");
    let windowed = args.iter().any(|a| a == "--windowed");

    // 窗口模式:真实呈现路径(配 SV_SHOW_FPS=1 / SV_RENDERER 用);
    // --mutate 时用自续任务链驱动增量更新(每次后台完成 → 改一行 → 再排队)
    if windowed {
        let rows_w = controls / 5;
        sv_shell::run_app("membench 窗口", move |doc, _root| {
            let sigs = if virtual_mode {
                let off = build_virtual(doc, rows_w);
                if mutate {
                    fn chain_scroll(off: sv_reactive::Signal<usize>) {
                        sv_ui::tasks::spawn(
                            async { std::thread::sleep(std::time::Duration::from_millis(3)) },
                            move |_| {
                                off.update(|o| *o += 1);
                                chain_scroll(off);
                            },
                        );
                    }
                    chain_scroll(off);
                }
                Vec::new()
            } else {
                build(doc, rows_w)
            };
            if mutate && !sigs.is_empty() {
                fn chain(sigs: std::rc::Rc<Vec<sv_reactive::Signal<i32>>>, i: usize) {
                    sv_ui::tasks::spawn(
                        async { std::thread::sleep(std::time::Duration::from_millis(3)) },
                        move |_| {
                            sigs[i % sigs.len()].update(|v| *v += 1);
                            chain(sigs.clone(), i + 1);
                        },
                    );
                }
                chain(std::rc::Rc::new(sigs), 0);
            }
        })
        .expect("窗口模式失败");
        return;
    }

    let rows = controls / 5; // 每行 5 个控件
    let doc = Doc::new();
    let d = doc.clone();
    let t0 = Instant::now();
    let (driver, _scope) = create_root(move || {
        if virtual_mode {
            Driver::Scroll(build_virtual(&d, rows))
        } else {
            Driver::Rows(build(&d, rows))
        }
    });
    let built = t0.elapsed();

    let backend = args
        .iter()
        .position(|a| a == "--backend")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "cpu".into());

    // 预热帧(含字体解析/管线编译)与计时帧分开:帧率只看稳态
    let mut warmup_ms = 0u128;
    let mut samples: Vec<f64> = Vec::with_capacity(frames);
    if !no_render {
        match backend.as_str() {
            "cpu" => {
                let t = Instant::now();
                let _ = sv_shell::render_frame(&doc, 1920, 1080, 1.0);
                warmup_ms = t.elapsed().as_millis();
                for i in 0..frames {
                    if mutate {
                        driver.mutate(i);
                    }
                    let t = Instant::now();
                    let _ = sv_shell::render_frame(&doc, 1920, 1080, 1.0);
                    samples.push(t.elapsed().as_secs_f64() * 1000.0);
                }
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
                for i in 0..frames {
                    if mutate {
                        driver.mutate(i);
                    }
                    let t = Instant::now();
                    let _ = sv_shell::render_frame_vello(&doc, 1920, 1080, 1.0);
                    samples.push(t.elapsed().as_secs_f64() * 1000.0);
                }
            }
            other => {
                println!("BACKEND-UNAVAILABLE {other}");
                return;
            }
        }
    }

    let nodes = doc.read(|inner| inner.nodes.len());
    let signals = sv_reactive::debug_node_count();
    // 帧统计:均值 / p99 / 1% low(最差 1% 帧的均值换算 fps,144Hz 目标的验收口径)
    let (avg, p99, low1) = if samples.is_empty() {
        (0.0, 0.0, 0.0)
    } else {
        let mut sorted = samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let avg = sorted.iter().sum::<f64>() / sorted.len() as f64;
        let p99 = sorted[((sorted.len() as f64 * 0.99) as usize).min(sorted.len() - 1)];
        let worst = &sorted[sorted.len() - sorted.len().div_ceil(100)..];
        let low1 = 1000.0 / (worst.iter().sum::<f64>() / worst.len() as f64);
        (avg, p99, low1)
    };
    println!(
        "READY backend={backend} mutate={mutate} virtual={virtual_mode} nodes={nodes} signals={signals} build_ms={} warmup_ms={warmup_ms} frame_avg_ms={avg:.2} p99_ms={p99:.2} low1_fps={low1:.0} fps={:.0} frames={}",
        built.as_millis(),
        if avg > 0.0 { 1000.0 / avg } else { 0.0 },
        if no_render { 0 } else { frames }
    );
    // 驻留供外部采样
    std::thread::sleep(std::time::Duration::from_secs(hold as u64));
}

/// 突变驱动:普通模式改行信号;虚拟模式推滚动位(全视口重填,最坏情况)
enum Driver {
    Rows(Vec<sv_reactive::Signal<i32>>),
    Scroll(sv_reactive::Signal<usize>),
}

impl Driver {
    fn mutate(&self, i: usize) {
        match self {
            Driver::Rows(sigs) if !sigs.is_empty() => {
                sigs[i % sigs.len()].update(|v| *v += 1);
            }
            Driver::Scroll(offset) => offset.update(|o| *o += 1),
            _ => {}
        }
    }
}

/// 虚拟列表工况:逻辑 rows 行(每行 5 控件),视口只实例化 30 行。
/// 返回滚动位信号(--mutate 每帧 +1 = 连续滚动,虚拟化的最坏工况)
fn build_virtual(doc: &Doc, total_rows: usize) -> sv_reactive::Signal<usize> {
    let offset = state(0usize);
    doc.update_style(doc.root(), |s| {
        s.padding = 8.0.into();
        s.gap = 2.0;
    });
    sv_ui::virtual_list(
        doc,
        doc.root(),
        move || total_rows,
        offset,
        30,
        |i| i,
        |doc, parent, slot, _| {
            let row = doc.create_view();
            doc.append(parent, row);
            doc.update_style(row, |s| {
                s.direction = Direction::Row;
                s.gap = 4.0;
            });
            let cb = doc.create_checkbox();
            doc.append(row, cb);
            let label = doc.create_text("");
            doc.append(row, label);
            bind_text(doc, label, move || {
                slot.get().map_or("空".into(), |i| format!("条目 {i}"))
            });
            let tag = doc.create_text("静态标签");
            doc.append(row, tag);
            let btn = doc.create_button("操作");
            doc.append(row, btn);
            doc.update_style(btn, |s| {
                s.padding = 4.0.into();
                s.bg = Some(Color::rgb(255, 62, 0));
            });
        },
    );
    offset
}

/// 典型混合行:标题文本(响应式)+ 静态文本 + 按钮(带点击)+ 复选框,外包一个行容器。
/// 返回各行的信号(--mutate 增量工况驱动用)
fn build(doc: &Doc, rows: usize) -> Vec<sv_reactive::Signal<i32>> {
    let mut sigs = Vec::with_capacity(rows);
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
        sigs.push(count);

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
    sigs
}
