//! 布局 + 绘制(CPU 自绘原型)
//!
//! 布局:行/列堆叠 + CSS 盒模型最小集(四方向 padding/margin、border、
//! 固定宽高覆盖;缺省即 border-box 语义)。TODO 换 taffy。
//! **继承**:`fg=None` / `font_size=NAN` 沿父链解析(color/font-size 白名单,
//! ADR-8 C1),根 fallback BLACK/16。measure 自顶向下携带解析值,
//! paint 对平铺列表做 O(depth) 父链回溯。
//! 绘制走 tiny-skia + swash;逻辑坐标布局、物理坐标绘制(乘 scale)。
//! 文本 shaping 为简化线性排版:charmap 逐字映射 + advance 推进(无 kerning/
//! 连字;能力与原 fontdue 持平,M2 换 Parley/HarfRust)。

use std::collections::HashMap;

use swash::FontRef;
use tiny_skia::Pixmap;

use sv_ui::{Color, Direction, Doc, DocumentInner, ElementKind, Overflow, ViewId};

use crate::font::ui_font;
use crate::paint::{GlyphKey, GlyphPos, Painter, TinySkiaPainter};

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

/// 一次布局的完整产物
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub placed: Vec<Placed>,
    pub scroll_areas: Vec<ScrollArea>,
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

/// 行度量:(基线距行顶, 行高)。font.metrics 按 px 缩放(ascent/descent/leading)
fn line_metrics(font: &FontRef, px: f32) -> (f32, f32) {
    let m = font.metrics(&[]).scale(px);
    (m.ascent, m.ascent + m.descent + m.leading)
}

pub fn measure_text(font: &FontRef, text: &str, px: f32) -> (f32, f32) {
    let (_, line_h) = line_metrics(font, px);
    if text.is_empty() {
        return (0.0, line_h);
    }
    let charmap = font.charmap();
    let gm = font.glyph_metrics(&[]).scale(px);
    let w: f32 = text.chars().map(|c| gm.advance_width(charmap.map(c))).sum();
    (w, line_h)
}

/// 折行测量(swash 线性排版 + UAX #14 断点,Slint 同款依赖;调研 23 T2。
/// **计划内报废**:M2 Parley 落地时整体换门面)。`wrap_w=None` 即单行。
/// 返回 (最宽行宽, 总高, 行字节区间);断点带 CJK 规则与标点禁则,
/// 超长不可断段(长 URL)按字符强制断
pub fn measure_text_wrapped(
    font: &FontRef,
    text: &str,
    px: f32,
    wrap_w: Option<f32>,
) -> (f32, f32, Vec<std::ops::Range<usize>>) {
    use std::cell::RefCell;
    type Cached = (f32, f32, Vec<std::ops::Range<usize>>);
    thread_local! {
        static HOT: RefCell<HashMap<(u64, u32, u32), Cached>> = RefCell::new(HashMap::new());
        static COLD: RefCell<HashMap<(u64, u32, u32), Cached>> = RefCell::new(HashMap::new());
    }
    const CAP: usize = 1024;
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        (
            h.finish(),
            px.to_bits(),
            wrap_w.map_or(u32::MAX, f32::to_bits),
        )
    };
    if let Some(hit) = HOT.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    if let Some(hit) = COLD.with(|c| c.borrow_mut().remove(&key)) {
        HOT.with(|c| c.borrow_mut().insert(key, hit.clone()));
        return hit;
    }

    let result = compute_wrapped(font, text, px, wrap_w);
    HOT.with(|c| {
        let mut hot = c.borrow_mut();
        if hot.len() >= CAP {
            // 分代淘汰(与 glyph_cache 同款):热代满则整代降冷
            let demoted = std::mem::take(&mut *hot);
            COLD.with(|cold| *cold.borrow_mut() = demoted);
        }
        hot.insert(key, result.clone());
    });
    result
}

