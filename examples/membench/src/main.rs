//! 性能基准测试台(调研 16 / ADR-9)。
//!
//! 一个二进制多场景:同一套渲染循环 + 同一行输出,跑在不同的树形与更新工况上。
//! 场景之间**只差一个变量**是刻意的——单个绝对数字没有意义,能相减的两个才有:
//! rows↔scroll 差出滚动帧的重布局、deep 大小深度差出父链回溯、text 池内/池外
//! 差出 Parley measure 的单价。
//!
//! 用法:
//! ```text
//! membench [--scene rows|virtual|deep|text|scroll|churn] [--controls 3000]
//!          [--frames N] [--mutate] [--backend cpu|vello] [--no-render]
//!          [--hold-secs 6] [--windowed]
//! 场景专属:--depth N(deep)  --text-pool N / --wrap(text)  --unkeyed(churn)
//! ```
//! 帧率口径:预热 1 帧后连续渲染 N 帧取均值(vello 离屏含纹理回读,略高估帧成本)。
//! 输出:`READY backend=… scene=… … p99_ms=… …` 后驻留 `--hold-secs` 秒,
//! 供外部(PowerShell)采样进程 WorkingSet/Private。
//!
//! **输出格式只增字段不改字段**:CI 的帧预算闸用 sed 抠 `p99_ms=`
//! (.github/workflows/ci.yml 的 bench job),换名字/换分隔符会让那道闸静默失效。
//! 各场景的实测数字与解读见同目录 README.md。

use std::rc::Rc;
use std::time::Instant;

use sv_reactive::{Signal, create_root, state};
use sv_ui::{
    Color, Direction, Doc, Overflow, TextWrap, ViewId, bind_text, each_block, each_block_keyed,
    virtual_list,
};

/// 场景选择。`--virtual` 是 CI 与旧采样脚本在用的老口径,等价于 `--scene virtual`,
/// 两者并存(旧命令行不能因为新增场景而失效)
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scene {
    Rows,
    Virtual,
    Deep,
    Text,
    Scroll,
    Churn,
}

impl Scene {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "rows" => Scene::Rows,
            "virtual" => Scene::Virtual,
            "deep" => Scene::Deep,
            "text" => Scene::Text,
            "scroll" => Scene::Scroll,
            "churn" => Scene::Churn,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Scene::Rows => "rows",
            Scene::Virtual => "virtual",
            Scene::Deep => "deep",
            Scene::Text => "text",
            Scene::Scroll => "scroll",
            Scene::Churn => "churn",
        }
    }
}

/// 场景参数(Copy:窗口模式要把它搬进 'static 闭包)
#[derive(Clone, Copy)]
struct Cfg {
    controls: usize,
    depth: usize,
    text_pool: usize,
    wrap: bool,
    unkeyed: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |name: &str, default: usize| -> usize {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let get_str = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let controls = get("--controls", 3000);
    let frames = get("--frames", 3);
    let hold = get("--hold-secs", 6);
    let no_render = args.iter().any(|a| a == "--no-render");
    let mutate = args.iter().any(|a| a == "--mutate");
    let windowed = args.iter().any(|a| a == "--windowed");
    let cfg = Cfg {
        controls,
        depth: get("--depth", 200),
        text_pool: get("--text-pool", 0),
        wrap: args.iter().any(|a| a == "--wrap"),
        unkeyed: args.iter().any(|a| a == "--unkeyed"),
    };
    let scene = match get_str("--scene") {
        Some(s) => match Scene::parse(&s) {
            Some(sc) => sc,
            None => {
                println!("SCENE-UNKNOWN {s}");
                return;
            }
        },
        None if args.iter().any(|a| a == "--virtual") => Scene::Virtual,
        None => Scene::Rows,
    };
    let virtual_mode = scene == Scene::Virtual;

    // 窗口模式:真实呈现路径(配 SV_SHOW_FPS=1 / SV_RENDERER 用);
    // --mutate 时用自续任务链驱动增量更新(每次后台完成 → 改一次 → 再排队)
    if windowed {
        sv_shell::run_app("membench 窗口", move |doc, _root| {
            let driver = Rc::new(build_scene(doc, scene, cfg));
            if mutate {
                fn chain(driver: Rc<Driver>, i: usize) {
                    sv_ui::tasks::spawn(
                        async { std::thread::sleep(std::time::Duration::from_millis(3)) },
                        move |_| {
                            driver.mutate(i);
                            chain(driver.clone(), i + 1);
                        },
                    );
                }
                chain(driver, 0);
            }
        })
        .expect("窗口模式失败");
        return;
    }

    let doc = Doc::new();
    let d = doc.clone();
    let t0 = Instant::now();
    let (driver, _scope) = create_root(move || build_scene(&d, scene, cfg));
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
        "READY backend={backend} scene={} mutate={mutate} virtual={virtual_mode} nodes={nodes} signals={signals} build_ms={} warmup_ms={warmup_ms} frame_avg_ms={avg:.2} p99_ms={p99:.2} low1_fps={low1:.0} fps={:.0} frames={}",
        scene.name(),
        built.as_millis(),
        if avg > 0.0 { 1000.0 / avg } else { 0.0 },
        if no_render { 0 } else { frames }
    );
    // 驻留供外部采样
    std::thread::sleep(std::time::Duration::from_secs(hold as u64));
}

