//! 变更分级与脏日志 —— 让渲染壳知道"这一帧到底脏在哪一层"。
//!
//! 今天每个写方法都 `bump()` 版本号,渲染壳看到版本变了就**整棵树重新布局**。
//! 于是"改一个前景色"和"重建整张表"同价:实测 30k 树全量布局 311ms,而滚动
//! 一像素、勾一个复选框、在输入框里敲一个字,布局产物**逐字节不变**。
//!
//! 分三级(命名对着渲染壳要做的事,不是对着"什么变了"):
//!
//! | 级 | 渲染壳的动作 | 例子 |
//! |---|---|---|
//! | [`DirtyItem::Paint`] | 整份复用上一帧的布局产物 | 换色、勾选、聚焦、打字 |
//! | [`DirtyItem::Position`] | 复用布局树,只重走一遍产出坐标 | 滚动、弹层锚点 |
//! | 其余 | 重建布局树 | 文本、布局相关样式、结构 |
//!
//! # 为什么是日志,不是"每节点脏位"
//!
//! 脏位要遍历全树才能收集,量纲 O(n) —— 而 n 正是我们想摆脱的那个数。
//! 日志是 O(变更数)。
//!
//! # 为什么每条 `bump` 都必须带分级(而不是事后查表)
//!
//! 前一版方案是"写一张分级表,渲染壳按写入口查"。落地前把全仓 `bump()` 调用点
//! 数了一遍:**34 处**,而人工整理的表覆盖了 26 处 —— 漏 8 处,每漏一处就是
//! **画错一帧**且没有任何报错。所以 [`Doc::bump`](crate::Doc) 改成了必须传
//! [`DirtyItem`] 的形状:**新增一个写方法却忘了定级,是编译错误,不是线上 bug。**
//! 这比"记得查表"可靠,因为它不依赖任何人记得。

use crate::ViewId;

// ---------------------------------------------------------------------------
// **诚实说明:哪些信息今天没有消费者。**
//
// 现在的渲染壳只问日志两个问题:「要不要重建布局树」「要不要重新产出坐标」。
// 一旦要重建,它就**整棵扔掉重建** —— 于是下面这些字段今天一个都没被读:
//
// - `Structure { from, to }` 的 `from`/`to`(重建不需要知道搬到哪儿);
// - `InheritFontSize`(改字号必然同时记一条 `Measure`,重建已覆盖);
// - `Measure`/`Position`/`Structure` 里的 `id` 本身。
//
// 变异测试证实了这层冗余:把 `update_style` 的 `Measure` 拿掉、只留
// `InheritFontSize`,差分 fuzz 仍然全绿;反过来也一样。只有**两条都拿掉**才红。
//
// 留着它们不是装饰,是为了增量 `mark_dirty`(计划步骤 3):那时消费端要按
// id 精确标脏,而 `from`(reparent 的旧父)与 `InheritFontSize`(字号继承的
// 子树根)是**记录时不抓就永远拿不到**的两样东西 —— reparent 之后 Doc 里
// 只剩新父,`Doc::remove` 之后节点已从 slotmap 消失。等到那时再补,
// 意味着回头改全部 34 个 `bump` 点。
//
// 反过来说:**今天不要拿这些字段的存在当作"增量已经做了"的证据。**
// ---------------------------------------------------------------------------

