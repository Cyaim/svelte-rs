//! 布局 + 绘制(CPU 自绘原型)
//!
//! 布局:taffy 0.12(封在 [`layout_tree`] 内,`Vec<Placed>` 输出契约不变)。
//! **继承**:`fg=None` / `font_size=NAN` 沿父链解析(color/font-size 白名单,
//! ADR-8 C1),根 fallback BLACK/16。measure 自顶向下携带解析值,
//! paint 对平铺列表做 O(depth) 父链回溯。
//! 绘制走 tiny-skia;逻辑坐标布局、物理坐标绘制(乘 scale)。
//! 排版/度量/光标几何**一律经 [`crate::text`] 门面**(Parley)——P3 起本文件
//! 再无第二套 shaping,旧线性 advance 路径与 `font.rs` 已退役。

use std::collections::HashMap;
use std::rc::Rc;

use tiny_skia::Pixmap;

use sv_ui::dirty::DirtyItem;
use sv_ui::{Color, Direction, Doc, DocumentInner, ElementKind, Overflow, ViewId};

use crate::paint::{Painter, TinySkiaPainter};

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// 一次布局的产物:绘制顺序排列(父先子后),rect 为逻辑坐标 border-box
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub id: ViewId,
    pub rect: Rect,
    /// 生效裁剪矩形(祖先滚动/Hidden 容器的交集;None = 不裁)。
    /// 命中测试直接用;绘制按 clip_depth 维护 push/pop 栈
    pub clip: Option<Rect>,
    /// 裁剪嵌套深度(= 祖先链上 overflow≠Visible 的容器数)
    pub clip_depth: u16,
}

impl Placed {
    /// 命中:点在 border-box 内且未被祖先裁掉(视口外不可点/不可悬停)
    pub fn hit(&self, x: f32, y: f32) -> bool {
        self.rect.contains(x, y) && self.clip.is_none_or(|c| c.contains(x, y))
    }
}

/// 滚动区元数据(布局旁路输出:滚轮路由 clamp 与滚动条比例的依据)
#[derive(Clone, Copy, Debug)]
pub struct ScrollArea {
    pub id: ViewId,
    /// border-box(逻辑坐标)
    pub viewport: Rect,
    /// 内容尺寸(不含 padding;content_override 优先)
    pub content: (f32, f32),
    /// 各轴最大滚动偏移(content − 内区,≥0)
    pub max: (f32, f32),
}

/// 弹层在 `Layout.placed` 中占据的区间(命中阻断与关闭手势的依据;
/// 调研 25:区间法比"遮罩节点吞事件"可靠——遮罩没 handler 会穿透)
#[derive(Clone, Copy, Debug)]
pub struct OverlayRegion {
    pub root: ViewId,
    pub start: usize,
    pub end: usize,
    pub layer: sv_ui::OverlayLayer,
    pub modal: bool,
    pub close: sv_ui::CloseBehavior,
}

/// 一次布局的完整产物
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub placed: Vec<Placed>,
    pub scroll_areas: Vec<ScrollArea>,
    pub overlay_regions: Vec<OverlayRegion>,
}

impl Layout {
    /// 命中许可:Tooltip 区间恒不可命中;存在 modal 时其 start 之下整体跳过
    pub fn hit_allowed(&self, idx: usize) -> bool {
        for r in &self.overlay_regions {
            if r.layer == sv_ui::OverlayLayer::Tooltip && idx >= r.start && idx < r.end {
                return false;
            }
        }
        let floor = self
            .overlay_regions
            .iter()
            .filter(|r| r.modal)
            .map(|r| r.start)
            .max()
            .unwrap_or(0);
        idx >= floor
    }

    /// 区间感知的最上层可点击命中(渲染壳用;裸 `hit_click_target` 留测试兼容)
    pub fn hit_click(&self, doc: &Doc, x: f32, y: f32) -> Option<ViewId> {
        self.placed
            .iter()
            .enumerate()
            .rev()
            .find(|(i, p)| self.hit_allowed(*i) && p.hit(x, y) && doc.click_handler(p.id).is_some())
            .map(|(_, p)| p.id)
    }
}

const ROOT_FONT_SIZE: f32 = 16.0;

/// 继承解析:自身 NAN → 父链向上,根 fallback
fn resolve_font_size(inner: &DocumentInner, id: ViewId) -> f32 {
    let mut cur = Some(id);
    while let Some(c) = cur {
        let Some(n) = inner.nodes.get(c) else { break };
        if !n.style.font_size.is_nan() {
            return n.style.font_size;
        }
        cur = n.parent;
    }
    ROOT_FONT_SIZE
}

fn resolve_fg(inner: &DocumentInner, id: ViewId) -> Color {
    let mut cur = Some(id);
    while let Some(c) = cur {
        let Some(n) = inner.nodes.get(c) else { break };
        if let Some(fg) = n.style.fg {
            return fg;
        }
        cur = n.parent;
    }
    Color::BLACK
}

// ---------------------------------------------------------------------------
// taffy 布局引擎(调研 23 T1:变更帧重建 TaffyTree,封在 layout_tree 内;
// Vec<Placed> 输出契约不变,paint/命中/缓存全部不动)
// ---------------------------------------------------------------------------

fn intersect(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    Rect {
        x: x0,
        y: y0,
        w: (x1 - x0).max(0.0),
        h: (y1 - y0).max(0.0),
    }
}

/// 叶子测量上下文(build 期解析好继承字号,继承不进 taffy)
struct MeasureCtx {
    kind: ElementKind,
    text: String,
    px: f32,
    /// Text 叶子是否折行(Button/Checkbox 恒单行;TextInput 见 rows)
    wrap: bool,
    /// TextInput 的可见行数:1 = 单行 `<input>`,>1 = 多行 `<textarea>`
    rows: u16,
    /// Animation 的固有尺寸(逻辑 px)。**与帧号无关** —— 动画换帧不改盒子,
    /// 这正是 `set_anim_frame` 能定级为纯绘制的依据
    intrinsic: (f32, f32),
}

// ---------------------------------------------------------------------------
// **不要在这里加"叶内 measure memo"。已经试过了,是负收益。**
//
// 动机看着很扎实:taffy 的多趟协议对每个叶子每帧问 **10 次**
// (实测 180000 次 / 18000 个叶子),而这 10 次只对应约 2.3 个不同的 `wrap_w`,
// 剩下全是重复询问。它们本该命中 taffy 自己的 9 槽缓存,但那是**直接映射**的:
// `compute_cache_slot` 让 "definite available space" 与 max-content 共用同一个槽
// 并无条件覆盖,于是"先问固有宽、再问折行高"这个必然序列每次都自己踩掉自己
// (所以给上游把槽数调大也没用 —— 是冲突,不是容量)。
//
// 于是在 `MeasureCtx` 里加了 4 槽 memo(键 = `wrap_w`),调用次数确实从 18 万
// 压到 4.2 万。**结果反而慢**,实测(30k 树,release,同机 A/B 各三轮):
//
// |            | 共享文本 | 逐行唯一 |
// |------------|---------|---------|
// | 有 memo    | 106ms   | 115ms   |
// | 无 memo    | **82ms**| **89ms**|
//
// 原因是 `text.rs` 的全局两代缓存命中一次只要一次哈希 + 一次探测,**本来就便宜**;
// 而 memo 要线性扫 4 格,还让 `MeasureCtx` 从约 40B 涨到约 112B(30000 个叶子
// 多摸 2MB)。省下来的比多付出的少。
//
// 更值得记的是**中间那次翻车**:memo 刚加上时逐行唯一档从 96ms 劣化到 365ms
// (3.8 倍)。根因不在 memo 自身,而在当时 `text.rs` 的容量自适应是
// "装满就降代 + 容量翻倍"的棘轮 —— 它的爬升速度取决于**有多少次查询打到它**。
// memo 把查询量砍掉四分之三,棘轮就爬不上去,缓存一直在颠簸。
// 即:**在一层缓存前面加一层缓存,会让后面那层的自适应失效。**
// 那条已改成"没到内存上限就原地扩容"(见 `text.rs` 的 `CAP`),与查询次数解耦;
// 记在这里是因为这类耦合极难从代码上看出来 —— 它没有任何错误行为,只是慢。
// ---------------------------------------------------------------------------