/// 突变驱动:每个场景一种"每帧改什么"。改的东西不同,失效路径就不同——
/// 这正是场景要分开压的原因(节点总数相同,帧成本可以差一个数量级)
enum Driver {
    /// 行信号 +1:细粒度更新只重跑一处绑定,但版本 bump 让布局缓存整片失效
    Rows(Vec<Signal<i32>>),
    /// 虚拟列表滚动位(行域):逐槽 set,零节点创建/销毁
    Scroll(Signal<usize>),
    /// 真滚动容器的像素偏移:滚动帧 = 全树重布局(ADR-9 里虚拟化要省掉的那笔)
    ScrollPx {
        doc: Doc,
        container: ViewId,
        span: f32,
    },
    /// 帧号信号:所有 Text 的绑定都读它 → 一次写入改掉全部文本
    Frame(Signal<u64>),
    /// keyed each 整表左旋:key 一个不少但顺序全变 → reconcile 的重排分支
    Rotate(Signal<Vec<u32>>),
}

impl Driver {
    fn mutate(&self, i: usize) {
        match self {
            Driver::Rows(sigs) if !sigs.is_empty() => {
                sigs[i % sigs.len()].update(|v| *v += 1);
            }
            Driver::Rows(_) => {}
            Driver::Scroll(offset) => offset.update(|o| *o += 1),
            Driver::ScrollPx {
                doc,
                container,
                span,
            } => {
                // 取模回卷:偏移撞到内容底会被布局钳住,钳住之后每帧画的是同一屏,
                // 压出来的就不再是滚动成本了
                doc.set_scroll(*container, 0.0, (i as f32 * 13.0) % span.max(1.0));
            }
            Driver::Frame(f) => f.update(|v| *v += 1),
            Driver::Rotate(items) => items.update(|v| v.rotate_left(1)),
        }
    }
}

fn build_scene(doc: &Doc, scene: Scene, cfg: Cfg) -> Driver {
    let rows = cfg.controls / 5; // 每行 5 个控件(rows/virtual/scroll/churn 共同口径)
    match scene {
        Scene::Rows => Driver::Rows(build(doc, rows)),
        Scene::Virtual => Driver::Scroll(build_virtual(doc, rows)),
        Scene::Deep => Driver::Rows(build_deep(doc, cfg.controls, cfg.depth)),
        Scene::Text => Driver::Frame(build_text(doc, cfg.controls, cfg.text_pool, cfg.wrap)),
        Scene::Scroll => {
            let (container, span) = build_scroll(doc, rows);
            Driver::ScrollPx {
                doc: doc.clone(),
                container,
                span,
            }
        }
        Scene::Churn => Driver::Rotate(build_churn(doc, rows, !cfg.unkeyed)),
    }
}