/// 一条变更记录。
///
/// **刻意不含任何 taffy / 渲染概念** —— sv-ui 是编译目标,保持零渲染依赖
/// (与 `Painter` 边界同一条纪律)。这里描述的是"Doc 发生了什么",
/// 由渲染壳自己决定那意味着多少工作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirtyItem {
    /// 只影响绘制:布局产物整份复用。
    ///
    /// 收录了一条反直觉的:**在输入框里打字也是 Paint**。因为 `TextInput` 的
    /// 测量恒为 `200 × 行高×rows`,与内容无关(`render.rs` 的 `measure_leaf`)。
    /// 哪天做了 auto-size input,这一条要升级成 [`DirtyItem::Measure`] ——
    /// 分级的地方留了注释。
    Paint,
    /// 只挪位置:布局树可以复用,但产出的坐标要重算一遍(滚动、弹层锚点)。
    ///
    /// 滚动偏移根本不进 taffy —— 它是产出坐标时对子原点的一次平移。
    Position { id: ViewId },
    /// 节点自身的尺寸变了(文本 / 布局相关样式 / textarea 行数)。
    Measure { id: ViewId },
    /// 结构变了:某个父的 children 变了。
    ///
    /// `from` / `to` 都给,是因为 `Doc::append` 是 **reparent** 语义(会先把
    /// 节点从旧父摘掉),而消费端拿到日志时 Doc 里**只剩新父了** ——
    /// 旧父再也查不回来。删除同理:`Doc::remove` 之后节点已从 slotmap 消失。
    Structure {
        id: ViewId,
        from: Option<ViewId>,
        to: Option<ViewId>,
    },
    /// 继承字号要沿子树下传。
    ///
    /// 字号继承不进布局树 —— 它在建树时就地解析好了。于是改中间层的 `font_size`,
    /// 或者把一棵子树 reparent 到字号不同的新父下,子树里**所有自己没设字号的
    /// 叶子**的测量都变了,而它们自己一个字段都没动。
    /// 两条路径共用这一条;漏掉任何一条,表现都是"字变大了但行高没跟上"。
    InheritFontSize { subtree_root: ViewId },
    /// 弹层注册表变了(注册 / 注销 / 换锚点)。
    ///
    /// **它不带 `ViewId`,这正是重点**:`add_overlay` 只往注册表 push 一项,
    /// **一个节点都不脏**。若日志只能表达"某个 id 脏了",打开弹层的那一帧
    /// 日志就是空的 —— 渲染壳按"空日志 ⇒ 复用上帧"处理,**弹层永远不出现**。
    OverlayRegistry,
    /// 全局失效:字体注册表变化、DPI 变化。今天没有触发点,留着是因为
    /// 少了它就只能靠"恰好还有别的东西脏了"蒙混过去。
    InvalidateAll,
}

impl DirtyItem {
    /// 这条变更要不要重建布局树
    pub fn needs_rebuild(self) -> bool {
        match self {
            DirtyItem::Paint | DirtyItem::Position { .. } => false,
            DirtyItem::Measure { .. }
            | DirtyItem::Structure { .. }
            | DirtyItem::InheritFontSize { .. }
            | DirtyItem::OverlayRegistry
            | DirtyItem::InvalidateAll => true,
        }
    }

    /// 这条变更要不要重新产出坐标(重建布局树的一定要,只挪位置的也要)
    pub fn needs_rewalk(self) -> bool {
        !matches!(self, DirtyItem::Paint)
    }
}

/// 一帧的变更日志。渲染壳每帧 `take` 走。
#[derive(Clone, Debug, Default)]
pub struct DirtyLog {
    pub items: Vec<DirtyItem>,
    /// 变更太多,日志已丢弃 —— 消费者必须当作 [`DirtyItem::InvalidateAll`]。
    pub overflowed: bool,
}

/// 日志上限。超了就丢日志置 `overflowed`,退化成"全量重建"= 今天的行为。
///
/// 两个作用,第二个更容易被忘:
/// 1. `{#each}` 整表重建一次能推进来上万条,**记日志本身比重算还贵**;
/// 2. **没有消费者时的兜底**。`Doc` 在无渲染壳的场景下照样被用(单测、
///    离屏 PNG、直调 `layout_tree_full`),没人 `take` 日志就会一直涨。
pub const DIRTY_LOG_CAP: usize = 1024;

impl DirtyLog {
    pub fn push(&mut self, item: DirtyItem) {
        if self.overflowed {
            return;
        }
        if self.items.len() >= DIRTY_LOG_CAP {
            self.items.clear();
            self.items.shrink_to_fit();
            self.overflowed = true;
            return;
        }
        self.items.push(item);
    }

    /// 空日志 = 什么都没变。**注意 `overflowed` 时 `items` 也是空的**,
    /// 所以判"没变"必须两个都看 —— 只看 `items.is_empty()` 会把
    /// "变得太多以至于放弃记录"错读成"什么都没变",那是最坏的一种错。
    pub fn is_clean(&self) -> bool {
        self.items.is_empty() && !self.overflowed
    }

    /// 这一帧要不要重建布局树
    pub fn needs_rebuild(&self) -> bool {
        self.overflowed || self.items.iter().any(|i| i.needs_rebuild())
    }