/// sv-ui Style → taffy::Style 纯映射(sv-ui 不依赖 taffy 类型,
/// 与 Painter 边界同理;调研 23 §2.2 映射表)
fn to_taffy(s: &sv_ui::Style) -> taffy::Style {
    use taffy::prelude::*;
    // 穷尽解构闸门(计划 §5.2 三道之一,与 sv_ui 的 Style::eq / dirty::layout_relevant 对齐):
    // 给 Style 加字段而不在这里过一遍 = 编译错误,逼作者判断它是否进 taffy 布局。
    // 绑到 `_` 只为形成编译期门禁,实际映射仍走下面的 `s.field`。
    let sv_ui::Style {
        direction: _,
        gap: _,
        padding: _,
        margin: _,
        border: _,
        width: _,
        height: _,
        min_width: _,
        min_height: _,
        max_width: _,
        max_height: _,
        flex_grow: _,
        flex_shrink: _,
        flex_wrap: _,
        justify_content: _,
        align_items: _,
        align_self: _,
        overflow: _,
        overflow_x: _,
        // 影响文本测量,经 measure_leaf 进布局,但不落 taffy::Style:
        font_size: _,
        text_wrap: _,
        // 以下纯绘制,不影响布局:
        bg: _,
        fg: _,
        corner_radius: _,
        opacity: _,
        cursor: _,
        text_align: _,
    } = s;
    let dim = |v: Option<f32>| v.map_or(Dimension::auto(), Dimension::length);
    let bw = s.border.map(|b| b.width).unwrap_or(0.0);
    taffy::Style {
        display: taffy::Display::Flex,
        flex_direction: match s.direction {
            Direction::Row => taffy::FlexDirection::Row,
            Direction::Column => taffy::FlexDirection::Column,
        },
        // 语义差已知:现状 gap 只作用主轴,taffy 双轴(wrap 后交叉轴也生效);
        // nowrap 下等价,`gap_cross_axis_semantics_pinned` 钉住
        gap: taffy::Size {
            width: LengthPercentage::length(s.gap),
            height: LengthPercentage::length(s.gap),
        },
        padding: taffy::Rect {
            left: LengthPercentage::length(s.padding.left),
            right: LengthPercentage::length(s.padding.right),
            top: LengthPercentage::length(s.padding.top),
            bottom: LengthPercentage::length(s.padding.bottom),
        },
        margin: taffy::Rect {
            left: LengthPercentageAuto::length(s.margin.left),
            right: LengthPercentageAuto::length(s.margin.right),
            top: LengthPercentageAuto::length(s.margin.top),
            bottom: LengthPercentageAuto::length(s.margin.bottom),
        },
        border: taffy::Rect {
            left: LengthPercentage::length(bw),
            right: LengthPercentage::length(bw),
            top: LengthPercentage::length(bw),
            bottom: LengthPercentage::length(bw),
        },
        size: taffy::Size {
            width: dim(s.width),
            height: dim(s.height),
        },
        min_size: taffy::Size {
            width: dim(s.min_width),
            height: dim(s.min_height),
        },
        max_size: taffy::Size {
            width: dim(s.max_width),
            height: dim(s.max_height),
        },
        // box_sizing 缺省 BorderBox,与现状"显式宽高 = border-box 覆盖"一致
        flex_grow: s.flex_grow,
        flex_shrink: s.flex_shrink,
        flex_wrap: match s.flex_wrap {
            sv_ui::FlexWrap::NoWrap => taffy::FlexWrap::NoWrap,
            sv_ui::FlexWrap::Wrap => taffy::FlexWrap::Wrap,
        },
        justify_content: Some(match s.justify_content {
            sv_ui::JustifyContent::Start => taffy::JustifyContent::FLEX_START,
            sv_ui::JustifyContent::Center => taffy::JustifyContent::CENTER,
            sv_ui::JustifyContent::End => taffy::JustifyContent::FLEX_END,
            sv_ui::JustifyContent::SpaceBetween => taffy::JustifyContent::SPACE_BETWEEN,
            sv_ui::JustifyContent::SpaceAround => taffy::JustifyContent::SPACE_AROUND,
            sv_ui::JustifyContent::SpaceEvenly => taffy::JustifyContent::SPACE_EVENLY,
        }),
        // taffy 缺省(None)= stretch;sv 缺省 Start = 保持既有
        // "顶对齐不拉伸"行为,迁移零回归优先(调研 23 §2.2)
        align_items: Some(map_align(s.align_items)),
        align_self: s.align_self.map(map_align),
        overflow: if s.overflow == Overflow::Visible && s.overflow_x == Overflow::Visible {
            taffy::Point {
                x: taffy::Overflow::Visible,
                y: taffy::Overflow::Visible,
            }
        } else {
            // Hidden/Scroll 都按 taffy Scroll 建模(min-content 归零、
            // 尺寸不被内容撑开);滚动条空间自绘不占位
            taffy::Point {
                x: taffy::Overflow::Scroll,
                y: taffy::Overflow::Scroll,
            }
        },
        scrollbar_width: 0.0,
        ..Default::default()
    }
}

fn map_align(a: sv_ui::AlignItems) -> taffy::AlignItems {
    match a {
        sv_ui::AlignItems::Start => taffy::AlignItems::FLEX_START,
        sv_ui::AlignItems::Center => taffy::AlignItems::CENTER,
        sv_ui::AlignItems::End => taffy::AlignItems::FLEX_END,
        sv_ui::AlignItems::Stretch => taffy::AlignItems::STRETCH,
    }
}

/// build 期递归建 TaffyTree(View → with_children,叶子 → leaf+context;
/// 继承字号在此解析,taffy 不知道继承)
/// 建树递归深度上限。**超了就把该子树当叶子处理,而不是继续递归爆栈**。
///
/// 实测(membench `--scene deep`,Windows 主线程 1MB 栈):约 400 层时
/// `build_taffy`/`walk_taffy` 的递归会**栈溢出**,而同样深度加 `--no-render`
/// 安然无恙 —— 爆的确实是这两个递归,不是建树也不是析构。
///
/// 为什么不改成显式栈的迭代版:那是正解,但两个递归都要改、都要重测,
/// 而真实 UI 不会有 256 层嵌套(有的话是生成器写崩了)。这里先用一道
/// **可诊断的截断**换掉"不可恢复的崩溃":栈溢出既不能 catch 也没有栈回溯,
/// 是所有失败模式里最难查的一种(R4 去 panic 同一纪律:宁可降级,不要崩)。
const MAX_TREE_DEPTH: usize = 256;

fn build_taffy(
    inner: &DocumentInner,
    tree: &mut taffy::TaffyTree<MeasureCtx>,
    map: &mut HashMap<u64, ViewId>,
    id: ViewId,
    inherited_font: f32,
) -> taffy::NodeId {
    build_taffy_at(inner, tree, map, id, inherited_font, 0)
}

fn build_taffy_at(
    inner: &DocumentInner,
    tree: &mut taffy::TaffyTree<MeasureCtx>,
    map: &mut HashMap<u64, ViewId>,
    id: ViewId,
    inherited_font: f32,
    depth: usize,
) -> taffy::NodeId {
    let n = &inner.nodes[id];
    let fs = if n.style.font_size.is_nan() {
        inherited_font
    } else {
        n.style.font_size
    };
    let tstyle = to_taffy(&n.style);
    // 到顶就不再往下:该子树整体当叶子(零尺寸),界面会缺一块,
    // 但进程还活着、日志里写清了是谁太深
    let too_deep = depth >= MAX_TREE_DEPTH;
    if too_deep {
        report_too_deep(id);
    }
    let node = if n.kind == ElementKind::View && !too_deep {
        let children: Vec<taffy::NodeId> = n
            .children
            .iter()
            .map(|c| build_taffy_at(inner, tree, map, *c, fs, depth + 1))
            .collect();
        tree.new_with_children(tstyle, &children)
            .expect("sv-shell: taffy 建节点失败")
    } else {
        tree.new_leaf_with_context(
            tstyle,
            MeasureCtx {
                kind: n.kind,
                text: n.text.clone(),
                px: fs,
                wrap: n.kind == ElementKind::Text && n.style.text_wrap == sv_ui::TextWrap::Wrap,
                rows: n
                    .input
                    .as_deref()
                    .filter(|i| i.multiline)
                    .map_or(1, |i| i.rows),
                intrinsic: n.anim.as_deref().map_or((0.0, 0.0), |a| a.intrinsic),
            },
        )
        .expect("sv-shell: taffy 建叶子失败")
    };
    map.insert(u64::from(node), id);
    node
}

