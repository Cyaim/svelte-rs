//! 脏矩形与 scroll-blit 计划器(CPU 呈现路径专用)。
//!
//! 输入:上一帧快照([`FrameSnapshot`])+ 本帧布局 + 本帧脏日志;
//! 输出:[`DamagePlan`] —— 要么整帧重画(`Full`),要么
//! 「先把上一帧像素按滚动位移搬一段([`Blit`]),再只重画损伤矩形」。
//!
//! # 为什么滚动要 blit,不是一般 dirty-rect
//!
//! 滚动时**整个滚动视口都在变**(内容整体位移),把视口当损伤矩形重画
//! 省不了多少;正解是把上一帧已经画好的那部分**按位移复制**,只重画新露出的
//! 一条(调研 22;Win32 `ScrollWindowEx` / 浏览器合成器同款思路)。
//! 持久 framebuffer(上一帧像素还在)是它的前提。
//!
//! # 正确性模型
//!
//! 损伤重画走「**scratch 同尺寸副本**」:把损伤矩形在 scratch 里白底重画
//! (带剔除的完整 DFS 遍历,坐标**不平移**,与整帧渲染逐字节同路),
//! 再把矩形逐行拷回 framebuffer。出血(描边越裁剪、路径不吃裁剪、圆角
//! 重切)全部落在 scratch 的矩形之外,拷回时天然丢弃 —— 不需要给画家
//! 层加任何"精确裁剪"能力。
//!
//! blit 的合法性由 [`plan`] 逐条守卫:整数物理位移、视口/裁剪未变、
//! 视口内无外来绘制(隔离扫描)、无弹层、无矢量动画、无结构脏。
//! 任何一条不满足就降级 —— 降级方向永远是**多画**(视口矩形或整帧),
//! 不是少画。逐字节等价由 `blit_render_matches_full_render_*` 差分测试守着。

use std::collections::HashMap;
use std::rc::Rc;

use sv_ui::dirty::{DirtyItem, DirtyLog};
use sv_ui::{Doc, ElementKind, ViewId};

use crate::render::{Layout, Placed, Rect, ScrollArea};

/// 剔除与损伤外扩的墨迹垫(逻辑 px):盖住焦点环(外扩 2 + 描边 2 + AA)、
/// 描边 AA、斜体字形出格。宁大勿小 —— 大了多画几个节点,小了缺一块像素
pub const INK_PAD: f32 = 8.0;

/// 隔离扫描用的墨迹垫(逻辑 px)。**方向与 INK_PAD 相同(必须 ≥ 真实出墨),
/// 但取值贴着真实上界走**:描边 AA ≤ 1、字形出格(bearing/斜体)≤ ~3。
/// 取大了会把"滚动区上方 8px 处的标题"误判成伸进视口,邻接布局永远进不了
/// blit;焦点环出墨更远(~4),但环由计划器的 focused 分支单独收损伤,
/// 不靠这里。改这个值必须让差分 fuzz(大字号/emoji 贴边)先绿
const ISOLATION_PAD: f32 = 4.0;

/// f32 几何比较容差(逻辑 px):重建/重走产生的坐标对未动节点是位级重现,
/// 只有滚动平移的补偿差(浮点非结合)需要容差
const GEOM_EPS: f32 = 0.01;

/// 「物理位移是整数」的判定容差(物理 px)
const INT_EPS: f32 = 1e-3;

/// 损伤覆盖率超过整帧的这个比例就整帧重画:矩形一多,scratch 重画 + 拷回
/// 的固定开销反超收益
const FULL_COVERAGE_RATIO: f32 = 0.7;

/// 单帧损伤矩形数上限(合并后)。超了说明这帧到处都在变,整帧更省
const MAX_RECTS: usize = 16;

// ---------------------------------------------------------------------------
// 整数物理矩形
// ---------------------------------------------------------------------------