    /// 这一帧要不要重新产出坐标
    pub fn needs_rewalk(&self) -> bool {
        self.overflowed || self.items.iter().any(|i| i.needs_rewalk())
    }
}

/// 两份样式之间的差异会不会改变布局。
///
/// **穷尽解构,不是逐字段 `!=`**:加一个 `Style` 字段就是编译错误,
/// 逼着加的人回答"它进不进布局"。漏答的后果是"改了样式但界面没动",
/// 而这类 bug 只在特定字段上才复现,测试极难覆盖。
///
/// 判据是渲染壳的 `to_taffy` 与 `measure_leaf` 到底读了哪些字段 ——
/// 注意 `border` 只有 `width` 进布局(颜色不进),`text_align` 完全不进
/// (测量恒按左对齐做,对齐是产出坐标时的事)。
pub fn layout_relevant(a: &crate::Style, b: &crate::Style) -> bool {
    // 解构两边:字段名对齐,少一个都编译不过
    let crate::Style {
        direction: a_direction,
        gap: a_gap,
        padding: a_padding,
        margin: a_margin,
        border: a_border,
        bg: _,
        fg: _,
        font_size: a_font_size,
        width: a_width,
        height: a_height,
        corner_radius: _,
        opacity: _,
        cursor: _,
        overflow: a_overflow,
        overflow_x: a_overflow_x,
        justify_content: a_justify_content,
        align_items: a_align_items,
        align_self: a_align_self,
        flex_grow: a_flex_grow,
        flex_shrink: a_flex_shrink,
        flex_wrap: a_flex_wrap,
        min_width: a_min_width,
        min_height: a_min_height,
        max_width: a_max_width,
        max_height: a_max_height,
        text_wrap: a_text_wrap,
        text_align: _,
    } = a;
    let crate::Style {
        direction: b_direction,
        gap: b_gap,
        padding: b_padding,
        margin: b_margin,
        border: b_border,
        bg: _,
        fg: _,
        font_size: b_font_size,
        width: b_width,
        height: b_height,
        corner_radius: _,
        opacity: _,
        cursor: _,
        overflow: b_overflow,
        overflow_x: b_overflow_x,
        justify_content: b_justify_content,
        align_items: b_align_items,
        align_self: b_align_self,
        flex_grow: b_flex_grow,
        flex_shrink: b_flex_shrink,
        flex_wrap: b_flex_wrap,
        min_width: b_min_width,
        min_height: b_min_height,
        max_width: b_max_width,
        max_height: b_max_height,
        text_wrap: b_text_wrap,
        text_align: _,
    } = b;

    a_direction != b_direction
        || a_gap != b_gap
        || a_padding != b_padding
        || a_margin != b_margin
        // border 只有 width 进 taffy;换个边框颜色不该重排整棵树
        || a_border.map(|x| x.width) != b_border.map(|x| x.width)
        || !font_size_eq(*a_font_size, *b_font_size)
        || a_width != b_width
        || a_height != b_height
        || a_overflow != b_overflow
        || a_overflow_x != b_overflow_x
        || a_justify_content != b_justify_content
        || a_align_items != b_align_items
        || a_align_self != b_align_self
        || a_flex_grow != b_flex_grow
        || a_flex_shrink != b_flex_shrink
        || a_flex_wrap != b_flex_wrap
        || a_min_width != b_min_width
        || a_min_height != b_min_height
        || a_max_width != b_max_width
        || a_max_height != b_max_height
        || a_text_wrap != b_text_wrap
}