/// 叶子测量(taffy measure function 通道;调研 23 §2.3 两趟协议:
/// MaxContent 问固有宽 → Definite/known 问折行后高)
fn measure_leaf(
    known: taffy::Size<Option<f32>>,
    available: taffy::Size<taffy::AvailableSpace>,
    ctx: Option<&mut MeasureCtx>,
) -> taffy::Size<f32> {
    let Some(ctx) = ctx else {
        return taffy::Size::ZERO;
    };
    match ctx.kind {
        ElementKind::Text => {
            let wrap_w = if !ctx.wrap {
                None
            } else {
                match (known.width, available.width) {
                    (Some(w), _) => Some(w),
                    (None, taffy::AvailableSpace::Definite(w)) => Some(w),
                    // MinContent = 最长不可断段宽(逐段成行)
                    (None, taffy::AvailableSpace::MinContent) => Some(0.0),
                    (None, taffy::AvailableSpace::MaxContent) => None,
                }
            };
            let (w, h) = crate::text::measure(&ctx.text, ctx.px, wrap_w);
            taffy::Size {
                width: w,
                height: h,
            }
        }
        ElementKind::Button => {
            let (w, h) = crate::text::measure(&ctx.text, ctx.px, None);
            taffy::Size {
                width: w,
                height: h,
            }
        }
        ElementKind::Checkbox => {
            let side = ctx.px.max(14.0);
            taffy::Size {
                width: side,
                height: side,
            }
        }
        // 多行 textarea 的高 = rows × 行高(内容再长也不撑高:溢出靠滚动,
        // 与浏览器 textarea 一致)
        ElementKind::TextInput => taffy::Size {
            width: 200.0,
            height: crate::text::line_height(ctx.px) * f32::from(ctx.rows.max(1)),
        },
        // 动画的固有尺寸就是素材尺寸,不随帧号变。
        // 素材还没接上(占位)时是 0×0 —— 于是"忘了接素材"表现为界面上缺一块,
        // 而不是撑出一个莫名其妙的大洞
        ElementKind::Animation => taffy::Size {
            width: ctx.intrinsic.0,
            height: ctx.intrinsic.1,
        },
        // View 通常不是叶子——**除了被 MAX_TREE_DEPTH 截断的那一个**:
        // 它带着 View 的 kind 进了 new_leaf_with_context。给零尺寸,
        // 于是超深子树表现为"这里缺一块",而不是 unreachable! 崩掉
        ElementKind::View => taffy::Size::ZERO,
    }
}

/// taffy 布局结果 → 绝对坐标 Placed + clip 传播 + ScrollArea 旁路
/// (语义与旧 place 一致:滚动平移、布局期钳制、视口裁剪)
#[allow(clippy::too_many_arguments)]
fn walk_taffy(
    inner: &DocumentInner,
    tree: &taffy::TaffyTree<MeasureCtx>,
    map: &HashMap<u64, ViewId>,
    node: taffy::NodeId,
    origin: (f32, f32),
    clip: Option<Rect>,
    clip_depth: u16,
    out: &mut Layout,
) {
    let l = tree.layout(node).expect("sv-shell: taffy 布局缺节点");
    let vid = map[&u64::from(node)];
    let (x, y) = (origin.0 + l.location.x, origin.1 + l.location.y);
    let rect = Rect {
        x,
        y,
        w: l.size.width,
        h: l.size.height,
    };
    out.placed.push(Placed {
        id: vid,
        rect,
        clip,
        clip_depth,
    });
    let n = &inner.nodes[vid];
    if n.kind != ElementKind::View {
        return;
    }

    // 任一轴非 Visible 就要裁剪(裁剪矩形是二维的,没法只裁一轴)
    let clips = n.style.overflow != Overflow::Visible || n.style.overflow_x != Overflow::Visible;
    let (child_clip, child_depth, scroll) = if clips {
        let clip2 = Some(clip.map_or(rect, |c| intersect(c, rect)));
        // 内容尺寸与滚动范围:content_override(virtual_scroll 桥)优先,
        // 否则取 taffy 的 scrollable overflow(content_size 含 padding 贡献)
        let inner_w =
            l.size.width - l.padding.left - l.padding.right - l.border.left - l.border.right;
        let inner_h =
            l.size.height - l.padding.top - l.padding.bottom - l.border.top - l.border.bottom;
        let (content, max) = match n.content_override {
            Some((ow, oh)) => ((ow, oh), ((ow - inner_w).max(0.0), (oh - inner_h).max(0.0))),
            None => (
                (l.content_size.width, l.content_size.height),
                (l.scroll_width(), l.scroll_height()),
            ),
        };
        // 只在**该轴可滚**时给出滚动范围:横向 hidden + 纵向 scroll 的
        // 容器不该被滚轮横推,滚动条也只该出纵向那根
        let scrollable = (
            n.style.overflow_x == Overflow::Scroll,
            n.style.overflow == Overflow::Scroll,
        );
        let max = (
            if scrollable.0 { max.0 } else { 0.0 },
            if scrollable.1 { max.1 } else { 0.0 },
        );
        if scrollable.0 || scrollable.1 {
            out.scroll_areas.push(ScrollArea {
                id: vid,
                viewport: rect,
                content,
                max,
            });
        }
        (
            clip2,
            clip_depth + 1,
            (n.scroll_x.min(max.0), n.scroll_y.min(max.1)),
        )
    } else {
        (clip, clip_depth, (0.0, 0.0))
    };

    if let Ok(children) = tree.children(node) {
        for c in children {
            walk_taffy(
                inner,
                tree,
                map,
                c,
                (x - scroll.0, y - scroll.1),
                child_clip,
                child_depth,
                out,
            );
        }
    }
}

/// 版本键控布局缓存(完整产物):同一 Doc、同版本、同尺寸 → 直接复用。
/// 静止帧的 O(n) measure/place 归零(细粒度更新模型下,静止是常态)。
/// 滚动改 offset → bump 版本 → 键自然失效(滚动帧 = 全树重布局,
/// 大全量树靠 virtual_list 兜底,ADR-9)
/// 建好并算完的布局树 —— **持久但只读**。
///
/// 关键性质:它只有两种状态 —— **和 Doc 完全一致,或者已经被整棵扔掉**。
/// 没有"增量更新"这条路,于是也就不存在"两棵树失同步"这一整类 bug。
/// taffy 那几条著名的陷阱(`add_child` 不摘旧父、`remove` 不递归删后代、
/// `get_node_context_mut` 不标脏)一条都碰不到 —— 我们从不调用它们。
///
/// 它买到的是:滚动帧、`content_override` 帧不用再重建 30k 个 taffy 节点,
/// 只重走一遍产出坐标(实测 30k 档 2.6–4.6ms,对照全量 328–361ms)。
/// 代价是影子树从每帧临时变成常驻(30k 档约 33MB)——所以 [`KEEP_TREE_MIN_NODES`]
/// 给了闸门,小界面根本不付这笔钱。
struct LayoutTrees {
    tree: taffy::TaffyTree<MeasureCtx>,
    map: HashMap<u64, ViewId>,
    /// `map` 的反向:ViewId → NodeId,增量 Measure 用(改了哪个节点就直接找到它的
    /// taffy 节点)。只覆盖**基础树**;弹层节点不在里面(改弹层内节点会退回全量,
    /// 见 [`try_incremental_measure`])。建树时一次性从 `map` 反转,重建才更新。
    v2n: HashMap<ViewId, taffy::NodeId>,
    root: taffy::NodeId,
    /// 弹层各一棵独立树,**按 walk 顺序存**(Popup 注册序在前、Tooltip 恒最后)。
    /// 顺序只在"注册表没变"的前提下有效 —— 而注册表一变就是重建,所以恒有效
    overlays: Vec<OverlayTree>,
}

struct OverlayTree {
    entry: sv_ui::overlay::OverlayEntry,
    tree: taffy::TaffyTree<MeasureCtx>,
    map: HashMap<u64, ViewId>,
    root: taffy::NodeId,
}

/// 小于这个节点数就不留树。
///
/// 留树是拿内存换时间,而小界面**两头都不划算**:全量重建本来就只要零点几毫秒,
/// 留着的树却要一直占着内存。阈值取 512 是因为实测 3k 档全量重建已经 ~19ms
/// (值得留),而几百个节点的对话框在噪声里。
const KEEP_TREE_MIN_NODES: usize = 512;

struct LayoutCache {
    doc_id: usize,
    w: u32,
    h: u32,
    layout: Rc<Layout>,
    /// 树可能没留(小界面,见 [`KEEP_TREE_MIN_NODES`]);没留就退化成全量重建
    trees: Option<LayoutTrees>,
}

thread_local! {
    static CACHE: std::cell::RefCell<Option<LayoutCache>> =
        const { std::cell::RefCell::new(None) };
}

