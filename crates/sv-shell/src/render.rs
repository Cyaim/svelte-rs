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

use tiny_skia::Pixmap;

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
    /// Text 叶子是否折行(Button/Checkbox/TextInput 恒单行)
    wrap: bool,
}

/// sv-ui Style → taffy::Style 纯映射(sv-ui 不依赖 taffy 类型,
/// 与 Painter 边界同理;调研 23 §2.2 映射表)
fn to_taffy(s: &sv_ui::Style) -> taffy::Style {
    use taffy::prelude::*;
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
        overflow: if s.overflow == Overflow::Visible {
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
fn build_taffy(
    inner: &DocumentInner,
    tree: &mut taffy::TaffyTree<MeasureCtx>,
    map: &mut HashMap<u64, ViewId>,
    id: ViewId,
    inherited_font: f32,
) -> taffy::NodeId {
    let n = &inner.nodes[id];
    let fs = if n.style.font_size.is_nan() {
        inherited_font
    } else {
        n.style.font_size
    };
    let tstyle = to_taffy(&n.style);
    let node = if n.kind == ElementKind::View {
        let children: Vec<taffy::NodeId> = n
            .children
            .iter()
            .map(|c| build_taffy(inner, tree, map, *c, fs))
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
        ElementKind::TextInput => taffy::Size {
            width: 200.0,
            height: crate::text::line_height(ctx.px),
        },
        ElementKind::View => unreachable!("View 不是叶子"),
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

    let (child_clip, child_depth, scroll) = if n.style.overflow != Overflow::Visible {
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
        if n.style.overflow == Overflow::Scroll {
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
pub fn layout_full_cached(doc: &Doc, logical_w: f32, logical_h: f32) -> Layout {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Option<(usize, u64, u32, u32, Layout)>> =
            const { RefCell::new(None) };
    }
    let key = (
        doc.identity(),
        doc.version(),
        logical_w.to_bits(),
        logical_h.to_bits(),
    );
    CACHE.with(|c| {
        let mut slot = c.borrow_mut();
        if let Some((id, ver, w, h, layout)) = slot.as_ref()
            && (*id, *ver, *w, *h) == key
        {
            return layout.clone();
        }
        let layout = layout_tree_full(doc, logical_w, logical_h);
        *slot = Some((key.0, key.1, key.2, key.3, layout.clone()));
        layout
    })
}

/// 布局整棵树(完整产物:Placed + 滚动区元数据)。root 强制占满窗口逻辑尺寸。
/// 引擎 = taffy 0.12(调研 23:变更帧重建 + measure fn;disable_rounding
/// 保逻辑坐标精度,取整留给绘制端——HiDPI 下逻辑取整会放大成整物理像素跳动)
pub fn layout_tree_full(doc: &Doc, logical_w: f32, logical_h: f32) -> Layout {
    doc.read(|inner| {
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

        let mut out = Layout::default();
        walk_taffy(inner, &tree, &map, root, (0.0, 0.0), None, 0, &mut out);

        // 弹层(调研 25):基础层后追加;Popup 注册序,Tooltip 恒最后。
        // 每弹层一棵独立 taffy 树(游离子树,尺寸=内容,上限=窗口)
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
                let ol = otree.layout(oroot).expect("sv-shell: 弹层根缺布局");
                let Some(origin) = resolve_anchor(
                    e.anchor,
                    ol.size.width,
                    ol.size.height,
                    logical_w,
                    logical_h,
                    &out.placed,
                ) else {
                    continue; // 锚点节点不存在/被裁:本帧不显示
                };
                let start = out.placed.len();
                walk_taffy(inner, &otree, &omap, oroot, origin, None, 0, &mut out);
                out.overlay_regions.push(OverlayRegion {
                    root: e.root,
                    start,
                    end: out.placed.len(),
                    layer: e.layer,
                    modal: e.modal,
                    close: e.close,
                });
            }
        }
        out
    })
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

                    // 光标跟随:每帧无状态计算横向滚移。几何一律在**逻辑 px**
                    // 求(与 taffy measure/断行同源),画的时候再乘 scale
                    let fs_l = resolve_font_size(inner, p.id);
                    let content_w_l = p.rect.w - (s.padding.horizontal() + bw * 2.0);
                    let caret_l = crate::text::caret_x(&display, fs_l, caret_byte);
                    let scroll = input_scroll_x(&display, fs_l, caret_byte, content_w_l);
                    let text_x = content_x - scroll * scale;

                    painter.push_clip(content_x - scale, y, content_w + 2.0 * scale, h, 0.0);

                    // 选区高亮(组合中隐藏选区,IME 惯例)。
                    // Selection::geometry 逐行给矩形——BiDi 混排会分段,故是序列
                    if focused && input.preedit.is_none() && input.cursor != input.anchor {
                        for (rx, _, rw, _) in crate::text::selection_rects(
                            value,
                            fs_l,
                            input.cursor.min(input.anchor),
                            input.cursor.max(input.anchor),
                        ) {
                            painter.fill_rounded_rect(
                                text_x + rx * scale,
                                content_y,
                                rw * scale,
                                content_h,
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
                        None,
                        sv_ui::TextAlign::Left,
                        text_x,
                        content_y,
                        scale,
                    ) {
                        painter.glyph_run(run.font, &run.glyphs, fg);
                    }

                    // 预编辑整段 2px 下划线(over-the-spot,候选窗是输入法自己的)
                    if let Some((lo, hi)) = preedit_range {
                        let x0 = crate::text::caret_x(&display, fs_l, lo) * scale;
                        let x1 = crate::text::caret_x(&display, fs_l, hi) * scale;
                        painter.fill_rounded_rect(
                            text_x + x0,
                            content_y + content_h - 2.0 * scale,
                            x1 - x0,
                            2.0 * scale,
                            0.0,
                            with_opacity(resolve_fg(inner, p.id), op),
                        );
                    }

                    // 光标竖线(仅焦点时)
                    if focused {
                        painter.fill_rounded_rect(
                            text_x + caret_l * scale,
                            content_y,
                            (1.5 * scale).max(1.0),
                            content_h,
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

/// 渲染一帧:布局(逻辑坐标)+ 绘制(物理坐标)。返回像素与命中测试用的布局
pub fn render_frame(doc: &Doc, phys_w: u32, phys_h: u32, scale: f32) -> (Pixmap, Vec<Placed>) {
    let logical_w = phys_w as f32 / scale;
    let logical_h = phys_h as f32 / scale;
    let layout = layout_full_cached(doc, logical_w, logical_h);

    let mut pixmap = Pixmap::new(phys_w.max(1), phys_h.max(1)).expect("sv-shell: 创建 pixmap 失败");
    pixmap.fill(tiny_skia::Color::from_rgba8(255, 255, 255, 255));
    let mut painter = TinySkiaPainter::new(&mut pixmap);
    paint_tree(doc, &layout.placed, &mut painter, scale);
    paint_scrollbars(doc, &layout.scroll_areas, &mut painter, scale);

    (pixmap, layout.placed)
}

/// 点击命中 TextInput 时:窗口逻辑 x → 值内字节偏移(含 padding/border 内缩)。
/// 溢出滚移与绘制层同源([`input_scroll_x`]),长文本尾部点击不再偏到左边
pub fn input_caret_at(doc: &Doc, p: &Placed, lx: f32) -> usize {
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
        let content_w = p.rect.w - (s.padding.horizontal() + bw * 2.0);
        // 滚移按显示串算(与绘制一致),命中按值算——组合中点击本就少见,
        // 且预编辑期光标由输入法掌控
        let (display, caret_byte, _) = sv_ui::input::display_text(&n.text, input);
        let scroll = input_scroll_x(&display, fs, caret_byte, content_w);
        crate::text::caret_index_at(&n.text, fs, lx - text_x + scroll)
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
        let caret_l = crate::text::caret_x(&display, fs, caret_byte);
        let scroll = input_scroll_x(&display, fs, caret_byte, content_w);
        Some((
            content_x + (caret_l - scroll) * scale,
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
    const BAR_W: f32 = 6.0;
    const MARGIN: f32 = 2.0;
    for a in areas {
        let track = a.viewport.h - MARGIN * 2.0;
        let inner_h = a.viewport.h; // 近似:track 按 border-box 高(视觉够用)
        let (_, sy) = doc.scroll_of(a.id);
        let Some((pos, len)) = scrollbar_thumb(track, inner_h, a.content.1, sy) else {
            continue;
        };
        painter.fill_rounded_rect(
            (a.viewport.x + a.viewport.w - BAR_W - MARGIN) * scale,
            (a.viewport.y + MARGIN + pos) * scale,
            BAR_W * scale,
            len * scale,
            BAR_W / 2.0 * scale,
            Color::rgba(120, 120, 134, 140),
        );
    }
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
    let mut target = placed
        .iter()
        .rev()
        .find(|p| {
            p.hit(x, y)
                && doc.read(|inner| {
                    inner
                        .nodes
                        .get(p.id)
                        .is_some_and(|n| n.style.overflow == Overflow::Scroll)
                })
        })
        .map(|p| p.id);
    while let Some(id) = target {
        if let Some(a) = areas.iter().find(|a| a.id == id) {
            let (sx, sy) = doc.scroll_of(id);
            let nx = (sx + dx).clamp(0.0, a.max.0);
            let ny = (sy + dy).clamp(0.0, a.max.1);
            if nx != sx || ny != sy {
                doc.set_scroll(id, nx, ny);
                return Some(id);
            }
        }
        // 到边界/无元数据:上浮找下一个可滚祖先
        target = doc.read(|inner| {
            let mut cur = inner.nodes.get(id).and_then(|n| n.parent);
            while let Some(c) = cur {
                if inner
                    .nodes
                    .get(c)
                    .is_some_and(|n| n.style.overflow == Overflow::Scroll)
                {
                    return Some(c);
                }
                cur = inner.nodes.get(c).and_then(|n| n.parent);
            }
            None
        });
    }
    None
}