/// 虚拟列表工况:逻辑 rows 行(每行 5 控件),视口只实例化 30 行。
/// 返回滚动位信号(--mutate 每帧 +1 = 连续滚动,虚拟化的最坏工况)
fn build_virtual(doc: &Doc, total_rows: usize) -> Signal<usize> {
    let offset = state(0usize);
    doc.update_style(doc.root(), |s| {
        s.padding = 8.0.into();
        s.gap = 2.0;
    });
    virtual_list(
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

/// 全量树工况(基线场景):rows 行直接挂在根上。
/// 返回各行的信号(--mutate 增量工况驱动用)
fn build(doc: &Doc, rows: usize) -> Vec<Signal<i32>> {
    let root = doc.root();
    doc.update_style(root, |s| {
        s.padding = 8.0.into();
        s.gap = 2.0;
    });
    build_rows_into(doc, root, rows)
}

/// 典型混合行:标题文本(响应式)+ 静态文本 + 按钮(带点击)+ 复选框,外包一个行容器。
/// rows / scroll 两个场景共用同一个行工厂——只有内容完全一致,两者的 p99 相减
/// 才等于"滚动这件事"的成本
fn build_rows_into(doc: &Doc, parent: ViewId, rows: usize) -> Vec<Signal<i32>> {
    let mut sigs = Vec::with_capacity(rows);
    for i in 0..rows {
        let row = doc.create_view();
        doc.append(parent, row);
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

/// 深树工况:节点数由 --controls 定,深度由 --depth 定,两者独立。
///
/// 压的是渲染期的**父链回溯**:字号/前景色继承(resolve_font_size / resolve_fg)
/// 与不透明度累乘(effective_opacity)都是从节点逐个往上走到根,一个都没有缓存,
/// 于是单帧代价是 O(节点数 × 深度) 而不是 O(节点数)。
/// 对照组不是别的场景,而是**同一场景的小 --depth**:节点数、内容、控件种类
/// 全都一样,只有深度变——差值就是父链那部分。
///
/// 叶子刻意用色块 View 而不是 Text:绘制端的 `text::shape` 没有缓存,一个叶子
/// 一次 parley 布局(几十 µs),几千个叶子的文本成本能把深度信号整个淹掉
/// (实测混文本时深度 200↔4 只差 20%,换色块后差一倍以上)。每层留一个
/// 绑定 Text 当"取样探针",保证 fg/font_size 两条继承路径也在被压。
fn build_deep(doc: &Doc, controls: usize, depth: usize) -> Vec<Signal<i32>> {
    let depth = depth.clamp(1, controls.max(1));
    // 每层 = 1 个链节 View + leaves 个叶子,凑够 controls 个节点
    let leaves = (controls / depth).saturating_sub(1).max(1);
    let root = doc.root();
    // 字号与前景色**只在根上给**:继承解析必须走满整条链才收敛(最坏工况);
    // 中间层一律不写,写了就等于给回溯装了个提前退出
    doc.update_style(root, |s| {
        s.padding = 4.0.into();
        s.font_size = 14.0;
        s.fg = Some(Color::rgb(30, 30, 34));
    });
    let mut sigs = Vec::with_capacity(depth);
    let mut cur = root;
    for level in 0..depth {
        let link = doc.create_view();
        doc.append(cur, link);
        doc.update_style(link, |s| {
            s.direction = Direction::Column;
            // opacity≠1 每 64 层才放一个:真实 UI 里半透明分组就是稀疏的。
            // 若哪天给 effective_opacity 加"整链都是 1 就早退"的快路,本场景
            // 应当如实变快 —— 层层写 0.99 只会把这条优化路堵死,测出假象
            if level % 64 == 63 {
                s.opacity = 0.92;
            }
        });
        // 【临时实验:去掉 Text 探针,改成同数量的色块】
        let head = doc.create_view();
        doc.append(link, head);
        doc.update_style(head, |s| {
            s.width = Some(8.0);
            s.height = Some(8.0);
            s.bg = Some(Color::rgb(200, 205, 210));
        });
        let sig = state(level as i32);
        sigs.push(sig);
        // 定尺色块:布局有确定尺寸(不进 measure 通道)、绘制只有一次 fill,
        // 于是每个叶子的帧成本几乎只剩 effective_opacity 的那趟父链
        for _ in 1..leaves {
            let leaf = doc.create_view();
            doc.append(link, leaf);
            doc.update_style(leaf, |s| {
                s.width = Some(8.0);
                s.height = Some(8.0);
                s.bg = Some(Color::rgb(200, 205, 210));
            });
        }
        cur = link;
    }
    sigs
}

/// 文本重排工况:controls 个 Text,每帧**全部**改内容(一个帧号信号 →
/// controls 个绑定重跑 → controls 次 set_text)。
///
/// 压的是 Parley measure 缓存的两条路:
/// - `--text-pool 0`(默认):每帧生成从没见过的串 = 全 miss,真跑 parley 布局;
/// - `--text-pool N`:在 N 个串里循环 = 全 hit(N 远小于缓存容量 4096),量出
///   "改文本"本身的地板价(绑定重跑 + set_text + 版本 bump + 重绘)。
///
/// 两者相减 = 测量阶段 parley 布局的单价。注意绘制端的 `text::shape` **没有缓存**,
/// 两边都得付,所以这个差值是 measure 的下界而不是文本栈的全部成本。
/// `--wrap` 再叠一层:定宽容器 + 折行,taffy 的两趟测量协议(MaxContent 问固有宽
/// → Definite 问折行后的高)会让每个叶子每帧问两次 parley。
fn build_text(doc: &Doc, count: usize, pool: usize, wrap: bool) -> Signal<u64> {
    /// 折行工况的段落体:中英混排 + 标点,断点要走 UAX #14(CJK 逐字可断、
    /// 拉丁词整体不可断),比纯 ASCII 更贴近真实界面
    const BODY: &str = "中英混排的长段落 measure 压力 abcdefg hijklmn,\
                        折行要走 UAX #14 断点表,行宽一变就得重来一遍";
    let frame = state(0u64);
    let root = doc.root();
    doc.update_style(root, |s| {
        s.padding = 8.0.into();
        s.gap = 1.0;
    });
    let col = doc.create_view();
    doc.append(root, col);
    if wrap {
        // 必须定宽:不定宽 taffy 直接按固有宽摆,折行路径一次都走不到
        doc.update_style(col, |s| s.width = Some(420.0));
    }
    for i in 0..count {
        let t = doc.create_text("");
        doc.append(col, t);
        doc.update_style(t, |s| {
            s.text_wrap = if wrap {
                TextWrap::Wrap
            } else {
                TextWrap::NoWrap
            };
        });
        bind_text(doc, t, move || {
            let f = frame.get() as usize;
            if pool > 0 {
                // 池内串**不能带行号**:带了就变成 count×pool 个不同串,
                // 冲爆缓存后又成了 miss 路径,池的意义就没了
                let s = format!("池 {} 号文本 abcdefg", (f + i) % pool);
                if wrap { format!("{s} {BODY}") } else { s }
            } else {
                let s = format!("行 {i} 帧 {f} 内容 abcdefg");
                if wrap { format!("{s} {BODY}") } else { s }
            }
        });
    }
    frame
}

/// 滚动搅动工况:一个真·滚动容器(overflow: Scroll + 定高视口)装下**全部** rows 行,
/// 刻意不虚拟化;每帧推一次像素偏移。
///
/// 布局缓存是按 Doc 版本号键控的整份产物,滚动一下就是一次 bump → 整棵树重新
/// measure/place + 重新算裁剪矩形。与 `--scene rows --mutate`(同样的行、同样每帧
/// 一次 bump)相减 = 裁剪/滚动区簿记的开销;与 `--scene virtual` 相减 = ADR-9
/// 里虚拟化省下的那笔钱。返回(容器, 滚动行程)
fn build_scroll(doc: &Doc, rows: usize) -> (ViewId, f32) {
    const VIEWPORT_H: f32 = 900.0;
    let root = doc.root();
    doc.update_style(root, |s| s.padding = 8.0.into());
    let container = doc.create_view();
    doc.append(root, container);
    doc.update_style(container, |s| {
        s.overflow = Overflow::Scroll;
        s.height = Some(VIEWPORT_H);
        s.gap = 2.0;
    });
    build_rows_into(doc, container, rows);
    // 行程按行高粗估即可:只要不越过内容底(越过会被布局钳住)就还在滚
    (container, (rows as f32 * 20.0 - VIEWPORT_H).max(1.0))
}

/// 结构搅动工况:each 列表每帧整表左旋 1(顺序全变、成员一个不少)。
///
/// 默认 keyed:行子树与行内状态全部复用(不重建、不重跑行内绑定),reconcile
/// 落到"逐行 append 对齐新序"的重排分支——append 先从父的 children 里摘掉再推到
/// 末尾,整表重排 = n 次 O(n) 摘除。这是 keyed each 唯一的超线性风险点,
/// 别的场景一个都压不到。
///
/// `--unkeyed` 换成不带 key 的 [`sv_ui::each_block`]:列表一变就
/// `clear_children` + 整表重建。它不是"另一个工况",而是**复用一旦失效的参照值**
/// ——keyed 那档的数字单看没有意义,要跟这一档比才知道复用值多少钱。
///
/// 实测结论(见 README):600–2400 行档两者同价(建/毁节点很便宜,而 keyed 自己
/// 的按 key 线性查找也是 O(n²)),keyed each 的价值是**状态保留**而不是帧成本。
fn build_churn(doc: &Doc, rows: usize, keyed: bool) -> Signal<Vec<u32>> {
    let items = state((0..rows as u32).collect::<Vec<u32>>());
    let root = doc.root();
    doc.update_style(root, |s| {
        s.padding = 8.0.into();
        s.gap = 2.0;
    });
    if keyed {
        each_block_keyed(
            doc,
            root,
            move || items.get(),
            |v: &u32| *v,
            |doc, row, sig| {
                // 行首标签绑信号:同 key 换内容走原地 set,行不重建
                let label = churn_row(doc, row);
                bind_text(doc, label, move || format!("条目 {}", sig.get()));
            },
        );
    } else {
        // 不带 key:行容器由本回调自己建(每次重建),节点结构与 keyed 档对齐
        each_block(
            doc,
            root,
            move || items.get(),
            |doc, parent, v, _| {
                let row = doc.create_view();
                doc.append(parent, row);
                let label = churn_row(doc, row);
                doc.set_text(label, &format!("条目 {v}"));
            },
        );
    }
    items
}

/// churn 的行内容(两档共用,保证只差"复用还是重建"这一个变量)。
/// 返回行首标签节点——keyed 档给它挂绑定,unkeyed 档直接写死文本
fn churn_row(doc: &Doc, row: ViewId) -> ViewId {
    doc.update_style(row, |s| {
        s.direction = Direction::Row;
        s.gap = 4.0;
    });
    let cb = doc.create_checkbox();
    doc.append(row, cb);
    let label = doc.create_text("");
    doc.append(row, label);
    let tag = doc.create_text("静态标签");
    doc.append(row, tag);
    doc.update_style(tag, |s| s.fg = Some(Color::rgb(120, 120, 136)));
    let btn = doc.create_button("操作");
    doc.append(row, btn);
    doc.update_style(btn, |s| {
        s.padding = 4.0.into();
        s.bg = Some(Color::rgb(255, 62, 0));
    });
    label
}