/// 按 [`sv_ui::Doc::take_dirty`] 的分级决定这一帧到底要做多少事。
///
/// 三档,从便宜到贵:
/// 1. **什么都没脏** → 直接把上一帧的 `Rc<Layout>` 递回去(连 clone 都没有);
/// 2. **只挪位置**(滚动 / `content_override`)→ 复用布局树,只重走产出坐标;
/// 3. **真布局脏**(文本 / 布局样式 / 结构 / 弹层注册表)→ 整棵重建。
///
/// 返回 **`Rc<Layout>`** 而不是 `Layout`:命中时只拷指针。以前命中返回
/// `layout.clone()` —— 深拷三个 Vec,而每帧**要调两次**(`render_frame` 内部
/// 一次、事件循环存 `self.layout` 一次),30k 档一份 Layout ≈1.4MB,
/// 于是静止帧也在跑 1.4MB × 2 × 帧率的纯 memcpy。
///
/// # 关于"日志被取走"这件事
///
/// [`sv_ui::Doc::take_dirty`] 是破坏性的。这看起来危险,实际上正好修掉一个旧问题:
/// 同一帧内第二次调用本函数时日志已空 → 走第 1 档直接复用,
/// 而以前它靠版本号比较,拿到"命中"之后仍然深拷了一份。
///
/// 没有渲染壳的调用方(单测、离屏 PNG、直调 [`layout_tree_full`])不取日志,
/// 日志会一直涨到上限然后置 `overflowed` —— 那是"退化成全量",方向是安全的。
pub fn layout_full_cached(doc: &Doc, logical_w: f32, logical_h: f32) -> Rc<Layout> {
    let dirty = doc.take_dirty();
    let (doc_id, w, h) = (doc.identity(), logical_w.to_bits(), logical_h.to_bits());

    CACHE.with(|c| {
        let mut slot = c.borrow_mut();
        // Doc 换了或窗口尺寸变了:缓存整个作废。
        // 【已知局限】缓存是单槽 —— 两个窗口交替渲染会互相顶掉,退化成每帧全量。
        // 多窗口今天还没有支持,真做的时候这里要改成按 doc_id 分槽
        let reusable = match slot.as_ref() {
            Some(cached) => cached.doc_id == doc_id && cached.w == w && cached.h == h,
            None => false,
        };

        // 判据是 `needs_rewalk`,**不是 `is_clean`**:一帧里全是 Paint 时日志非空
        // 但布局产物逐字节不变。第一版写成 `is_clean()`,于是换色帧掉进了下面的
        // "重走"分支 —— 结果正确、但白跑一遍 walk,而 walk 正是 30k 档空转帧的
        // 全部成本。`paint_only_change_reuses_layout_verbatim` 就是逮它的
        if reusable && !dirty.needs_rewalk() {
            return Rc::clone(&slot.as_ref().expect("reusable 已保证非空").layout);
        }

        if reusable && !dirty.needs_rebuild() {
            // 只挪位置:布局树整棵复用,重走一遍产出坐标
            let cached = slot.as_mut().expect("reusable 已保证非空");
            if let Some(trees) = cached.trees.as_ref() {
                let layout =
                    Rc::new(doc.read(|inner| walk_trees(inner, trees, logical_w, logical_h)));
                cached.layout = Rc::clone(&layout);
                return layout;
            }
            // 树没留(小界面):落到重建,反正它便宜
        }

        // 增量 Measure(计划步骤 3 的安全子集):这一帧要重建,但**全部重建项都是
        // `Measure`**(结构没动)且树还留着 → 只更新改动节点、让 taffy 重算脏子树,
        // 不整棵重扔。结构一变(Structure/InheritFontSize/OverlayRegistry/溢出)就
        // 不走这条,老老实实全量 —— 那些才是 §3.4 taffy 陷阱的所在,这里一个不碰。
        if reusable && !dirty.overflowed {
            let only_measure = dirty.items.iter().all(|i| {
                matches!(
                    i,
                    DirtyItem::Paint | DirtyItem::Position { .. } | DirtyItem::Measure { .. }
                )
            });
            if only_measure {
                let changed: Vec<ViewId> = dirty
                    .items
                    .iter()
                    .filter_map(|i| match i {
                        DirtyItem::Measure { id } => Some(*id),
                        _ => None,
                    })
                    .collect();
                let cached = slot.as_mut().expect("reusable 已保证非空");
                if let Some(trees) = cached.trees.as_mut()
                    && let Some(layout) = doc.read(|inner| {
                        try_incremental_measure(inner, trees, &changed, logical_w, logical_h)
                    })
                {
                    let layout = Rc::new(layout);
                    cached.layout = Rc::clone(&layout);
                    return layout;
                }
                // 增量没走成(节点不在基础树等)→ 落到下面全量,安全
            }
        }

        // **先把旧树扔掉再建新的**。留着它直到 `*slot = Some(..)` 才落地,
        // 意味着重建帧上新旧两棵树同时活着 —— 峰值内存翻倍,分配器局部性变差。
        // 实测 membench `rows 3k --mutate`(每帧都重建):不扔 20.0–21.0ms,
        // 扔了 19.0–19.3ms,**白拿 5%**。这条只在"每帧都是 C 类"的负载上看得见,
        // 而那恰恰是最坏情况
        *slot = None;
        let (layout, trees) = doc.read(|inner| {
            let trees = build_trees(inner, logical_w, logical_h);
            let layout = walk_trees(inner, &trees, logical_w, logical_h);
            let keep = layout.placed.len() >= KEEP_TREE_MIN_NODES;
            (layout, keep.then_some(trees))
        });
        let layout = Rc::new(layout);
        *slot = Some(LayoutCache {
            doc_id,
            w,
            h,
            layout: Rc::clone(&layout),
            trees,
        });
        layout
    })
}