/// 整数物理像素矩形(半开区间 [x0,x1)×[y0,y1))
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl PhysRect {
    pub const EMPTY: PhysRect = PhysRect {
        x0: 0,
        y0: 0,
        x1: 0,
        y1: 0,
    };

    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub fn w(&self) -> i32 {
        (self.x1 - self.x0).max(0)
    }

    pub fn h(&self) -> i32 {
        (self.y1 - self.y0).max(0)
    }

    pub fn area(&self) -> i64 {
        self.w() as i64 * self.h() as i64
    }

    pub fn union(&self, o: &PhysRect) -> PhysRect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        PhysRect {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }

    pub fn intersect(&self, o: &PhysRect) -> PhysRect {
        let r = PhysRect {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
        };
        if r.is_empty() { PhysRect::EMPTY } else { r }
    }

    pub fn intersects(&self, o: &PhysRect) -> bool {
        !self.intersect(o).is_empty()
    }

    /// 逻辑矩形 → 物理矩形,**向外**取整再加墨迹垫(损伤/剔除用:宁大勿小)
    pub fn outward(r: Rect, scale: f32, pad_logical: f32) -> PhysRect {
        PhysRect {
            x0: ((r.x - pad_logical) * scale).floor() as i32,
            y0: ((r.y - pad_logical) * scale).floor() as i32,
            x1: ((r.x + r.w + pad_logical) * scale).ceil() as i32,
            y1: ((r.y + r.h + pad_logical) * scale).ceil() as i32,
        }
    }

    /// 逻辑矩形 → 物理矩形,**向内**取整(blit 源/目标用:宁小勿大,
    /// 边缘吃不满整像素的半行归损伤重画管)。容差吸收 f32 噪声,
    /// 让"逻辑上恰好整像素"的边不被向内多啃一像素
    pub fn inward(r: Rect, scale: f32) -> PhysRect {
        PhysRect {
            x0: (r.x * scale - INT_EPS).ceil() as i32,
            y0: (r.y * scale - INT_EPS).ceil() as i32,
            x1: ((r.x + r.w) * scale + INT_EPS).floor() as i32,
            y1: ((r.y + r.h) * scale + INT_EPS).floor() as i32,
        }
    }

    pub fn shift(&self, dx: i32, dy: i32) -> PhysRect {
        PhysRect {
            x0: self.x0 + dx,
            y0: self.y0 + dy,
            x1: self.x1 + dx,
            y1: self.y1 + dy,
        }
    }

    pub fn clamp_to(&self, w: u32, h: u32) -> PhysRect {
        self.intersect(&PhysRect {
            x0: 0,
            y0: 0,
            x1: w as i32,
            y1: h as i32,
        })
    }
}

// ---------------------------------------------------------------------------
// 帧快照与计划
// ---------------------------------------------------------------------------

/// 上一帧(成功画进 framebuffer 的那帧)的快照:blit 位移与 placed 差分的基准。
/// framebuffer 重分配 / scale 变化 / Doc 更换时必须丢弃(见 `render_into_cached`)
pub struct FrameSnapshot {
    pub layout: Rc<Layout>,
    /// 每个滚动区**当帧生效**(上钳制后)的偏移。存储偏移可以超出 max
    /// (`set_scroll` 只下钳),画面用的是钳后值 —— blit 位移必须按钳后算,
    /// 否则贴底部继续滚会搬错位
    pub offsets: HashMap<ViewId, (f32, f32)>,
    pub scale: f32,
    pub phys_w: u32,
    pub phys_h: u32,
}

/// 滚动区当帧生效的(钳后)偏移
pub fn effective_offsets(doc: &Doc, layout: &Layout) -> HashMap<ViewId, (f32, f32)> {
    layout
        .scroll_areas
        .iter()
        .map(|a| {
            let (sx, sy) = doc.scroll_of(a.id);
            (a.id, (sx.min(a.max.0), sy.min(a.max.1)))
        })
        .collect()
}

/// 一段帧内像素搬移:`region` 内每个目标像素取 `(x+dx, y+dy)` 处的源像素
/// (源出界的部分不搬 —— 那正是"新露出的条",归损伤重画)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blit {
    pub region: PhysRect,
    pub dx: i32,
    pub dy: i32,
}

#[derive(Debug)]
pub enum DamagePlan {
    /// 整帧重画(任何守卫不过就落这里 —— 方向是多画,不是少画)
    Full,
    /// 先搬后补:`blits` 逐个执行,再把 `rects` 白底重画。
    /// 两者都空 = 这帧没有任何像素变化(纯回调注册之类),连 present 都可省
    Partial {
        blits: Vec<Blit>,
        rects: Vec<PhysRect>,
    },
}

// ---------------------------------------------------------------------------
// 墨迹范围(剔除与损伤共用同一套判断,别让两边各猜各的)
// ---------------------------------------------------------------------------

/// 节点绘制可能触及的物理范围(保守超集)。
///
/// 常规节点 = border-box 外扩 `pad`(剔除/损伤传 [`INK_PAD`],隔离扫描传
/// [`ISOLATION_PAD`])。**不折行文本与按钮**的字形可以横向越出 border-box
/// (NoWrap 溢出、按钮文本比按钮宽时居中外溢),横向按其裁剪矩形放宽;
/// 没有裁剪就只能按整帧宽算
pub fn ink_rect(
    p: &Placed,
    kind: ElementKind,
    wraps: bool,
    scale: f32,
    frame_w: i32,
    pad: f32,
) -> PhysRect {
    let mut r = PhysRect::outward(p.rect, scale, pad);
    let text_can_overflow_x =
        matches!(kind, ElementKind::Button) || (kind == ElementKind::Text && !wraps);
    if text_can_overflow_x {
        match p.clip {
            Some(c) => {
                let cp = PhysRect::outward(c, scale, pad);
                r.x0 = r.x0.min(cp.x0);
                r.x1 = r.x1.max(cp.x1);
            }
            None => {
                r.x0 = 0;
                r.x1 = frame_w;
            }
        }
    }
    r
}

fn rect_close(a: Rect, b: Rect) -> bool {
    (a.x - b.x).abs() <= GEOM_EPS
        && (a.y - b.y).abs() <= GEOM_EPS
        && (a.w - b.w).abs() <= GEOM_EPS
        && (a.h - b.h).abs() <= GEOM_EPS
}