fn compute_wrapped(
    font: &FontRef,
    text: &str,
    px: f32,
    wrap_w: Option<f32>,
) -> (f32, f32, Vec<std::ops::Range<usize>>) {
    let (_, line_h) = line_metrics(font, px);
    if text.is_empty() {
        return (0.0, line_h, Vec::from([0..0]));
    }
    let Some(wrap_w) = wrap_w else {
        let (w, h) = measure_text(font, text, px);
        return (w, h, Vec::from([0..text.len()]));
    };
    let charmap = font.charmap();
    let gm = font.glyph_metrics(&[]).scale(px);
    let advance = |c: char| gm.advance_width(charmap.map(c));
    let width_of = |r: std::ops::Range<usize>| -> f32 { text[r].chars().map(advance).sum() };

    // UAX #14:linebreaks 产出 (断点后首字节偏移, 强制/可选);
    // 相邻断点之间即"不可断段"。段尾空白**悬挂**:不参与行宽判定
    // (CSS 同款,否则 "hello " 恰好放不下会把空格挤成一行)
    let mut lines: Vec<std::ops::Range<usize>> = Vec::new();
    let mut line_start = 0usize;
    let mut pen = 0.0f32; // 含段尾空白的推进量
    let mut pen_trim = 0.0f32; // 到最后一个非空白为止的行宽
    let mut max_w = 0.0f32;
    let mut prev = 0usize;
    for (idx, op) in unicode_linebreak::linebreaks(text) {
        let seg = prev..idx;
        let seg_str = &text[seg.clone()];
        let fit_end = seg.start + seg_str.trim_end().len();
        let fit_w = width_of(seg.start..fit_end);
        if pen > 0.0 && pen + fit_w > wrap_w {
            // 段放不下 → 在段前断行
            lines.push(line_start..seg.start);
            max_w = max_w.max(pen_trim);
            line_start = seg.start;
            pen = 0.0;
        }
        if fit_w > wrap_w {
            // 超长不可断段:字符级强制断(仅非空白部分)
            for (ci, c) in text[seg.start..fit_end].char_indices() {
                let abs = seg.start + ci;
                let cw = advance(c);
                if pen > 0.0 && pen + cw > wrap_w {
                    lines.push(line_start..abs);
                    max_w = max_w.max(pen);
                    line_start = abs;
                    pen = 0.0;
                }
                pen += cw;
            }
            pen_trim = pen;
            pen += width_of(fit_end..seg.end); // 尾部空白悬挂推进
        } else {
            pen_trim = pen + fit_w;
            pen += width_of(seg.clone());
        }
        // 强制断(\n;文末的 Mandatory 不产生空行)
        if op == unicode_linebreak::BreakOpportunity::Mandatory && idx < text.len() {
            lines.push(line_start..idx);
            max_w = max_w.max(pen_trim);
            line_start = idx;
            pen = 0.0;
            pen_trim = 0.0;
        }
        prev = idx;
    }
    if line_start < text.len() || lines.is_empty() {
        lines.push(line_start..text.len());
        max_w = max_w.max(pen_trim);
    }
    (max_w, lines.len() as f32 * line_h, lines)
}

/// 折行 shaping:逐行 shape + text-align 逐行 x 偏移。
/// 断行判定在**逻辑坐标**做(与布局同源,HiDPI 下不会画/量各断各的),
/// 字形坐标按物理 px 产出
#[allow(clippy::too_many_arguments)]
fn shape_text_wrapped(
    font: crate::font::FontHandle,
    text: &str,
    px_logical: f32,
    wrap_w_logical: Option<f32>,
    align: sv_ui::TextAlign,
    ox: f32,
    oy: f32,
    box_w_phys: f32,
    scale: f32,
) -> Vec<GlyphPos> {
    let px_phys = px_logical * scale;
    let fref = font.font_ref();
    let (_, line_h_phys) = line_metrics(&fref, px_phys);
    let (_, _, lines) = measure_text_wrapped(&fref, text, px_logical, wrap_w_logical);
    let mut out = Vec::new();
    for (li, r) in lines.iter().enumerate() {
        let line = text[r.clone()].trim_end_matches(['\n', '\r']);
        let x0 = match align {
            sv_ui::TextAlign::Left => ox,
            sv_ui::TextAlign::Center => {
                ox + (box_w_phys - measure_text(&fref, line, px_phys).0) / 2.0
            }
            sv_ui::TextAlign::Right => ox + box_w_phys - measure_text(&fref, line, px_phys).0,
        };
        out.extend(shape_text(
            font,
            line,
            px_phys,
            x0,
            oy + li as f32 * line_h_phys,
        ));
    }
    out
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
    font: &FontRef,
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
            let (w, h, _) = measure_text_wrapped(font, &ctx.text, ctx.px, wrap_w);
            taffy::Size {
                width: w,
                height: h,
            }
        }
        ElementKind::Button => {
            let (w, h) = measure_text(font, &ctx.text, ctx.px);
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
        ElementKind::TextInput => {
            let (_, line_h) = line_metrics(font, ctx.px);
            taffy::Size {
                width: 200.0,
                height: line_h,
            }
        }
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
    let font = ui_font();
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
            |known, available, _id, ctx, _style| measure_leaf(&font, known, available, ctx),
        )
        .expect("sv-shell: taffy 布局失败");

        let mut out = Layout::default();
        walk_taffy(inner, &tree, &map, root, (0.0, 0.0), None, 0, &mut out);
        out
    })
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