// 测试探针:增量 Measure 路径被成功走过的次数(证明它不是死代码)。
// **thread_local**:布局本就是线程局部单线程模型(CACHE 同款),而测试并行跑 ——
// 用进程级 static 会被别的测试线程的自增打乱 `+1` 断言。
#[cfg(test)]
thread_local! {
    static INCREMENTAL_MEASURE_HITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn incremental_hits() -> usize {
    INCREMENTAL_MEASURE_HITS.with(|c| c.get())
}

#[cfg(test)]
pub(crate) fn reset_incremental_hits() {
    INCREMENTAL_MEASURE_HITS.with(|c| c.set(0));
}

/// 测试探针:上一帧的布局树还留着吗。
/// 用来断言"滚动帧没有重建树",比计时可靠得多
#[cfg(test)]
pub(crate) fn cache_has_trees() -> bool {
    CACHE.with(|c| c.borrow().as_ref().is_some_and(|x| x.trees.is_some()))
}

#[cfg(test)]
pub(crate) fn cache_reset() {
    CACHE.with(|c| *c.borrow_mut() = None);
}

/// 布局整棵树(完整产物:Placed + 滚动区元数据)。root 强制占满窗口逻辑尺寸。
/// 引擎 = taffy 0.12(调研 23:变更帧重建 + measure fn;disable_rounding
/// 保逻辑坐标精度,取整留给绘制端——HiDPI 下逻辑取整会放大成整物理像素跳动)
pub fn layout_tree_full(doc: &Doc, logical_w: f32, logical_h: f32) -> Layout {
    doc.read(|inner| {
        let trees = build_trees(inner, logical_w, logical_h);
        walk_trees(inner, &trees, logical_w, logical_h)
    })
}

/// 建树 + 算尺寸。**贵的那一半**:30k 档 300ms 量级,其中绝大部分是叶子测量。
fn build_trees(inner: &DocumentInner, logical_w: f32, logical_h: f32) -> LayoutTrees {
    let mut tree: taffy::TaffyTree<MeasureCtx> = taffy::TaffyTree::new();
    tree.disable_rounding();
    let mut map: HashMap<u64, ViewId> = HashMap::new();
    let root = build_taffy(inner, &mut tree, &mut map, inner.root, ROOT_FONT_SIZE);
    // root 强制占满窗口
    let mut rs = to_taffy(&inner.nodes[inner.root].style);
    rs.size = taffy::Size {
        width: taffy::Dimension::length(logical_w),
        height: taffy::Dimension::length(logical_h),
    };
    tree.set_style(root, rs)
        .expect("sv-shell: 设 root 样式失败");
    tree.compute_layout_with_measure(
        root,
        taffy::Size {
            width: taffy::AvailableSpace::Definite(logical_w),
            height: taffy::AvailableSpace::Definite(logical_h),
        },
        |known, available, _id, ctx, _style| measure_leaf(known, available, ctx),
    )
    .expect("sv-shell: taffy 布局失败");

    // 弹层(调研 25):基础层后追加;Popup 注册序,Tooltip 恒最后。
    // 每弹层一棵独立 taffy 树(游离子树,尺寸=内容,上限=窗口)。
    // **两趟循环在这里就定死顺序**,产出坐标那一半照着存下来的顺序走即可
    let mut overlays = Vec::new();
    for pass_tooltip in [false, true] {
        for e in &inner.overlays {
            if (e.layer == sv_ui::OverlayLayer::Tooltip) != pass_tooltip {
                continue;
            }
            let mut otree: taffy::TaffyTree<MeasureCtx> = taffy::TaffyTree::new();
            otree.disable_rounding();
            let mut omap: HashMap<u64, ViewId> = HashMap::new();
            let oroot = build_taffy(inner, &mut otree, &mut omap, e.root, ROOT_FONT_SIZE);
            otree
                .compute_layout_with_measure(
                    oroot,
                    taffy::Size {
                        width: taffy::AvailableSpace::Definite(logical_w),
                        height: taffy::AvailableSpace::Definite(logical_h),
                    },
                    |known, available, _id, ctx, _style| measure_leaf(known, available, ctx),
                )
                .expect("sv-shell: 弹层布局失败");
            overlays.push(OverlayTree {
                entry: e.clone(),
                tree: otree,
                map: omap,
                root: oroot,
            });
        }
    }

    // 反转 map(NodeId→ViewId)得到 v2n(ViewId→NodeId)。一次 O(节点数),
    // 与建树同量级;增量 Measure 靠它 O(1) 找到改动节点的 taffy 节点。
    let v2n = map
        .iter()
        .map(|(&n, &v)| (v, taffy::NodeId::from(n)))
        .collect();

    LayoutTrees {
        tree,
        map,
        v2n,
        root,
        overlays,
    }
}

/// 增量 Measure:一帧里**只有 `Measure` 变更**(结构没动)时,不重建整棵树,
/// 只把改动节点的 taffy 样式 / 测量上下文更新掉,让 taffy 重算脏子树。
///
/// **为什么这条安全**(§3.4 的雷一个都不碰):它**从不** `add_child` / `remove` /
/// reparent —— 结构没变是进入这条路的前提。改的只有 `set_style`(标脏,安全)与
/// `set_node_context`(标脏,安全),都是 taffy 最稳的那一层。
///
/// 任一步对不上(节点不在基础树 v2n 里、Doc 里查不到、taffy 报错)就返回 `None`,
/// 调用方**退回全量重建** —— 宁可慢一帧,不出错。差分 fuzz 会拿它的产物逐帧对拍
/// 全量重算,坐标错了立刻红。
fn try_incremental_measure(
    inner: &DocumentInner,
    trees: &mut LayoutTrees,
    changed: &[ViewId],
    logical_w: f32,
    logical_h: f32,
) -> Option<Layout> {
    for &id in changed {
        let node = trees.v2n.get(&id).copied()?;
        let n = inner.nodes.get(id)?;
        // 布局相关样式可能也变了(Measure 也涵盖"布局样式变") → 同步 taffy 样式
        trees.tree.set_style(node, to_taffy(&n.style)).ok()?;
        // 叶子(有测量上下文)才重建 context;View 只有样式没有 context
        if trees.tree.get_node_context(node).is_some() {
            // 有效字号:节点自己设了就用自己的,否则用旧 context 里的继承值 ——
            // 继承值在 Measure-only 帧里稳定(改继承是 InheritFontSize,会走全量)
            let old_px = trees
                .tree
                .get_node_context(node)
                .map_or(ROOT_FONT_SIZE, |c| c.px);
            let fs = if n.style.font_size.is_nan() {
                old_px
            } else {
                n.style.font_size
            };
            let ctx = MeasureCtx {
                kind: n.kind,
                text: n.text.clone(),
                px: fs,
                wrap: n.kind == ElementKind::Text && n.style.text_wrap == sv_ui::TextWrap::Wrap,
                rows: n
                    .input
                    .as_deref()
                    .filter(|i| i.multiline)
                    .map_or(1, |i| i.rows),
                intrinsic: n.anim.as_deref().map_or((0.0, 0.0), |a| a.intrinsic),
            };
            trees.tree.set_node_context(node, Some(ctx)).ok()?;
        }
    }
    // 测试探针:证明这条增量路径**真的被走过**(不是死代码),供 fuzz 之外的定点测试断言
    #[cfg(test)]
    INCREMENTAL_MEASURE_HITS.with(|c| c.set(c.get() + 1));

    // taffy 只重算被标脏的子树(及其受影响的祖先);未动的分支命中缓存
    trees
        .tree
        .compute_layout_with_measure(
            trees.root,
            taffy::Size {
                width: taffy::AvailableSpace::Definite(logical_w),
                height: taffy::AvailableSpace::Definite(logical_h),
            },
            |known, available, _id, ctx, _style| measure_leaf(known, available, ctx),
        )
        .ok()?;
    Some(walk_trees(inner, trees, logical_w, logical_h))
}

/// 算好的树 → 绝对坐标产物。**便宜的那一半**:30k 档 2.6–4.6ms,纯遍历。
///
/// 它每次都重读 `inner`,所以滚动偏移、`content_override` 这些"不进 taffy 但
/// 影响最终坐标"的东西会被如实反映 —— 这正是 Position 档只跑这一半的依据。
fn walk_trees(
    inner: &DocumentInner,
    trees: &LayoutTrees,
    logical_w: f32,
    logical_h: f32,
) -> Layout {
    let mut out = Layout::default();
    walk_taffy(
        inner,
        &trees.tree,
        &trees.map,
        trees.root,
        (0.0, 0.0),
        None,
        0,
        &mut out,
    );

    for o in &trees.overlays {
        let ol = o.tree.layout(o.root).expect("sv-shell: 弹层根缺布局");
        let Some(origin) = resolve_anchor(
            o.entry.anchor,
            ol.size.width,
            ol.size.height,
            logical_w,
            logical_h,
            &out.placed,
        ) else {
            continue; // 锚点节点不存在/被裁:本帧不显示
        };
        let start = out.placed.len();
        walk_taffy(inner, &o.tree, &o.map, o.root, origin, None, 0, &mut out);
        out.overlay_regions.push(OverlayRegion {
            root: o.entry.root,
            start,
            end: out.placed.len(),
            layer: o.entry.layer,
            modal: o.entry.modal,
            close: o.entry.close,
        });
    }
    out
}

/// 锚点解析(调研 25 §2.4):Node 锚 → 侧向 + 越界翻转;最后 clamp 进窗口
fn resolve_anchor(
    anchor: sv_ui::Anchor,
    ow: f32,
    oh: f32,
    lw: f32,
    lh: f32,
    placed: &[Placed],
) -> Option<(f32, f32)> {
    use sv_ui::{Anchor, Side};
    let (x, y) = match anchor {
        Anchor::WindowCenter => (((lw - ow) / 2.0), ((lh - oh) / 2.0)),
        Anchor::Point(x, y) => (x, y),
        Anchor::Node { id, side, gap } => {
            let r = placed.iter().find(|p| p.id == id)?.rect;
            let (mut x, mut y) = match side {
                Side::Below => (r.x, r.y + r.h + gap),
                Side::Above => (r.x, r.y - gap - oh),
                Side::Right => (r.x + r.w + gap, r.y),
                Side::Left => (r.x - gap - ow, r.y),
            };
            // 主轴放不下且对侧放得下 → 翻转
            match side {
                Side::Below if y + oh > lh && r.y - gap - oh >= 0.0 => y = r.y - gap - oh,
                Side::Above if y < 0.0 && r.y + r.h + gap + oh <= lh => y = r.y + r.h + gap,
                Side::Right if x + ow > lw && r.x - gap - ow >= 0.0 => x = r.x - gap - ow,
                Side::Left if x < 0.0 && r.x + r.w + gap + ow <= lw => x = r.x + r.w + gap,
                _ => {}
            }
            (x, y)
        }
    };
    Some((
        x.clamp(0.0, (lw - ow).max(0.0)),
        y.clamp(0.0, (lh - oh).max(0.0)),
    ))
}

/// 点击前的弹层关闭手势判定(纯函数,离屏可测)。返回是否吞掉该次点击:
/// OnClickOutside 点外 → dismiss + 吞;OnAnyClick → dismiss + 不吞(点选项照常)
pub fn overlay_click_gate(doc: &Doc, layout: &Layout, x: f32, y: f32) -> bool {
    use sv_ui::{CloseBehavior, OverlayLayer};
    let Some(r) = layout
        .overlay_regions
        .iter()
        .rev()
        .find(|r| r.layer == OverlayLayer::Popup && r.close != CloseBehavior::None)
    else {
        return false;
    };
    let inside = layout.placed[r.start..r.end].iter().any(|p| p.hit(x, y));
    match r.close {
        CloseBehavior::OnClickOutside if !inside => {
            doc.dismiss_overlay(r.root);
            true
        }
        CloseBehavior::OnAnyClick => {
            doc.dismiss_overlay(r.root);
            false
        }
        _ => false,
    }
}

/// 兼容入口:只要 Placed 列表
pub fn layout_tree(doc: &Doc, logical_w: f32, logical_h: f32) -> Vec<Placed> {
    layout_tree_full(doc, logical_w, logical_h).placed
}

/// 把节点的不透明度乘进颜色 alpha
fn with_opacity(c: Color, o: f32) -> Color {
    Color::rgba(c.r, c.g, c.b, (c.a as f32 * o.clamp(0.0, 1.0)) as u8)
}

/// 有效不透明度 = 自身 × 祖先链乘积(近似组透明,v0 无合成层;
/// 换 vello 后由 push_layer/pop_layer 天然正确)
fn effective_opacity(inner: &DocumentInner, id: ViewId) -> f32 {
    let mut o = 1.0f32;
    let mut cur = Some(id);
    while let Some(c) = cur {
        let Some(n) = inner.nodes.get(c) else { break };
        o *= n.style.opacity;
        cur = n.parent;
    }
    o
}

/// 多行输入的纵向滚移(逻辑 px):光标行掉出可视区底部时把文本整体上推,
/// 其余不动 —— 与浏览器 textarea 同款"最小移动"
fn input_scroll_y(caret_y: f32, caret_h: f32, content_h: f32) -> f32 {
    (caret_y + caret_h - content_h).max(0.0)
}

/// 超深子树的限流告警:前三次逐条报,之后每 600 次报一次。
/// 不限流的话一棵病树能刷屏刷到看不见别的东西
fn report_too_deep(id: ViewId) {
    use std::cell::Cell;
    thread_local! {
        static HITS: Cell<u32> = const { Cell::new(0) };
    }
    let n = HITS.with(|h| {
        h.set(h.get() + 1);
        h.get()
    });
    if n <= 3 || n.is_multiple_of(600) {
        log::warn!(
            "sv-shell: 子树嵌套超过 {MAX_TREE_DEPTH} 层(节点 {id:?}),\
             该子树按叶子处理以避免栈溢出;累计 {n} 次"
        );
    }
}

/// 单行输入的横向滚移(逻辑 px):光标顶到右内边时把文本整体推左。
/// 绘制、命中、IME 上报三处共用本函数——"画的"、"点的"、"报的"同一个数
fn input_scroll_x(display: &str, px: f32, caret_byte: usize, content_w: f32) -> f32 {
    let caret = crate::text::caret_x(display, px, caret_byte);
    (caret - (content_w - 2.0)).max(0.0)
}

/// 共享绘制遍历:对任意 Painter 后端发出同一命令流。
/// 这是"可切换渲染后端"的支点(调研 14):后端只实现 Painter 三个动词
pub fn paint_tree(doc: &Doc, placed: &[Placed], painter: &mut dyn Painter, scale: f32) {
    doc.read(|inner| {
        // 裁剪栈按 clip_depth 同步(Placed 是 DFS 序,深度每步至多 +1;
        // effective rect 已含祖先交集,push 交集幂等)
        let mut clip_stack: Vec<Rect> = Vec::new();
        for p in placed {
            while clip_stack.len() > p.clip_depth as usize {
                clip_stack.pop();
                painter.pop_clip();
            }
            if (p.clip_depth as usize) > clip_stack.len()
                && let Some(c) = p.clip
            {
                clip_stack.push(c);
                painter.push_clip(c.x * scale, c.y * scale, c.w * scale, c.h * scale, 0.0);
            }
            let Some(n) = inner.nodes.get(p.id) else {
                continue;
            };
            let s = &n.style;
            let op = effective_opacity(inner, p.id);
            let bw = s.border.map(|b| b.width).unwrap_or(0.0);
            let (x, y, w, h) = (
                p.rect.x * scale,
                p.rect.y * scale,
                p.rect.w * scale,
                p.rect.h * scale,
            );
            let inset = (s.padding.left + bw) * scale;
            let inset_top = (s.padding.top + bw) * scale;

            if let Some(bg) = s.bg {
                painter.fill_rounded_rect(
                    x,
                    y,
                    w,
                    h,
                    s.corner_radius * scale,
                    with_opacity(bg, op),
                );
            }
            if let Some(b) = s.border {
                painter.stroke_rounded_rect(
                    x,
                    y,
                    w,
                    h,
                    s.corner_radius * scale,
                    b.width * scale,
                    with_opacity(b.color, op),
                );
            }

            match n.kind {
                ElementKind::Animation => {
                    // 贴在**内容盒**上(扣掉 padding 与边框),与文本同口径。
                    // 素材没接上 / 帧号越界 / 句柄失效 → 什么都不画。
                    // **刻意不画占位方块**:占位方块会让"素材没接上"看起来像
                    // "接上了但内容是灰的",而这两者查的方向完全不同
                    if let Some(a) = n.anim.as_deref() {
                        let cw = (p.rect.w - s.padding.horizontal()) * scale - bw * 2.0;
                        let ch = (p.rect.h - s.padding.vertical()) * scale - bw * 2.0;
                        match a.source {
                            sv_ui::AnimSource::Frames { .. } => {
                                if let Some(img) = crate::animation::image_for(a) {
                                    painter.draw_image(x + inset, y + inset_top, cw, ch, &img);
                                }
                            }
                            // 矢量档(Lottie):每帧现算路径,直接发到 Painter,不落位图
                            sv_ui::AnimSource::Vector { handle } => {
                                crate::animation::render_vector(
                                    handle,
                                    a.frame,
                                    (x + inset, y + inset_top, cw, ch),
                                    op,
                                    painter,
                                );
                            }
                        }
                    }
                }
                ElementKind::Text => {
                    let fg = with_opacity(resolve_fg(inner, p.id), op);
                    // 断行在逻辑坐标做,与布局(taffy measure)同源;
                    // fallback 混排 = 多字体 run(P0 载体在此兑现)
                    let fs_logical = resolve_font_size(inner, p.id);
                    let content_w_logical = p.rect.w - s.padding.horizontal() - bw * 2.0;
                    let wrap_w =
                        (s.text_wrap == sv_ui::TextWrap::Wrap).then_some(content_w_logical);
                    for run in crate::text::shape(
                        &n.text,
                        fs_logical,
                        wrap_w,
                        s.text_align,
                        x + inset,
                        y + inset_top,
                        scale,
                    ) {
                        painter.glyph_run(run.font, &run.glyphs, fg);
                    }
                }
                ElementKind::Button => {
                    let fg = with_opacity(s.fg.unwrap_or(Color::WHITE), op);
                    let fs_logical = resolve_font_size(inner, p.id);
                    let (tw_l, th_l) = crate::text::measure(&n.text, fs_logical, None);
                    let (tw, th) = (tw_l * scale, th_l * scale);
                    for run in crate::text::shape(
                        &n.text,
                        fs_logical,
                        None,
                        sv_ui::TextAlign::Left,
                        x + (w - tw) / 2.0,
                        y + (h - th) / 2.0,
                        scale,
                    ) {
                        painter.glyph_run(run.font, &run.glyphs, fg);
                    }
                }
                ElementKind::Checkbox => {
                    let boxc = with_opacity(s.bg.unwrap_or(Color::rgb(221, 221, 234)), op);
                    let r = if s.corner_radius > 0.0 {
                        s.corner_radius
                    } else {
                        4.0
                    };
                    painter.fill_rounded_rect(x, y, w, h, r * scale, boxc);
                    if n.checked {
                        let accent = with_opacity(s.fg.unwrap_or(Color::rgb(255, 62, 0)), op);
                        let ins = w * 0.25;
                        painter.fill_rounded_rect(
                            x + ins,
                            y + ins,
                            w - ins * 2.0,
                            h - ins * 2.0,
                            2.0 * scale,
                            accent,
                        );
                    }
                }
                ElementKind::TextInput => {
                    let Some(input) = n.input.as_deref() else {
                        continue;
                    };
                    let focused = inner.focused == Some(p.id);
                    // 默认底/边(style 设了 bg/border 则上面已统一画过,不重复)
                    let radius = if s.corner_radius > 0.0 {
                        s.corner_radius
                    } else {
                        4.0
                    };
                    if s.bg.is_none() {
                        painter.fill_rounded_rect(
                            x,
                            y,
                            w,
                            h,
                            radius * scale,
                            with_opacity(Color::rgb(248, 248, 252), op),
                        );
                    }
                    if s.border.is_none() {
                        painter.stroke_rounded_rect(
                            x,
                            y,
                            w,
                            h,
                            radius * scale,
                            1.0 * scale,
                            with_opacity(Color::rgb(200, 200, 212), op),
                        );
                    }

                    let content_x = x + inset;
                    let content_y = y + inset_top;
                    let content_w = w - (s.padding.horizontal() + bw * 2.0) * scale;
                    let content_h = h - (s.padding.vertical() + bw * 2.0) * scale;

                    // 显示串 = value[..cursor] + 预编辑 + value[cursor..]
                    // (仅绘制层拼接,ViewNode.text 不含半成品组合文本)
                    let value = &n.text;
                    let (display, caret_byte, preedit_range) =
                        sv_ui::input::display_text(value, input);

                    // 几何一律在**逻辑 px**求(与 taffy measure/断行同源),
                    // 画的时候再乘 scale
                    let fs_l = resolve_font_size(inner, p.id);
                    let content_w_l = p.rect.w - (s.padding.horizontal() + bw * 2.0);
                    let content_h_l = p.rect.h - (s.padding.vertical() + bw * 2.0);
                    // 多行按内容宽折行;单行不折(靠横向滚移)
                    let wrap_w = input.multiline.then_some(content_w_l);
                    let (caret_lx, caret_ly, caret_lh) =
                        crate::text::caret_rect(&display, fs_l, wrap_w, caret_byte);
                    // 光标跟随:单行推 x,多行推 y —— 都是每帧无状态算
                    let (scroll_x, scroll_y) = if input.multiline {
                        (0.0, input_scroll_y(caret_ly, caret_lh, content_h_l))
                    } else {
                        (input_scroll_x(&display, fs_l, caret_byte, content_w_l), 0.0)
                    };
                    let text_x = content_x - scroll_x * scale;
                    let text_y = content_y - scroll_y * scale;

                    painter.push_clip(content_x - scale, y, content_w + 2.0 * scale, h, 0.0);

                    // 选区高亮(组合中隐藏选区,IME 惯例)。
                    // Selection::geometry 逐行给矩形——多行选区天然分行,
                    // BiDi 混排也会分段,故是序列而不是一对 x
                    if focused && input.preedit.is_none() && input.cursor != input.anchor {
                        for (rx, ry, rw, rh) in crate::text::selection_rects_wrapped(
                            value,
                            fs_l,
                            wrap_w,
                            input.cursor.min(input.anchor),
                            input.cursor.max(input.anchor),
                        ) {
                            let (sy, sh) = if input.multiline {
                                (text_y + ry * scale, rh * scale)
                            } else {
                                (content_y, content_h)
                            };
                            painter.fill_rounded_rect(
                                text_x + rx * scale,
                                sy,
                                rw * scale,
                                sh,
                                0.0,
                                with_opacity(Color::rgba(60, 120, 255, 80), op),
                            );
                        }
                    }

                    // 文本 / placeholder(与 Text/Button 同一 TextEngine 通道:
                    // kerning、fallback 混排在输入框里同样生效)
                    let (draw, fg) = if display.is_empty() {
                        (
                            input.placeholder.as_str(),
                            with_opacity(Color::rgb(152, 152, 166), op),
                        )
                    } else {
                        (display.as_str(), with_opacity(resolve_fg(inner, p.id), op))
                    };
                    for run in crate::text::shape(
                        draw,
                        fs_l,
                        wrap_w,
                        sv_ui::TextAlign::Left,
                        text_x,
                        text_y,
                        scale,
                    ) {
                        painter.glyph_run(run.font, &run.glyphs, fg);
                    }

                    // 预编辑整段 2px 下划线(over-the-spot,候选窗是输入法自己的)。
                    // 组合串不跨行(输入法上屏前是一段),按光标所在行画
                    if let Some((lo, hi)) = preedit_range {
                        let (x0, uy, uh) = crate::text::caret_rect(&display, fs_l, wrap_w, lo);
                        let (x1, _, _) = crate::text::caret_rect(&display, fs_l, wrap_w, hi);
                        painter.fill_rounded_rect(
                            text_x + x0 * scale,
                            text_y + (uy + uh) * scale - 2.0 * scale,
                            (x1 - x0) * scale,
                            2.0 * scale,
                            0.0,
                            with_opacity(resolve_fg(inner, p.id), op),
                        );
                    }

                    // 光标竖线(仅焦点时):多行按行定位,单行占满内容高
                    if focused {
                        let (cy, ch) = if input.multiline {
                            (text_y + caret_ly * scale, caret_lh * scale)
                        } else {
                            (content_y, content_h)
                        };
                        painter.fill_rounded_rect(
                            text_x + caret_lx * scale,
                            cy,
                            (1.5 * scale).max(1.0),
                            ch,
                            0.0,
                            with_opacity(Color::rgb(255, 62, 0), op),
                        );
                    }

                    painter.pop_clip();
                }
                ElementKind::View => {}
            }
        }
        // 收尾:退出全部裁剪层(焦点环画在裁剪之外,保持始终可见)
        for _ in 0..clip_stack.len() {
            painter.pop_clip();
        }

        // 默认焦点环(调研 20:stroke 外扩 2px,宽 2px,accent 定色;
        // 画在所有节点之后 = 永远在最上层;Painter 零新动词)
        if let Some(fid) = inner.focused
            && let Some(p) = placed.iter().find(|p| p.id == fid)
        {
            let m = 2.0 * scale;
            let radius = inner
                .nodes
                .get(fid)
                .map(|n| n.style.corner_radius)
                .unwrap_or(0.0);
            painter.stroke_rounded_rect(
                p.rect.x * scale - m,
                p.rect.y * scale - m,
                p.rect.w * scale + m * 2.0,
                p.rect.h * scale + m * 2.0,
                (radius + 2.0) * scale,
                2.0 * scale,
                Color::rgb(255, 62, 0),
            );
        }
    });
}

/// 渲染一帧:布局(逻辑坐标)+ 绘制(物理坐标)。返回像素与命中测试用的布局。
/// 布局是 `Rc`:调用方多半还要再存一份(事件循环的命中测试),不该再拷一次
pub fn render_frame(doc: &Doc, phys_w: u32, phys_h: u32, scale: f32) -> (Pixmap, Rc<Layout>) {
    let logical_w = phys_w as f32 / scale;
    let logical_h = phys_h as f32 / scale;
    let layout = layout_full_cached(doc, logical_w, logical_h);

    // 分配失败(尺寸超大/内存耗尽)退化成 1×1:调用方拿到的是一帧无用像素,
    // 而不是一个崩掉的进程(R4 去 panic,调研 25 §3.4)
    let mut pixmap = match Pixmap::new(phys_w.max(1), phys_h.max(1)) {
        Some(p) => p,
        None => {
            log::warn!("sv-shell: {phys_w}×{phys_h} pixmap 分配失败,本帧退化为 1×1");
            Pixmap::new(1, 1).expect("1×1 pixmap 分配不可能失败")
        }
    };
    pixmap.fill(tiny_skia::Color::from_rgba8(255, 255, 255, 255));
    let mut painter = TinySkiaPainter::new(&mut pixmap);
    paint_tree(doc, &layout.placed, &mut painter, scale);
    paint_scrollbars(doc, &layout.scroll_areas, &mut painter, scale);

    (pixmap, layout)
}

/// 点击命中 TextInput 时:窗口逻辑坐标 → 值内字节偏移(含 padding/border 内缩)。
/// 溢出滚移与绘制层同源(`input_scroll_x` / `input_scroll_y`,私有),长文本尾部
/// 点击不再偏到左边;多行时 `ly` 决定落在第几行
pub fn input_caret_at(doc: &Doc, p: &Placed, lx: f32, ly: f32) -> usize {
    doc.read(|inner| {
        let Some(n) = inner.nodes.get(p.id) else {
            return 0;
        };
        let Some(input) = n.input.as_deref() else {
            return 0;
        };
        let s = &n.style;
        let fs = resolve_font_size(inner, p.id);
        let bw = s.border.map(|b| b.width).unwrap_or(0.0);
        let text_x = p.rect.x + s.padding.left + bw;
        let text_y = p.rect.y + s.padding.top + bw;
        let content_w = p.rect.w - (s.padding.horizontal() + bw * 2.0);
        let content_h = p.rect.h - (s.padding.vertical() + bw * 2.0);
        // 滚移按显示串算(与绘制一致),命中按值算——组合中点击本就少见,
        // 且预编辑期光标由输入法掌控
        let (display, caret_byte, _) = sv_ui::input::display_text(&n.text, input);
        let wrap_w = input.multiline.then_some(content_w);
        if input.multiline {
            let (_, cy, ch) = crate::text::caret_rect(&display, fs, wrap_w, caret_byte);
            let scroll_y = input_scroll_y(cy, ch, content_h);
            crate::text::caret_index_at_point(
                &n.text,
                fs,
                wrap_w,
                lx - text_x,
                ly - text_y + scroll_y,
            )
        } else {
            let scroll = input_scroll_x(&display, fs, caret_byte, content_w);
            crate::text::caret_index_at(&n.text, fs, lx - text_x + scroll)
        }
    })
}

/// 多行输入的上下行移动:模型层只认字节,视觉行是排版的产物,所以这一步
/// 归渲染壳。返回目标字节偏移(已在行首/行尾则原地不动)
pub fn input_caret_line_move(doc: &Doc, p: &Placed, down: bool) -> Option<usize> {
    doc.read(|inner| {
        let n = inner.nodes.get(p.id)?;
        let input = n.input.as_deref()?;
        if !input.multiline {
            return None;
        }
        let s = &n.style;
        let fs = resolve_font_size(inner, p.id);
        let bw = s.border.map(|b| b.width).unwrap_or(0.0);
        let content_w = p.rect.w - (s.padding.horizontal() + bw * 2.0);
        let wrap_w = Some(content_w);
        let (cx, cy, ch) = crate::text::caret_rect(&n.text, fs, wrap_w, input.cursor);
        // 目标点 = 同一 x、上/下一行的行中;越界时 from_point 会钳到首/末行
        let ty = if down { cy + ch * 1.5 } else { cy - ch * 0.5 };
        Some(crate::text::caret_index_at_point(
            &n.text, fs, wrap_w, cx, ty,
        ))
    })
}

/// 焦点输入框的光标矩形(物理 px;IME 候选窗定位用)。
/// 与绘制层同一 display/caret/scroll 计算——"画的"与"报的"一致
pub fn ime_caret_rect(doc: &Doc, placed: &[Placed], scale: f32) -> Option<(f32, f32, f32, f32)> {
    doc.read(|inner| {
        let id = inner.focused?;
        let n = inner.nodes.get(id)?;
        let input = n.input.as_deref()?;
        let p = placed.iter().find(|p| p.id == id)?;
        let fs = resolve_font_size(inner, id);
        let s = &n.style;
        let bw = s.border.map(|b| b.width).unwrap_or(0.0);
        let content_x = (p.rect.x + s.padding.left + bw) * scale;
        let content_y = (p.rect.y + s.padding.top + bw) * scale;
        let content_w = p.rect.w - (s.padding.horizontal() + bw * 2.0);
        let content_h = (p.rect.h - (s.padding.vertical() + bw * 2.0)) * scale;
        let (display, caret_byte, _) = sv_ui::input::display_text(&n.text, input);
        let wrap_w = input.multiline.then_some(content_w);
        let (cx, cy, ch) = crate::text::caret_rect(&display, fs, wrap_w, caret_byte);
        if input.multiline {
            let content_h_l = p.rect.h - (s.padding.vertical() + bw * 2.0);
            let scroll_y = input_scroll_y(cy, ch, content_h_l);
            return Some((
                content_x + cx * scale,
                content_y + (cy - scroll_y) * scale,
                (1.5 * scale).max(1.0),
                ch * scale,
            ));
        }
        let scroll = input_scroll_x(&display, fs, caret_byte, content_w);
        Some((
            content_x + (cx - scroll) * scale,
            content_y,
            (1.5 * scale).max(1.0),
            content_h,
        ))
    })
}

/// 命中测试(逻辑坐标),返回最上层可点击节点(视口外的子节点不可点)
pub fn hit_click_target(doc: &Doc, placed: &[Placed], x: f32, y: f32) -> Option<ViewId> {
    placed
        .iter()
        .rev()
        .find(|p| p.hit(x, y) && doc.click_handler(p.id).is_some())
        .map(|p| p.id)
}

/// 滚动条 thumb 几何(纯函数):给定轨道长/视口/内容/偏移 →
/// (thumb 起点偏移, thumb 长度);内容未溢出返回 None
pub fn scrollbar_thumb(track: f32, viewport: f32, content: f32, offset: f32) -> Option<(f32, f32)> {
    if content <= viewport || content <= 0.0 || track <= 0.0 {
        return None;
    }
    let len = (viewport / content * track).max(24.0).min(track);
    let max_off = content - viewport;
    let pos = (offset.clamp(0.0, max_off) / max_off) * (track - len);
    Some((pos, len))
}

/// 滚动条绘制:shell 合成,不入场景树(egui 同构;调研 22 §2.4)。
/// v0 纵向 thumb only(横向 API 留通道);宽 6 逻辑 px、右缘内贴 2px
pub fn paint_scrollbars(doc: &Doc, areas: &[ScrollArea], painter: &mut dyn Painter, scale: f32) {
    for a in areas {
        let (_, sy) = doc.scroll_of(a.id);
        let Some((pos, len)) = vbar_thumb(a, sy) else {
            continue;
        };
        let (bx, bw) = vbar_x(a);
        painter.fill_rounded_rect(
            bx * scale,
            (a.viewport.y + SCROLLBAR_MARGIN + pos) * scale,
            bw * scale,
            len * scale,
            bw / 2.0 * scale,
            Color::rgba(120, 120, 134, 140),
        );
    }
}

/// 滚动条几何常量(绘制与命中共用一套,拖起来才不会偏)
const SCROLLBAR_W: f32 = 6.0;
const SCROLLBAR_MARGIN: f32 = 2.0;
/// 命中容差:6px 的条太细,指针差一两像素就抓空(Fitts 定律,业界普遍加宽)
const SCROLLBAR_GRAB_PAD: f32 = 4.0;

/// 纵向滚动条的 x 与宽(逻辑 px)
fn vbar_x(a: &ScrollArea) -> (f32, f32) {
    (
        a.viewport.x + a.viewport.w - SCROLLBAR_W - SCROLLBAR_MARGIN,
        SCROLLBAR_W,
    )
}

/// 纵向 thumb 的 (轨内偏移, 长度);内容未溢出返回 None
fn vbar_thumb(a: &ScrollArea, sy: f32) -> Option<(f32, f32)> {
    let track = a.viewport.h - SCROLLBAR_MARGIN * 2.0;
    // 近似:track 按 border-box 高(视觉够用)
    scrollbar_thumb(track, a.viewport.h, a.content.1, sy)
}

/// 点(逻辑坐标)命中了哪个纵向 thumb —— 返回 (滚动容器, 指针在 thumb 内的偏移)。
/// 抓住 thumb 的**哪一点**要记住,否则拖动时 thumb 会跳到指针中心(S4)
pub fn scrollbar_grab(doc: &Doc, areas: &[ScrollArea], x: f32, y: f32) -> Option<(ViewId, f32)> {
    // 后画的在上层:与绘制顺序一致地反向找
    areas.iter().rev().find_map(|a| {
        let (_, sy) = doc.scroll_of(a.id);
        let (pos, len) = vbar_thumb(a, sy)?;
        let (bx, bw) = vbar_x(a);
        let top = a.viewport.y + SCROLLBAR_MARGIN + pos;
        let in_x = x >= bx - SCROLLBAR_GRAB_PAD && x <= bx + bw + SCROLLBAR_GRAB_PAD;
        let in_y = y >= top && y <= top + len;
        (in_x && in_y).then_some((a.id, y - top))
    })
}

/// 拖动中:指针 y → 新的纵向 offset(按轨道/内容比例反算,并钳到范围内)。
/// `grab` 是按下时记住的"指针在 thumb 内的偏移"
pub fn scrollbar_drag_offset(areas: &[ScrollArea], id: ViewId, y: f32, grab: f32) -> Option<f32> {
    let a = areas.iter().find(|a| a.id == id)?;
    let track = a.viewport.h - SCROLLBAR_MARGIN * 2.0;
    let (_, len) = scrollbar_thumb(track, a.viewport.h, a.content.1, 0.0)?;
    let travel = track - len;
    if travel <= 0.0 {
        return Some(0.0);
    }
    let top = (y - grab - (a.viewport.y + SCROLLBAR_MARGIN)).clamp(0.0, travel);
    Some((top / travel * a.max.1).clamp(0.0, a.max.1))
}

/// 滚轮路由(纯函数,离屏可测;调研 22 §2.4):命中最上层可滚容器,
/// 该方向到边界则沿父链上浮(浏览器 scroll chaining 语义)。
/// dx/dy 为期望的 offset 增量(正 = 内容向左/上移);返回消费者
pub fn route_wheel(
    doc: &Doc,
    placed: &[Placed],
    areas: &[ScrollArea],
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
) -> Option<ViewId> {
    route_wheel_with(doc, placed, areas, x, y, dx, dy, false)
}

/// 同上,`smooth=true` 时纵向走平滑滚动(S6):把目标交给动画通道逐帧逼近。
/// 横向仍是直接写 —— 触摸板横滚本身连续,再补一层缓动只会更黏
#[allow(clippy::too_many_arguments)]
pub fn route_wheel_with(
    doc: &Doc,
    placed: &[Placed],
    areas: &[ScrollArea],
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    smooth: bool,
) -> Option<ViewId> {
    let mut target = placed
        .iter()
        .rev()
        .find(|p| {
            p.hit(x, y)
                && doc.read(|inner| {
                    inner.nodes.get(p.id).is_some_and(|n| {
                        n.style.overflow == Overflow::Scroll
                            || n.style.overflow_x == Overflow::Scroll
                    })
                })
        })
        .map(|p| p.id);
    while let Some(id) = target {
        if let Some(a) = areas.iter().find(|a| a.id == id) {
            let (sx, sy) = doc.scroll_of(id);
            let nx = (sx + dx).clamp(0.0, a.max.0);
            // 平滑模式下在**进行中的目标**上累加,而不是在"这一帧画到哪儿"
            // 上累加 —— 后者会让连续快滚越滚越慢(每次都从落后的位置起算)
            let base_y = if smooth {
                sv_ui::anim::scroll_y_target(doc, id)
            } else {
                sy
            };
            let ny = (base_y + dy).clamp(0.0, a.max.1);
            if nx != sx || ny != base_y {
                if smooth && ny != base_y {
                    sv_ui::anim::scroll_y_to(doc, id, ny);
                    if nx != sx {
                        doc.set_scroll(id, nx, sy);
                    }
                } else {
                    doc.set_scroll(id, nx, ny);
                }
                return Some(id);
            }
        }
        // 到边界/无元数据:上浮找下一个可滚祖先
        target = doc.read(|inner| {
            let mut cur = inner.nodes.get(id).and_then(|n| n.parent);
            while let Some(c) = cur {
                if inner.nodes.get(c).is_some_and(|n| {
                    n.style.overflow == Overflow::Scroll || n.style.overflow_x == Overflow::Scroll
                }) {
                    return Some(c);
                }
                cur = inner.nodes.get(c).and_then(|n| n.parent);
            }
            None
        });
    }
    None
}