fn clip_close(a: Option<Rect>, b: Option<Rect>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => rect_close(a, b),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// 计划器
// ---------------------------------------------------------------------------

struct ScrollDelta {
    /// 滚动区在 `cur.placed` 里的下标(其子树 = 紧随其后的 clip_depth 更深段)
    placed_idx: usize,
    subtree_end: usize,
    /// 生效偏移增量(new − old,逻辑 px)
    delta_l: (f32, f32),
}

/// 生成本帧的重画计划。`prev` 的 scale/尺寸与本帧不一致时调用方就不该进来
/// (那是整帧重画,连快照都作废)。
#[allow(clippy::too_many_arguments)]
pub fn plan(
    doc: &Doc,
    prev: &FrameSnapshot,
    cur: &Layout,
    dirty: &DirtyLog,
    scale: f32,
    phys_w: u32,
    phys_h: u32,
    caret_flip: bool,
) -> DamagePlan {
    // ---- 结构级守卫:这些形态本帧无法局部化,整帧最稳 ----
    if dirty.overflowed {
        return DamagePlan::Full;
    }
    for item in &dirty.items {
        match item {
            DirtyItem::Structure { .. }
            | DirtyItem::InheritFontSize { .. }
            | DirtyItem::OverlayRegistry
            | DirtyItem::InvalidateAll => return DamagePlan::Full,
            DirtyItem::Paint { .. } | DirtyItem::Position { .. } | DirtyItem::Measure { .. } => {}
        }
    }
    // 弹层锚着滚动内容会跟着动、且叠画在同一缓冲里 —— v1 一律整帧
    if !cur.overlay_regions.is_empty() || !prev.layout.overlay_regions.is_empty() {
        return DamagePlan::Full;
    }

    let has_measure = dirty
        .items
        .iter()
        .any(|i| matches!(i, DirtyItem::Measure { .. }));

    doc.read(|inner| {
        // 矢量动画(Lottie)的 fill_path 不吃裁剪也不受 border-box 约束,
        // 墨迹范围无从保守估计 —— 出现即整帧
        for p in &cur.placed {
            if let Some(n) = inner.nodes.get(p.id)
                && n.kind == ElementKind::Animation
                && n.anim
                    .as_deref()
                    .is_some_and(|a| matches!(a.source, sv_ui::AnimSource::Vector { .. }))
            {
                return DamagePlan::Full;
            }
        }

        let mut cur_by_id: HashMap<ViewId, usize> = HashMap::with_capacity(cur.placed.len());
        for (i, p) in cur.placed.iter().enumerate() {
            cur_by_id.insert(p.id, i);
        }
        let mut prev_by_id: HashMap<ViewId, usize> =
            HashMap::with_capacity(prev.layout.placed.len());
        for (i, p) in prev.layout.placed.iter().enumerate() {
            prev_by_id.insert(p.id, i);
        }

        let ink_of = |layout: &Layout, idx: usize| -> PhysRect {
            let p = &layout.placed[idx];
            let (kind, wraps) = inner
                .nodes
                .get(p.id)
                .map(|n| (n.kind, n.style.text_wrap == sv_ui::TextWrap::Wrap))
                .unwrap_or((ElementKind::View, false));
            ink_rect(p, kind, wraps, scale, phys_w as i32, INK_PAD)
        };

        let mut rects: Vec<PhysRect> = Vec::new();
        let mut blits: Vec<Blit> = Vec::new();
        let mut full = false;
        let push_rect = |rects: &mut Vec<PhysRect>, r: PhysRect| {
            let r = r.clamp_to(phys_w, phys_h);
            if !r.is_empty() {
                rects.push(r);
            }
        };

        // ---- 滚动:按生效偏移差找出真滚了的区 ----
        let cur_offsets = effective_offsets(doc, cur);
        let mut scrolled: Vec<ScrollDelta> = Vec::new();
        for area in &cur.scroll_areas {
            let cur_off = cur_offsets[&area.id];
            let Some(&prev_off) = prev.offsets.get(&area.id) else {
                // 新出现的滚动区(样式切换出来的):它那块全算损伤
                if let Some(&i) = cur_by_id.get(&area.id) {
                    push_rect(&mut rects, ink_of(cur, i));
                }
                continue;
            };
            let prev_area = prev.layout.scroll_areas.iter().find(|a| a.id == area.id);
            let content_same = prev_area.is_some_and(|pa| {
                (pa.content.0 - area.content.0).abs() <= GEOM_EPS
                    && (pa.content.1 - area.content.1).abs() <= GEOM_EPS
                    && rect_close(pa.viewport, area.viewport)
            });
            let delta_l = (cur_off.0 - prev_off.0, cur_off.1 - prev_off.1);
            if delta_l.0.abs() <= f32::EPSILON && delta_l.1.abs() <= f32::EPSILON && content_same {
                continue; // 没滚(Position 可能只是 scroll_y_to 的起步 bump)
            }
            let (Some(&pi), Some(&ci)) = (prev_by_id.get(&area.id), cur_by_id.get(&area.id)) else {
                full = true;
                break;
            };
            // 视口自身没动、裁剪上下文没变、内容尺寸没变,才谈得上搬像素
            let stable = content_same
                && rect_close(prev.layout.placed[pi].rect, cur.placed[ci].rect)
                && clip_close(prev.layout.placed[pi].clip, cur.placed[ci].clip);
            if !stable {
                push_rect(&mut rects, ink_of(cur, ci).union(&ink_of(&prev.layout, pi)));
                continue;
            }
            let subtree_end = subtree_end(cur, ci);
            scrolled.push(ScrollDelta {
                placed_idx: ci,
                subtree_end,
                delta_l,
            });
        }
        if full {
            return DamagePlan::Full;
        }
        // 消失的滚动区:原来那块全算损伤
        for pa in &prev.layout.scroll_areas {
            if !cur.scroll_areas.iter().any(|a| a.id == pa.id)
                && let Some(&pi) = prev_by_id.get(&pa.id)
            {
                push_rect(&mut rects, ink_of(&prev.layout, pi));
            }
        }

        // ---- 逐滚动区决定 blit 还是整视口重画 ----
        for s in &scrolled {
            let p = &cur.placed[s.placed_idx];
            let node = &inner.nodes[p.id];
            let dxp = s.delta_l.0 * scale;
            let dyp = s.delta_l.1 * scale;
            let (dxi, dyi) = (dxp.round(), dyp.round());
            let integral = (dxp - dxi).abs() <= INT_EPS && (dyp - dyi).abs() <= INT_EPS;

            // blit 源/目标区:border-box 按 max(边框宽, 圆角) 内缩(边框线与
            // 圆角弧不随内容动,搬了就成串线),再与祖先裁剪求交、向内取整
            let bw = node.style.border.map(|b| b.width).unwrap_or(0.0);
            let inset = bw.max(node.style.corner_radius);
            let vp = p.rect;
            let inner_rect = Rect {
                x: vp.x + inset,
                y: vp.y + inset,
                w: (vp.w - inset * 2.0).max(0.0),
                h: (vp.h - inset * 2.0).max(0.0),
            };
            let mut region = PhysRect::inward(inner_rect, scale);
            if let Some(c) = p.clip {
                region = region.intersect(&PhysRect::inward(c, scale));
            }
            region = region.clamp_to(phys_w, phys_h);

            let viewport_ink = ink_of(cur, s.placed_idx);
            let big_jump =
                dxi.abs() as i64 >= region.w() as i64 || dyi.abs() as i64 >= region.h() as i64;

            let eligible = integral
                && !big_jump
                && !region.is_empty()
                && (dxi != 0.0 || dyi != 0.0)
                && blit_region_isolated(inner, cur, s, &region, scale, phys_w as i32);

            if !eligible {
                // 整视口重画(含滚动条列、含焦点环出界部分 —— ink 垫已盖住)
                push_rect(&mut rects, viewport_ink);
                continue;
            }
            let (dxi, dyi) = (dxi as i32, dyi as i32);
            blits.push(Blit {
                region,
                dx: dxi,
                dy: dyi,
            });
            // 新露出的条 = 目标像素的源(x+dx, y+dy)落在 region 外的部分
            for strip in exposed_strips(&region, dxi, dyi) {
                push_rect(&mut rects, strip);
            }
            // 滚动条 thumb 每帧都动且半透明(叠着搬会双重变深):
            // 轨道列一律重画;横向 blit 还会把旧 thumb 搬进内容区,
            // 把"搬到哪儿"的那列也划进损伤
            let track = track_column(&cur.scroll_areas, p.id, scale);
            push_rect(&mut rects, track);
            if dxi != 0 {
                push_rect(&mut rects, track.shift(-dxi, 0));
            }
            // 视口边缘的半像素缝(向内取整丢掉的):上下左右各一条 1px 补画
            let outer = PhysRect::outward(vp, scale, 0.0)
                .intersect(&viewport_ink)
                .clamp_to(phys_w, phys_h);
            for edge in frame_ring(&outer, &region) {
                push_rect(&mut rects, edge);
            }
        }

        // blit 区彼此相交(嵌套同滚 / 并排同滚且区域重叠)→ 搬移顺序相互污染,整帧
        for i in 0..blits.len() {
            for j in (i + 1)..blits.len() {
                if blits[i].region.intersects(&blits[j].region) {
                    return DamagePlan::Full;
                }
            }
        }

        // ---- 逐条局部损伤(Paint/Measure 带 id;Position 已由滚动差分覆盖)----
        for item in &dirty.items {
            let id = match item {
                DirtyItem::Paint { id } | DirtyItem::Measure { id } => *id,
                DirtyItem::Position { id } => {
                    // 不是滚动区的 Position(裸 content_override 之类):吃不准,整帧
                    if !cur.scroll_areas.iter().any(|a| a.id == *id)
                        && !prev.layout.scroll_areas.iter().any(|a| a.id == *id)
                    {
                        return DamagePlan::Full;
                    }
                    continue;
                }
                _ => continue,
            };
            if let Some(&ci) = cur_by_id.get(&id) {
                push_rect(&mut rects, ink_of(cur, ci));
            }
            if let Some(&pi) = prev_by_id.get(&id) {
                // 旧位置的旧墨迹;若旧位置落在本帧某个 blit 区里,它的像素
                // 已被搬到 -delta 处,把搬到的位置也划进损伤
                let ink = ink_of(&prev.layout, pi);
                push_rect(&mut rects, ink);
                for b in &blits {
                    if ink.intersects(&b.region) {
                        push_rect(&mut rects, ink.shift(-b.dx, -b.dy));
                    }
                }
            }
        }

        // ---- Measure 帧:重排可能挪动任何兄弟/祖先,placed 差分兜底 ----
        if has_measure {
            if cur.placed.len() != prev.layout.placed.len() {
                // Measure 不增删节点;数量变了说明有没记账的结构变化,整帧
                return DamagePlan::Full;
            }
            for (ci, p) in cur.placed.iter().enumerate() {
                let Some(&pi) = prev_by_id.get(&p.id) else {
                    return DamagePlan::Full;
                };
                let pp = &prev.layout.placed[pi];
                // 期望的"没变":本帧位置 + 所在滚动子树的平移补偿 == 上帧位置
                let (shx, shy) = scroll_shift_at(&scrolled, ci);
                let expected_prev = Rect {
                    x: p.rect.x + shx,
                    y: p.rect.y + shy,
                    w: p.rect.w,
                    h: p.rect.h,
                };
                if rect_close(expected_prev, pp.rect) && clip_close(p.clip, pp.clip) {
                    continue;
                }
                push_rect(&mut rects, ink_of(cur, ci));
                let ink = ink_of(&prev.layout, pi);
                push_rect(&mut rects, ink);
                for b in &blits {
                    if ink.intersects(&b.region) {
                        push_rect(&mut rects, ink.shift(-b.dx, -b.dy));
                    }
                }
            }
        }

        // ---- 焦点环 / 光标闪烁 ----
        let focused = inner.focused;
        if let Some(fid) = focused {
            let cur_ink = cur_by_id.get(&fid).map(|&i| ink_of(cur, i));
            let prev_ink = prev_by_id.get(&fid).map(|&i| ink_of(&prev.layout, i));
            let moved = match (cur_ink, prev_ink) {
                (Some(a), Some(b)) => a != b,
                (None, None) => false,
                _ => true,
            };
            let in_blit = |r: Option<PhysRect>| {
                r.is_some_and(|r| blits.iter().any(|b| r.intersects(&b.region)))
            };
            if caret_flip || moved || in_blit(cur_ink) || in_blit(prev_ink) {
                if let Some(r) = cur_ink {
                    push_rect(&mut rects, r);
                }
                if let Some(r) = prev_ink {
                    push_rect(&mut rects, r);
                }
            }
        } else if caret_flip {
            // 没有焦点却报闪烁相翻转:异常态,别赌,整帧
            return DamagePlan::Full;
        }

        // ---- 合并与覆盖率闸门 ----
        let rects = merge_rects(rects);
        let covered: i64 = rects.iter().map(|r| r.area()).sum();
        let frame_area = phys_w as i64 * phys_h as i64;
        if rects.len() > MAX_RECTS
            || (frame_area > 0 && covered as f32 / frame_area as f32 > FULL_COVERAGE_RATIO)
        {
            return DamagePlan::Full;
        }
        DamagePlan::Partial { blits, rects }
    })
}

/// `placed[idx]` 的子树在 placed 里的终点(不含):滚动容器必裁剪,
/// 后代 clip_depth 严格更深且连续(DFS 序)
fn subtree_end(layout: &Layout, idx: usize) -> usize {
    let d = layout.placed[idx].clip_depth;
    let mut j = idx + 1;
    while j < layout.placed.len() && layout.placed[j].clip_depth > d {
        j += 1;
    }
    j
}

/// `placed[i]` 相对上一帧的滚动平移补偿(逻辑 px;嵌套滚动叠加)
fn scroll_shift_at(scrolled: &[ScrollDelta], i: usize) -> (f32, f32) {
    let mut sh = (0.0f32, 0.0f32);
    for s in scrolled {
        if i > s.placed_idx && i < s.subtree_end {
            sh.0 += s.delta_l.0;
            sh.1 += s.delta_l.1;
        }
    }
    sh
}

/// 隔离扫描:blit 区内的每个像素来源必须「随内容一起动」或「平移不变」。
/// 逐 placed 检查与 region 相交的节点:
/// - 滚动子树内(含嵌套):随内容动 ✓
/// - 画在滚动容器**之前**、且在 region 上是均匀色块(View 的纯 bg,矩形
///   完全盖住 region、圆角弧与边框环都碰不到 region):平移不变 ✓
/// - 其余(之后画的兄弟、region 里探进来的文本/控件、被裁得只剩一角的
///   底色卡片):搬了必错 → 不许 blit
fn blit_region_isolated(
    inner: &sv_ui::DocumentInner,
    cur: &Layout,
    s: &ScrollDelta,
    region: &PhysRect,
    scale: f32,
    frame_w: i32,
) -> bool {
    for (i, p) in cur.placed.iter().enumerate() {
        if i > s.placed_idx && i < s.subtree_end {
            continue; // 滚动子树自身
        }
        let Some(n) = inner.nodes.get(p.id) else {
            continue;
        };
        let wraps = n.style.text_wrap == sv_ui::TextWrap::Wrap;
        let ink = ink_rect(p, n.kind, wraps, scale, frame_w, ISOLATION_PAD);
        if !ink.intersects(region) {
            continue;
        }
        if i > s.placed_idx {
            // 画在滚动内容之后又叠进 region:blit 会把它的旧像素错搬
            return false;
        }
        // 画在之前:必须在 region 范围内是均匀色块
        if n.kind != ElementKind::View {
            return false;
        }
        // 无 bg 无边框的 View 什么都不画 ✓
        let paints_nothing = n.style.bg.is_none() && n.style.border.is_none();
        if paints_nothing {
            continue;
        }
        // 有 bg:矩形要完全盖住 region,且圆角弧 / 边框环碰不到 region
        let inset = n
            .style
            .border
            .map(|b| b.width)
            .unwrap_or(0.0)
            .max(n.style.corner_radius);
        let safe_core = PhysRect::inward(
            Rect {
                x: p.rect.x + inset,
                y: p.rect.y + inset,
                w: (p.rect.w - inset * 2.0).max(0.0),
                h: (p.rect.h - inset * 2.0).max(0.0),
            },
            scale,
        );
        // region 必须整个落在均匀核内(边缘 AA 半像素也算不均匀,inward 已含)
        if region.intersect(&safe_core) != *region {
            return false;
        }
    }
    true
}

/// blit 后新露出的条(目标像素的源落在 region 外):最多两条 —— 纵向一整幅、
/// 横向剩余行的一列
pub fn exposed_strips(region: &PhysRect, dx: i32, dy: i32) -> Vec<PhysRect> {
    let mut out = Vec::new();
    let mut remaining = *region;
    if dy > 0 {
        // 内容上移,底部露出
        out.push(PhysRect {
            y0: (region.y1 - dy).max(region.y0),
            ..*region
        });
        remaining.y1 = (region.y1 - dy).max(region.y0);
    } else if dy < 0 {
        out.push(PhysRect {
            y1: (region.y0 - dy).min(region.y1),
            ..*region
        });
        remaining.y0 = (region.y0 - dy).min(region.y1);
    }
    if dx > 0 {
        out.push(PhysRect {
            x0: (region.x1 - dx).max(region.x0),
            ..remaining
        });
    } else if dx < 0 {
        out.push(PhysRect {
            x1: (region.x0 - dx).min(region.x1),
            ..remaining
        });
    }
    out.retain(|r| !r.is_empty());
    out
}

/// 滚动条轨道列(物理):thumb 只会出现在这一列里(纵向 v0)。
/// 取 GRAB_PAD 同款余量,把 AA 边也含进去
fn track_column(areas: &[ScrollArea], id: ViewId, scale: f32) -> PhysRect {
    let Some(a) = areas.iter().find(|a| a.id == id) else {
        return PhysRect::EMPTY;
    };
    let (bx, bw) = crate::render::vbar_geometry(a);
    PhysRect::outward(
        Rect {
            x: bx - 1.0,
            y: a.viewport.y,
            w: bw + 2.0,
            h: a.viewport.h,
        },
        scale,
        1.0,
    )
}

/// `outer` 相对 `inner` 的四条边框条(inner ⊆ outer;空条丢弃)。
/// blit 区向内取整后,视口边缘可能留下不足 1px 的缝,这四条补上
fn frame_ring(outer: &PhysRect, inner: &PhysRect) -> Vec<PhysRect> {
    if inner.is_empty() {
        return vec![*outer];
    }
    let mut v = Vec::with_capacity(4);
    v.push(PhysRect {
        y1: inner.y0,
        ..*outer
    }); // 上
    v.push(PhysRect {
        y0: inner.y1,
        ..*outer
    }); // 下
    v.push(PhysRect {
        x1: inner.x0,
        y0: inner.y0,
        y1: inner.y1,
        ..*outer
    }); // 左
    v.push(PhysRect {
        x0: inner.x1,
        y0: inner.y0,
        y1: inner.y1,
        ..*outer
    }); // 右
    v.retain(|r| !r.is_empty());
    v
}

/// 合并重叠/相邻的损伤矩形(O(k²) 固定点;k 有 MAX_RECTS 闸,合并激进一点:
/// 相交或并集面积不超过两者之和 1.2 倍就并 —— 碎矩形对 scratch 重画是纯开销)
fn merge_rects(mut rects: Vec<PhysRect>) -> Vec<PhysRect> {
    rects.retain(|r| !r.is_empty());
    loop {
        let mut merged_any = false;
        let mut i = 0;
        while i < rects.len() {
            let mut j = i + 1;
            while j < rects.len() {
                let a = rects[i];
                let b = rects[j];
                let u = a.union(&b);
                let should = a.intersects(&b) || u.area() <= (a.area() + b.area()) * 6 / 5;
                if should {
                    rects[i] = u;
                    rects.swap_remove(j);
                    merged_any = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        if !merged_any {
            return rects;
        }
    }
}

// ---------------------------------------------------------------------------
// blit 执行(同一 pixmap 内的重叠安全搬移)
// ---------------------------------------------------------------------------

/// 在 pixmap 内执行一段搬移。行序按 dy 方向选(内容上移=自上而下读后面的行,
/// 内容下移=自下而上),行内用 `copy_within`(memmove 语义)—— 两个方向的
/// 重叠都安全
pub fn apply_blit(pixmap: &mut tiny_skia::Pixmap, b: &Blit) {
    let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);
    let r = b.region.clamp_to(pw as u32, ph as u32);
    if r.is_empty() {
        return;
    }
    // 目标行/列范围:源 (x+dx, y+dy) 必须仍在 region 内
    let dy0 = r.y0.max(r.y0 - b.dy);
    let dy1 = r.y1.min(r.y1 - b.dy);
    let dx0 = r.x0.max(r.x0 - b.dx);
    let dx1 = r.x1.min(r.x1 - b.dx);
    if dy0 >= dy1 || dx0 >= dx1 {
        return;
    }
    let stride = pw as usize * 4;
    let data = pixmap.data_mut();
    let row_bytes = (dx1 - dx0) as usize * 4;
    let copy_row = |data: &mut [u8], dst_y: i32| {
        let src_y = dst_y + b.dy;
        let src = src_y as usize * stride + (dx0 + b.dx) as usize * 4;
        let dst = dst_y as usize * stride + dx0 as usize * 4;
        data.copy_within(src..src + row_bytes, dst);
    };
    if b.dy >= 0 {
        for y in dy0..dy1 {
            copy_row(data, y);
        }
    } else {
        for y in (dy0..dy1).rev() {
            copy_row(data, y);
        }
    }
}

/// 把 scratch 里的矩形逐行拷回 framebuffer(替换语义 —— scratch 里是
/// 白底完整重画的最终合成结果)
pub fn copy_rect(dst: &mut tiny_skia::Pixmap, src: &tiny_skia::Pixmap, r: &PhysRect) {
    debug_assert_eq!(dst.width(), src.width());
    debug_assert_eq!(dst.height(), src.height());
    let r = r.clamp_to(dst.width(), dst.height());
    if r.is_empty() {
        return;
    }
    let stride = dst.width() as usize * 4;
    let row_bytes = r.w() as usize * 4;
    let sdata = src.data();
    let ddata = dst.data_mut();
    for y in r.y0..r.y1 {
        let off = y as usize * stride + r.x0 as usize * 4;
        ddata[off..off + row_bytes].copy_from_slice(&sdata[off..off + row_bytes]);
    }
}

/// 在 pixmap 上把矩形填成不透明白(损伤重画的底色,与整帧清屏同色)
pub fn fill_rect_white(pixmap: &mut tiny_skia::Pixmap, r: &PhysRect) {
    let r = r.clamp_to(pixmap.width(), pixmap.height());
    if r.is_empty() {
        return;
    }
    let stride = pixmap.width() as usize * 4;
    let row_bytes = r.w() as usize * 4;
    let data = pixmap.data_mut();
    for y in r.y0..r.y1 {
        let off = y as usize * stride + r.x0 as usize * 4;
        for b in &mut data[off..off + row_bytes] {
            *b = 0xff;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x0: i32, y0: i32, x1: i32, y1: i32) -> PhysRect {
        PhysRect { x0, y0, x1, y1 }
    }

    #[test]
    fn phys_rect_rounding_directions() {
        let lr = Rect {
            x: 1.2,
            y: 2.0,
            w: 10.0,
            h: 5.6,
        };
        let out = PhysRect::outward(lr, 1.0, 0.0);
        assert_eq!(out, r(1, 2, 12, 8), "向外:floor 起点、ceil 终点");
        let inw = PhysRect::inward(lr, 1.0);
        assert_eq!(inw, r(2, 2, 11, 7), "向内:ceil 起点、floor 终点");
        // 容差:恰好整数的边不能被向内啃掉一像素
        let exact = Rect {
            x: 3.0,
            y: 4.0,
            w: 7.0,
            h: 2.0,
        };
        assert_eq!(PhysRect::inward(exact, 2.0), r(6, 8, 20, 12));
    }

    #[test]
    fn exposed_strips_cover_exactly_the_unsourced_part() {
        let region = r(10, 10, 110, 210);
        // 向下滚 30:底部露 30 行
        let s = exposed_strips(&region, 0, 30);
        assert_eq!(s, vec![r(10, 180, 110, 210)]);
        // 向上滚 30:顶部露 30 行
        let s = exposed_strips(&region, 0, -30);
        assert_eq!(s, vec![r(10, 10, 110, 40)]);
        // 斜向:纵条 + 横条不重叠且并起来 = region \ 搬到的部分
        let s = exposed_strips(&region, 20, 30);
        let total: i64 = s.iter().map(|x| x.area()).sum();
        let moved = (region.w() - 20) as i64 * (region.h() - 30) as i64;
        assert_eq!(total, region.area() - moved);
        for i in 0..s.len() {
            for j in (i + 1)..s.len() {
                assert!(!s[i].intersects(&s[j]), "露出条彼此不重叠");
            }
        }
        // 不动:没有露出条
        assert!(exposed_strips(&region, 0, 0).is_empty());
    }

    #[test]
    fn blit_moves_pixels_with_overlap_safety_both_directions() {
        // 8×8 渐变图,region 中央 6×6,上下两个方向各搬 2,逐像素对拍手算结果
        let mk = || {
            let mut p = tiny_skia::Pixmap::new(8, 8).unwrap();
            let d = p.data_mut();
            for y in 0..8u32 {
                for x in 0..8u32 {
                    let i = ((y * 8 + x) * 4) as usize;
                    d[i] = x as u8 * 10;
                    d[i + 1] = y as u8 * 10;
                    d[i + 2] = 0;
                    d[i + 3] = 255;
                }
            }
            p
        };
        for (dx, dy) in [(0, 2), (0, -2), (2, 0), (-2, 0), (2, 2), (-2, -2)] {
            let mut p = mk();
            let orig = mk();
            apply_blit(
                &mut p,
                &Blit {
                    region: r(1, 1, 7, 7),
                    dx,
                    dy,
                },
            );
            let stride = 8 * 4usize;
            for y in 1..7i32 {
                for x in 1..7i32 {
                    let (sx, sy) = (x + dx, y + dy);
                    let di = y as usize * stride + x as usize * 4;
                    if (1..7).contains(&sx) && (1..7).contains(&sy) {
                        let si = sy as usize * stride + sx as usize * 4;
                        assert_eq!(
                            &p.data()[di..di + 4],
                            &orig.data()[si..si + 4],
                            "({x},{y}) 应来自 ({sx},{sy}),delta=({dx},{dy})"
                        );
                    } else {
                        assert_eq!(
                            &p.data()[di..di + 4],
                            &orig.data()[di..di + 4],
                            "源出界的目标像素应保持原样(归损伤重画),delta=({dx},{dy})"
                        );
                    }
                }
            }
            // region 外一个字节都不许动
            for y in 0..8i32 {
                for x in 0..8i32 {
                    if (1..7).contains(&x) && (1..7).contains(&y) {
                        continue;
                    }
                    let i = y as usize * stride + x as usize * 4;
                    assert_eq!(
                        &p.data()[i..i + 4],
                        &orig.data()[i..i + 4],
                        "region 外不许动"
                    );
                }
            }
        }
    }

    #[test]
    fn copy_and_fill_touch_only_the_rect() {
        let mut dst = tiny_skia::Pixmap::new(6, 6).unwrap();
        let mut src = tiny_skia::Pixmap::new(6, 6).unwrap();
        src.fill(tiny_skia::Color::from_rgba8(10, 20, 30, 255));
        let rect = r(2, 3, 5, 5);
        copy_rect(&mut dst, &src, &rect);
        let stride = 6 * 4usize;
        for y in 0..6i32 {
            for x in 0..6i32 {
                let i = y as usize * stride + x as usize * 4;
                let inside = (2..5).contains(&x) && (3..5).contains(&y);
                let px = &dst.data()[i..i + 4];
                if inside {
                    assert_eq!(px, &src.data()[i..i + 4]);
                } else {
                    assert_eq!(px, &[0, 0, 0, 0], "矩形外必须保持透明零");
                }
            }
        }
        fill_rect_white(&mut dst, &r(0, 0, 1, 1));
        assert_eq!(&dst.data()[0..4], &[255, 255, 255, 255]);
        assert_eq!(&dst.data()[4..8], &[0, 0, 0, 0]);
    }

    #[test]
    fn merge_rects_folds_overlaps_and_keeps_distant_apart() {
        let merged = merge_rects(vec![r(0, 0, 10, 10), r(5, 5, 15, 15)]);
        assert_eq!(merged, vec![r(0, 0, 15, 15)]);
        let kept = merge_rects(vec![r(0, 0, 10, 10), r(500, 500, 510, 510)]);
        assert_eq!(kept.len(), 2, "远矩形不该被并成巨块");
        assert!(merge_rects(vec![PhysRect::EMPTY]).is_empty());
    }

    #[test]
    fn frame_ring_covers_outer_minus_inner() {
        let outer = r(0, 0, 100, 100);
        let inner = r(10, 20, 90, 80);
        let ring = frame_ring(&outer, &inner);
        let total: i64 = ring.iter().map(|x| x.area()).sum();
        assert_eq!(total, outer.area() - inner.area());
        for i in 0..ring.len() {
            for j in (i + 1)..ring.len() {
                assert!(!ring[i].intersects(&ring[j]));
            }
        }
    }
}