/// shaping:文本 → 已定位字形(物理坐标)。painter 只拿 glyph run。
/// 简化线性排版:charmap 逐字映射 + advance 推进(无 kerning/连字)。
/// `oy` 是文本框顶,基线 = oy + ascent;x/y 与 ox/oy 都是基线原点
/// (CPU 端由光栅 Placement 换算位图左上角,GPU 端直接喂 draw_glyphs)
fn shape_text(
    font: crate::font::FontHandle,
    text: &str,
    px: f32,
    ox: f32,
    oy: f32,
) -> Vec<GlyphPos> {
    if text.is_empty() {
        return Vec::new();
    }
    let fref = font.font_ref();
    let (ascent, _) = line_metrics(&fref, px);
    let baseline = oy + ascent;
    let charmap = fref.charmap();
    let gm = fref.glyph_metrics(&[]).scale(px);
    let mut pen = ox;
    let mut out = Vec::new();
    for c in text.chars() {
        let id = charmap.map(c);
        let adv = gm.advance_width(id);
        // 空白字符只推进 pen,不产出字形(与原 fontdue 过滤零宽位图语义一致)
        if !c.is_whitespace() {
            out.push(GlyphPos {
                key: GlyphKey::new(font, id, px),
                x: pen,
                y: baseline,
                id,
                ox: pen,
                oy: baseline,
            });
        }
        pen += adv;
    }
    out
}

/// 光标 x 偏移(逻辑 px,相对文本起点):`byte_idx` 前所有字符的 advance 和。
/// 与 [`shape_text`] 同一 advance 逻辑——保证"画的"和"点的"一致
pub fn caret_x(font: &FontRef, text: &str, px: f32, byte_idx: usize) -> f32 {
    let charmap = font.charmap();
    let gm = font.glyph_metrics(&[]).scale(px);
    text[..byte_idx.min(text.len())]
        .chars()
        .map(|c| gm.advance_width(charmap.map(c)))
        .sum()
}

/// 点击 x 坐标(相对文本起点)→ 最近 char 边界的字节偏移(与 caret_x 互逆)
pub fn caret_index_at(font: &FontRef, text: &str, px: f32, x: f32) -> usize {
    let charmap = font.charmap();
    let gm = font.glyph_metrics(&[]).scale(px);
    let mut pen = 0.0f32;
    for (i, c) in text.char_indices() {
        let adv = gm.advance_width(charmap.map(c));
        if x < pen + adv / 2.0 {
            return i;
        }
        pen += adv;
    }
    text.len()
}