/// `font_size` 的 NAN 是"继承"哨兵,不是缺失值:两边都 NAN 要算相等,
/// 否则每次 `update_style` 都判为字号变了,分级立刻失效
pub(crate) fn font_size_eq(a: f32, b: f32) -> bool {
    (a.is_nan() && b.is_nan()) || a == b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Style;

    #[test]
    fn paint_only_style_change_is_not_layout_relevant() {
        let a = Style::default();
        let mut b = a.clone();
        b.bg = Some(crate::Color::rgb(1, 2, 3));
        b.fg = Some(crate::Color::rgb(4, 5, 6));
        b.corner_radius = 8.0;
        b.opacity = 0.5;
        b.text_align = crate::TextAlign::Center;
        assert!(
            !layout_relevant(&a, &b),
            "换色/圆角/透明度/对齐都不进 taffy,不该触发重排"
        );
        assert!(a != b, "但它们确实不相等 —— 否则这条测试什么也没测");
    }

    #[test]
    fn border_color_is_paint_but_border_width_is_layout() {
        let border = |width, c| Style {
            border: Some(crate::Border {
                width,
                color: crate::Color::rgb(c, c, c),
            }),
            ..Style::default()
        };
        assert!(
            !layout_relevant(&border(2.0, 9), &border(2.0, 1)),
            "只换边框颜色不该重排"
        );
        assert!(
            layout_relevant(&border(2.0, 9), &border(3.0, 9)),
            "边框变宽会挤压内容,必须重排"
        );
        // None 与 width=0 在 taffy 里等价,但这里判为"变了"——保守方向是
        // 多排一次,不是少排一次。写下来是为了让后来人知道这是**已知**的
        assert!(layout_relevant(&Style::default(), &border(0.0, 9)));
    }

    #[test]
    fn inherit_sentinel_compares_equal() {
        let a = Style::default();
        let b = Style::default();
        assert!(a.font_size.is_nan() && b.font_size.is_nan());
        assert!(
            !layout_relevant(&a, &b),
            "两边都是继承哨兵 NAN 时必须判相等,否则分级永远退化成全量"
        );
    }

    #[test]
    fn layout_fields_are_relevant() {
        let base = Style::default();
        type Mutate = Box<dyn Fn(&mut Style)>;
        let cases: Vec<(&str, Mutate)> = vec![
            ("gap", Box::new(|s: &mut Style| s.gap = 4.0)),
            ("width", Box::new(|s: &mut Style| s.width = Some(10.0))),
            ("font_size", Box::new(|s: &mut Style| s.font_size = 20.0)),
            (
                "direction",
                Box::new(|s: &mut Style| s.direction = crate::Direction::Row),
            ),
            ("flex_grow", Box::new(|s: &mut Style| s.flex_grow = 1.0)),
            (
                "text_wrap",
                Box::new(|s: &mut Style| s.text_wrap = crate::TextWrap::NoWrap),
            ),
            (
                "padding",
                Box::new(|s: &mut Style| s.padding = crate::Edges::all(3.0)),
            ),
            (
                "max_height",
                Box::new(|s: &mut Style| s.max_height = Some(50.0)),
            ),
        ];
        for (name, f) in cases {
            let mut b = base.clone();
            f(&mut b);
            assert!(layout_relevant(&base, &b), "{name} 改了必须触发重排");
        }
    }

    #[test]
    fn overflow_cap_discards_and_flags() {
        let mut log = DirtyLog::default();
        for _ in 0..DIRTY_LOG_CAP + 10 {
            log.push(DirtyItem::Paint);
        }
        assert!(log.overflowed);
        assert!(log.items.is_empty(), "溢出后要把日志丢掉,不能继续吃内存");
        assert!(
            !log.is_clean(),
            "溢出的日志绝不能被读成'干净' —— 那会画错一整帧"
        );
        assert!(log.needs_rebuild(), "溢出必须退化成全量重建");
    }

    #[test]
    fn paint_only_log_needs_nothing() {
        let mut log = DirtyLog::default();
        log.push(DirtyItem::Paint);
        log.push(DirtyItem::Paint);
        assert!(!log.needs_rebuild());
        assert!(!log.needs_rewalk());
    }

    #[test]
    fn position_needs_rewalk_but_not_rebuild() {
        let mut log = DirtyLog::default();
        log.push(DirtyItem::Paint);
        log.push(DirtyItem::Position {
            id: crate::ViewId::default(),
        });
        assert!(!log.needs_rebuild(), "滚动不该动布局树");
        assert!(log.needs_rewalk(), "但坐标要重算");
    }

    #[test]
    fn overlay_registry_needs_rebuild_without_any_viewid() {
        // 这条是整个设计里最容易漏的:打开弹层时没有任何节点变脏
        let mut log = DirtyLog::default();
        log.push(DirtyItem::OverlayRegistry);
        assert!(log.needs_rebuild());
        assert!(!log.is_clean());
    }
}