/// 共享绘制遍历:对任意 Painter 后端发出同一命令流。
/// 这是"可切换渲染后端"的支点(调研 14):后端只实现 Painter 三个动词
pub fn paint_tree(doc: &Doc, placed: &[Placed], painter: &mut dyn Painter, scale: f32) {
    let font = ui_font();
    let fh = crate::font::ui_font_handle();
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
            let fs = resolve_font_size(inner, p.id) * scale;
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
                    // 断行在逻辑坐标做,与布局(taffy measure)同源
                    let fs_logical = resolve_font_size(inner, p.id);
                    let content_w_logical = p.rect.w - s.padding.horizontal() - bw * 2.0;
                    let wrap_w =
                        (s.text_wrap == sv_ui::TextWrap::Wrap).then_some(content_w_logical);
                    let run = shape_text_wrapped(
                        fh,
                        &n.text,
                        fs_logical,
                        wrap_w,
                        s.text_align,
                        x + inset,
                        y + inset_top,
                        content_w_logical * scale,
                        scale,
                    );
                    painter.glyph_run(fh, &run, fg);
                }
                ElementKind::Button => {
                    let fg = with_opacity(s.fg.unwrap_or(Color::WHITE), op);
                    let (tw, th) = measure_text(&font, &n.text, fs);
                    let run = shape_text(fh, &n.text, fs, x + (w - tw) / 2.0, y + (h - th) / 2.0);
                    painter.glyph_run(fh, &run, fg);
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

                    // 光标跟随:每帧无状态计算横向滚移(fs 已含 scale,均物理 px)
                    let caret_px = caret_x(&font, &display, fs, caret_byte);
                    let scroll = (caret_px - (content_w - 2.0 * scale)).max(0.0);
                    let text_x = content_x - scroll;

                    painter.push_clip(content_x - scale, y, content_w + 2.0 * scale, h, 0.0);

                    // 选区高亮(组合中隐藏选区,IME 惯例)
                    if focused && input.preedit.is_none() && input.cursor != input.anchor {
                        let lo = caret_x(&font, value, fs, input.cursor.min(input.anchor));
                        let hi = caret_x(&font, value, fs, input.cursor.max(input.anchor));
                        painter.fill_rounded_rect(
                            text_x + lo,
                            content_y,
                            hi - lo,
                            content_h,
                            0.0,
                            with_opacity(Color::rgba(60, 120, 255, 80), op),
                        );
                    }

                    // 文本 / placeholder
                    if display.is_empty() {
                        if !input.placeholder.is_empty() {
                            let run = shape_text(fh, &input.placeholder, fs, text_x, content_y);
                            painter.glyph_run(
                                fh,
                                &run,
                                with_opacity(Color::rgb(152, 152, 166), op),
                            );
                        }
                    } else {
                        let fg = with_opacity(resolve_fg(inner, p.id), op);
                        let run = shape_text(fh, &display, fs, text_x, content_y);
                        painter.glyph_run(fh, &run, fg);
                    }

                    // 预编辑整段 2px 下划线(over-the-spot,候选窗是输入法自己的)
                    if let Some((lo, hi)) = preedit_range {
                        let x0 = caret_x(&font, &display, fs, lo);
                        let x1 = caret_x(&font, &display, fs, hi);
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
                            text_x + caret_px,
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
/// v0 忽略溢出滚移(光标跟随滚动是绘制层每帧无状态计算,点击场景多为未溢出)
pub fn input_caret_at(doc: &Doc, p: &Placed, lx: f32) -> usize {
    let font = ui_font();
    doc.read(|inner| {
        let Some(n) = inner.nodes.get(p.id) else {
            return 0;
        };
        let fs = resolve_font_size(inner, p.id);
        let bw = n.style.border.map(|b| b.width).unwrap_or(0.0);
        let text_x = p.rect.x + n.style.padding.left + bw;
        caret_index_at(&font, &n.text, fs, lx - text_x)
    })
}

/// 焦点输入框的光标矩形(物理 px;IME 候选窗定位用)。
/// 与绘制层同一 display/caret/scroll 计算——"画的"与"报的"一致
pub fn ime_caret_rect(doc: &Doc, placed: &[Placed], scale: f32) -> Option<(f32, f32, f32, f32)> {
    let font = ui_font();
    doc.read(|inner| {
        let id = inner.focused?;
        let n = inner.nodes.get(id)?;
        let input = n.input.as_deref()?;
        let p = placed.iter().find(|p| p.id == id)?;
        let fs = resolve_font_size(inner, id) * scale;
        let bw = n.style.border.map(|b| b.width).unwrap_or(0.0);
        let s = &n.style;
        let (x, y, w, h) = (
            p.rect.x * scale,
            p.rect.y * scale,
            p.rect.w * scale,
            p.rect.h * scale,
        );
        let content_x = x + (s.padding.left + bw) * scale;
        let content_y = y + (s.padding.top + bw) * scale;
        let content_w = w - (s.padding.horizontal() + bw * 2.0) * scale;
        let content_h = h - (s.padding.vertical() + bw * 2.0) * scale;
        let (display, caret_byte, _) = sv_ui::input::display_text(&n.text, input);
        let caret_px = caret_x(&font, &display, fs, caret_byte);
        let scroll = (caret_px - (content_w - 2.0 * scale)).max(0.0);
        Some((
            content_x - scroll + caret_px,
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
