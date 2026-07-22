//! 模板数据面(ADR-2 无悔三步 ②:"生成数据而非生成类型";调研 09 §5.2/5.3)
//!
//! 一个 `.sv` 编成两面:
//!
//! ```text
//! counter.sv ──sv-compiler──┬── 代码面(经 rustc):script 体 + 表达式槽位闭包表(binders)
//!                           └── 数据面(不经 rustc):[`Template`] 结构表 + 静态样式声明
//! ```
//!
//! 本模块是**数据面的类型与解释器**。[`stamp`] 拿"结构数据 + 槽位闭包表"建出场景树,
//! 与 codegen 现在直接发射的命令式代码**语义逐字等价**(`stamp_matches_imperative`
//! 测试逐字节对拍 `dump()` 钉死这条契约)——这是将来 codegen 切过去的靶子。
//!
//! # 为什么值得把结构与样式变成数据
//!
//! 热重载的天花板由"哪些改动能不经 rustc"决定(调研 09 §5.1:Dioxus 的模板数据 diff
//! 是同一条路)。**结构增删改、静态文本、静态样式**是日常编辑里绝大多数,把它们放进
//! 数据面就能毫秒级替换;而**新表达式/新变量**必须重编译——那是任何数据 diff 路线的
//! 共同天花板,不是本设计的缺陷。
//!
//! # 三条刻意的裁决
//!
//! 1. **`&'static` 而不是 `Cow`**:release 下模板是 `static` 常量,const 构造零摩擦;
//!    dev 下热重载推来的数据用 `Box::leak` 泄漏成 `'static`。一份模板是 KB 级,
//!    一天几百次重载也就几 MB,而 `Cow` 会在**每个**节点上加一层分支与 `Clone` 约束,
//!    代价落在 release 的热路径上。用泄漏换热路径干净。
//! 2. **动态位只枚举高频三种(文本/样式/点击),其余一律 [`Bind::Wire`]**:
//!    数据面不该跟着 sv-ui 的 API 面积膨胀。新增事件种类本就必须重编译(调研 09 §5.1
//!    的免重编译边界里没有它),所以把低频事件塞进通用逃生舱不损失任何热重载能力,
//!    却让数据面的格式稳定下来——**格式一旦要跟着 API 改,热重载协议就得跟着改版**。
//! 3. **不引 serde**:调研 09 步骤 2 里带了 serde(dev JSON / 通道 postcard),
//!    但通道要到步骤 4-5 才存在。sv-ui 是双前端的编译目标,依赖面必须干净(仓库既有
//!    纪律),现在加了也没有消费者。本模块的类型全部是 `Copy`/`&'static`,
//!    owned 镜像与派生是平凡的,等通道真的落地再加不迟。
//!
//! # 槽位不匹配一律**跳过**而不是 panic
//!
//! 热重载时数据面可能领先于代码面(推来的模板引用了旧二进制里没有的槽)。
//! 崩掉整个 app 是最坏的处理方式(R4 去 panic 同一纪律),所以越界/类型不符
//! 只在 debug 下断言 + 跳过该位,其余照常建树。

use std::rc::Rc;

/// 通用接线闭包:拿到 `(doc, 目标节点)` 自己干活。
/// 块与低频事件共用它 —— 见模块头裁决 2
pub type WireFn = Rc<dyn Fn(&Doc, ViewId)>;

use crate::{
    AlignItems, Border, Color, Cursor, Direction, Doc, ElementKind, FlexWrap, JustifyContent,
    Overflow, Style, TextAlign, TextWrap, ViewId,
};

// ---------------------------------------------------------------------------
// 数据面
// ---------------------------------------------------------------------------

/// 一个模板(一个 `.sv` 的模板块;`{#if}`/`{#each}` 的分支体各是**独立子模板**)
#[derive(Clone, Copy, Debug)]
pub struct Template {
    /// 稳定 id:`"src/counter.sv#0"`(文件 + 块序号)。热重载按它索引实例
    pub id: &'static str,
    pub roots: &'static [TNode],
    /// 槽位签名表,**下标即槽位号**。热重载判据:签名一致 = 只推数据面即可
    pub sig: &'static [SlotSig],
}

/// 热重载判据的结论。
///
/// 语义只有两种,但 `DataOnly` 里那张重映射表是全部的难点所在。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// 只推数据面即可。`remap[新槽位] = 旧槽位下标` ——
    /// **推过去之前必须按它把新数据里的槽位号改写成旧表下标**,见 [`remap_slots`]。
    DataOnly { remap: Vec<u16> },
    /// 必须走 rustc。带上原因,给热通道打日志用(不给原因的话现场只能看到"又全量重编了")
    NeedsRustc(NeedsRustc),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NeedsRustc {
    /// 模板 id 变了(新增/改名的模板,旧二进制里没有它的任何闭包)
    NewTemplate,
    /// 新模板引用了旧表里找不到的签名 —— 即"写了一个旧二进制没编译过的表达式"
    NewExpr { slot: u16, sig: SlotSig },
}

impl Template {
    /// 新旧模板能否**只换数据面**(不经 rustc),以及新槽位号要怎么改写成旧的。
    ///
    /// # 判据(调研 09 §5.2 / `docs/plans/adr2-3-setup-render-split.md` §5.2)
    ///
    /// 新模板引用到的每个槽位签名,都能在旧签名表里找到**一个**同签名的槽位。
    /// 也就是说新表是旧表的**子集**,而且:
    /// - **可以少**:旧表里多出来的槽位闲置着,没人引用就没关系;
    /// - **可以重排**:匹配靠签名不靠下标;
    /// - **可以多对一**:新模板把 `{count}` 复制成两处,两处都映射到旧表同一个 binder。
    ///   这是合法的 —— 同一模板里同一个表达式两次出现在词法上完全等价,共用同一个
    ///   函数体作用域,绑同一个闭包不会错。
    ///
    /// # 为什么不能只写 `self.sig == next.sig`
    ///
    /// 本函数**之前就是那么写的**,而且它的方案文档里被记成"§5.2 的判据算法已有可
    /// 运行实现" —— 那句话不成立。逐位全等会把"新增一个静态节点导致后面槽位整体后移"
    /// 判成需要 rustc,而那恰恰是热重载最常见、最该免 rustc 的一类改动
    /// (改标记结构)。它连"少一个槽位"都判不过。
    ///
    /// # 重映射才是真正的难点
    ///
    /// 热通道推过去的数据面是**新编译的**,槽位编号是新编的;而运行中的 app 手里
    /// 只有**旧的** binders 表。所以推送前必须把新数据里的每个槽位号改写成旧表下标。
    /// Dioxus 也是卡在这一步。
    pub fn hot_swap_verdict(&self, next: &Template) -> Verdict {
        if self.id != next.id {
            return Verdict::NeedsRustc(NeedsRustc::NewTemplate);
        }
        // 旧表的签名索引:同签名多次出现时取**第一个**下标,让多对一映射有确定落点
        let mut idx: Vec<(SlotSig, u16)> = Vec::with_capacity(self.sig.len());
        for (i, s) in self.sig.iter().enumerate() {
            if !idx.iter().any(|(k, _)| k == s) {
                idx.push((*s, i as u16));
            }
        }
        let mut remap = Vec::with_capacity(next.sig.len());
        for (i, need) in next.sig.iter().enumerate() {
            match idx.iter().find(|(k, _)| k == need) {
                Some((_, old)) => remap.push(*old),
                None => {
                    return Verdict::NeedsRustc(NeedsRustc::NewExpr {
                        slot: i as u16,
                        sig: *need,
                    });
                }
            }
        }
        Verdict::DataOnly { remap }
    }

    /// 旧接口:只回答"能不能免 rustc"。保留是因为大量调用方只关心这一个 bit
    pub fn hot_swappable_with(&self, next: &Template) -> bool {
        matches!(self.hot_swap_verdict(next), Verdict::DataOnly { .. })
    }
}

/// 按 [`Verdict::DataOnly`] 的重映射表改写一棵新模板节点树里的槽位号。
///
/// **这一步漏了就是静默绑错闭包**:新数据面的槽位号是新编的,而运行中的 app
/// 手里是旧 binders 表 —— 不改写就会拿新号去索引旧表,取到的是别的表达式。
///
/// # ⚠️ 本函数**会泄漏内存**,而且是有意的
///
/// `TNode` 的 `binds`/`children` 是 `&'static [_]`(见本模块头的裁决 1:
/// 数据面用 `&'static` 换 `Copy` + 热路径零分支,dev 下靠 `Box::leak` 造出来)。
/// 改写后的数组必须也是 `'static`,所以这里**每调一次就 leak 一批小数组**。
///
/// 量级:一次热重载泄漏"这棵模板树的 binds + children 数组",一个中等组件
/// 在几 KB 量级 —— 开发时按分钟计的重载频率下可以忽略。
/// **但它只该出现在 dev 热通道里**;release 的数据面直接来自 `static`,
/// 根本不需要重映射(槽位号编译期就是对的)。
///
/// 之所以不做成"调用方负责 leak":外层返回的 `Vec<TNode>` 可以由调用方处置,
/// 但内层的 `binds`/`children` 在构造 `TNode` 的那一刻就必须已经是 `'static` 了,
/// 这个选择躲不掉。要躲只能把 `TNode` 改成 `Cow`,而那正是裁决 1 拒绝的方案。
pub fn remap_slots(nodes: &[TNode], remap: &[u16]) -> Vec<TNode> {
    fn map1(slot: u16, remap: &[u16]) -> u16 {
        match remap.get(slot as usize) {
            Some(new) => *new,
            None => {
                debug_assert!(
                    false,
                    "sv-ui::tmpl: 槽位 {slot} 不在重映射表里(表长 {})——                     判据与数据面对不上,保持原号(stamp 时会再逮一次)",
                    remap.len()
                );
                slot
            }
        }
    }
    nodes
        .iter()
        .map(|n| match n {
            TNode::Elem {
                kind,
                label,
                style,
                binds,
                children,
            } => TNode::Elem {
                kind: *kind,
                label,
                style,
                // binds/children 要跟着改写,而它们是 &'static —— 只能新建 + leak。
                // 这是本函数会泄漏的原因,见上面的 ⚠️ 段
                binds: Box::leak(
                    binds
                        .iter()
                        .map(|b| b.with_slot(map1(b.slot(), remap)))
                        .collect::<Vec<_>>()
                        .into_boxed_slice(),
                ),
                children: Box::leak(remap_slots(children, remap).into_boxed_slice()),
            },
            TNode::Block { slot } => TNode::Block {
                slot: map1(*slot, remap),
            },
        })
        .collect()
}

/// 槽位签名:种类 + 表达式源码 hash。由 codegen 产出(现阶段手写于测试)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SlotSig {
    pub kind: SlotKind,
    /// 表达式源码的 hash。**必须**由源码而非生成代码算——生成代码会随无关改动波动
    pub hash: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SlotKind {
    Text,
    Style,
    Click,
    Wire,
}

/// 模板节点。只有两种:元素与块——**块是数据面与代码面的边界**,
/// `{#if}`/`{#each}` 的控制流永远在代码面(我们不做 Rust 表达式解释器,ADR-2)
#[derive(Clone, Copy, Debug)]
pub enum TNode {
    Elem {
        kind: ElementKind,
        /// Text/Button 的静态 label;有插值时为空串并配一个 [`Bind::Text`]
        label: &'static str,
        /// 静态样式声明——**数据**,改样式不必过 rustc
        style: &'static [StyleDecl],
        /// 该元素上的动态位,指向 binders 表
        binds: &'static [Bind],
        children: &'static [TNode],
    },
    /// 整块交给槽位闭包(`{#if}`/`{#each}`/`{#key}`/`{#await}`/`<overlay>`)。
    /// 闭包内部照常调 `if_block`/`each_block`,并对分支体 [`stamp`] 子模板
    Block { slot: u16 },
}

/// 元素上的动态位。数据里只留"第几个槽 + 什么用途",闭包本体在代码面
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bind {
    /// `binders[i]` 是 [`Binder::Text`] → `bind_text`
    Text(u16),
    /// `binders[i]` 是 [`Binder::Style`] → `bind_style`
    Style(u16),
    /// `binders[i]` 是 [`Binder::Click`] → `set_on_click`
    Click(u16),
    /// `binders[i]` 是 [`Binder::Wire`] → 拿到 `(doc, 本元素)` 自己接线。
    /// 低频事件(键盘/输入/滚动/无障碍/附着/过渡)全走这里,见模块头裁决 2
    Wire(u16),
}

impl Bind {
    /// 该动态位期望的槽位种类(与 [`SlotSig::kind`] 对齐)
    pub fn kind(self) -> SlotKind {
        match self {
            Bind::Text(_) => SlotKind::Text,
            Bind::Style(_) => SlotKind::Style,
            Bind::Click(_) => SlotKind::Click,
            Bind::Wire(_) => SlotKind::Wire,
        }
    }

    pub fn slot(self) -> u16 {
        match self {
            Bind::Text(i) | Bind::Style(i) | Bind::Click(i) | Bind::Wire(i) => i,
        }
    }

    /// 换个槽位号,种类不变(热重载重映射用,见 [`remap_slots`])
    pub fn with_slot(self, slot: u16) -> Self {
        match self {
            Bind::Text(_) => Bind::Text(slot),
            Bind::Style(_) => Bind::Style(slot),
            Bind::Click(_) => Bind::Click(slot),
            Bind::Wire(_) => Bind::Wire(slot),
        }
    }
}

// ---------------------------------------------------------------------------
// 静态样式:声明即数据
// ---------------------------------------------------------------------------

/// 一条静态样式声明。**与 [`Style`] 字段一一对应**——
/// [`StyleDecl::snapshot`] 用解构写,所以 `Style` 新增字段会**编译期报错**,
/// 逼着这里同步补变体(否则那个键就悄悄没法数据化了,而这种遗漏运行时看不出来)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum StyleDecl {
    Direction(Direction),
    Gap(f32),
    Padding(crate::Edges),
    Margin(crate::Edges),
    Border(Option<Border>),
    Bg(Option<Color>),
    Fg(Option<Color>),
    /// NAN = 继承(与 [`Style::font_size`] 同哨兵)
    FontSize(f32),
    Width(Option<f32>),
    Height(Option<f32>),
    CornerRadius(f32),
    Opacity(f32),
    Cursor(Option<Cursor>),
    Overflow(Overflow),
    OverflowX(Overflow),
    JustifyContent(JustifyContent),
    AlignItems(AlignItems),
    AlignSelf(Option<AlignItems>),
    FlexGrow(f32),
    FlexShrink(f32),
    FlexWrap(FlexWrap),
    MinWidth(Option<f32>),
    MinHeight(Option<f32>),
    MaxWidth(Option<f32>),
    MaxHeight(Option<f32>),
    TextWrap(TextWrap),
    TextAlign(TextAlign),
}

impl StyleDecl {
    /// 应用到 [`Style`]
    pub fn apply(self, s: &mut Style) {
        match self {
            StyleDecl::Direction(v) => s.direction = v,
            StyleDecl::Gap(v) => s.gap = v,
            StyleDecl::Padding(v) => s.padding = v,
            StyleDecl::Margin(v) => s.margin = v,
            StyleDecl::Border(v) => s.border = v,
            StyleDecl::Bg(v) => s.bg = v,
            StyleDecl::Fg(v) => s.fg = v,
            StyleDecl::FontSize(v) => s.font_size = v,
            StyleDecl::Width(v) => s.width = v,
            StyleDecl::Height(v) => s.height = v,
            StyleDecl::CornerRadius(v) => s.corner_radius = v,
            StyleDecl::Opacity(v) => s.opacity = v,
            StyleDecl::Cursor(v) => s.cursor = v,
            StyleDecl::Overflow(v) => s.overflow = v,
            StyleDecl::OverflowX(v) => s.overflow_x = v,
            StyleDecl::JustifyContent(v) => s.justify_content = v,
            StyleDecl::AlignItems(v) => s.align_items = v,
            StyleDecl::AlignSelf(v) => s.align_self = v,
            StyleDecl::FlexGrow(v) => s.flex_grow = v,
            StyleDecl::FlexShrink(v) => s.flex_shrink = v,
            StyleDecl::FlexWrap(v) => s.flex_wrap = v,
            StyleDecl::MinWidth(v) => s.min_width = v,
            StyleDecl::MinHeight(v) => s.min_height = v,
            StyleDecl::MaxWidth(v) => s.max_width = v,
            StyleDecl::MaxHeight(v) => s.max_height = v,
            StyleDecl::TextWrap(v) => s.text_wrap = v,
            StyleDecl::TextAlign(v) => s.text_align = v,
        }
    }

    /// [`Style`] → 声明序列(**只留与缺省不同的项**,数据才紧凑)。
    ///
    /// codegen 从"发射静态样式闭包"迁到"发射数据"时走这条;dev 侧比对样式改动也走它。
    /// 实现刻意用**解构**:`Style` 新增字段 → 这里编译报错 → 不会出现"新样式键
    /// 悄悄无法数据化"这种只有用户才发现得了的洞。
    pub fn snapshot(style: &Style) -> Vec<StyleDecl> {
        let Style {
            direction,
            gap,
            padding,
            margin,
            border,
            bg,
            fg,
            font_size,
            width,
            height,
            corner_radius,
            opacity,
            cursor,
            overflow,
            overflow_x,
            justify_content,
            align_items,
            align_self,
            flex_grow,
            flex_shrink,
            flex_wrap,
            min_width,
            min_height,
            max_width,
            max_height,
            text_wrap,
            text_align,
        } = *style;
        let d = Style::default();
        let mut out = Vec::new();
        let mut push = |cond: bool, decl: StyleDecl| {
            if cond {
                out.push(decl);
            }
        };
        push(direction != d.direction, StyleDecl::Direction(direction));
        push(gap != d.gap, StyleDecl::Gap(gap));
        push(padding != d.padding, StyleDecl::Padding(padding));
        push(margin != d.margin, StyleDecl::Margin(margin));
        push(border != d.border, StyleDecl::Border(border));
        push(bg != d.bg, StyleDecl::Bg(bg));
        push(fg != d.fg, StyleDecl::Fg(fg));
        // font_size 的 NAN 是"继承"哨兵:两边同为 NAN 视为相同,
        // 否则每次快照都会吐出一条无意义的 FontSize(NaN)(NaN != NaN)
        push(
            !(font_size.is_nan() && d.font_size.is_nan()) && font_size != d.font_size,
            StyleDecl::FontSize(font_size),
        );
        push(width != d.width, StyleDecl::Width(width));
        push(height != d.height, StyleDecl::Height(height));
        push(
            corner_radius != d.corner_radius,
            StyleDecl::CornerRadius(corner_radius),
        );
        push(opacity != d.opacity, StyleDecl::Opacity(opacity));
        push(cursor != d.cursor, StyleDecl::Cursor(cursor));
        push(overflow != d.overflow, StyleDecl::Overflow(overflow));
        push(overflow_x != d.overflow_x, StyleDecl::OverflowX(overflow_x));
        push(
            justify_content != d.justify_content,
            StyleDecl::JustifyContent(justify_content),
        );
        push(
            align_items != d.align_items,
            StyleDecl::AlignItems(align_items),
        );
        push(align_self != d.align_self, StyleDecl::AlignSelf(align_self));
        push(flex_grow != d.flex_grow, StyleDecl::FlexGrow(flex_grow));
        push(
            flex_shrink != d.flex_shrink,
            StyleDecl::FlexShrink(flex_shrink),
        );
        push(flex_wrap != d.flex_wrap, StyleDecl::FlexWrap(flex_wrap));
        push(min_width != d.min_width, StyleDecl::MinWidth(min_width));
        push(min_height != d.min_height, StyleDecl::MinHeight(min_height));
        push(max_width != d.max_width, StyleDecl::MaxWidth(max_width));
        push(max_height != d.max_height, StyleDecl::MaxHeight(max_height));
        push(text_wrap != d.text_wrap, StyleDecl::TextWrap(text_wrap));
        push(text_align != d.text_align, StyleDecl::TextAlign(text_align));
        out
    }
}

// ---------------------------------------------------------------------------
// 代码面
// ---------------------------------------------------------------------------

/// 一个槽位的闭包本体。数据面只存"第几个槽",本体在这里——
/// 这就是"数据 diff 式热重载"只能引用**旧二进制里已编译的表达式**的原因
#[derive(Clone)]
pub enum Binder {
    /// 文本插值:`Fn() -> String`,接 `bind_text`
    Text(Rc<dyn Fn() -> String>),
    /// 动态样式(`style:` 指令 / 条件类 / `:hover` 等):`Fn(&mut Style)`,接 `bind_style`
    Style(Rc<dyn Fn(&mut Style)>),
    Click(Rc<dyn Fn()>),
    /// 通用逃生舱:拿到 `(doc, 目标节点)` 自己接线。
    /// 块([`TNode::Block`])与低频事件都走这条
    Wire(WireFn),
}

impl Binder {
    pub fn kind(&self) -> SlotKind {
        match self {
            Binder::Text(_) => SlotKind::Text,
            Binder::Style(_) => SlotKind::Style,
            Binder::Click(_) => SlotKind::Click,
            Binder::Wire(_) => SlotKind::Wire,
        }
    }

    /// 便捷构造(codegen 侧写起来短一点)
    pub fn text(f: impl Fn() -> String + 'static) -> Self {
        Binder::Text(Rc::new(f))
    }
    pub fn style(f: impl Fn(&mut Style) + 'static) -> Self {
        Binder::Style(Rc::new(f))
    }
    pub fn click(f: impl Fn() + 'static) -> Self {
        Binder::Click(Rc::new(f))
    }
    pub fn wire(f: impl Fn(&Doc, ViewId) + 'static) -> Self {
        Binder::Wire(Rc::new(f))
    }
}

// ---------------------------------------------------------------------------
// 解释器
// ---------------------------------------------------------------------------

/// 按模板数据建树(调研 09 的 `stamp`)。
///
/// 与 codegen 现在直接发射的命令式代码**语义等价**:同一份 UI 两条路建出的
/// `dump()` 必须逐字节相同(`stamp_matches_imperative` 钉死)。
/// 按模板数据建树并接线。可**重复调用**、不持有任何状态 ——
/// 热重载重放 = 清子树 + 再 stamp 一次。
///
/// # 为什么是 `Rc<[Binder]>` 而不是 `&[Binder]`
///
/// 块槽位(`{#if}`/`{#each}`)的闭包必须是 `'static`:它们进 effect,
/// 而 effect 活得比这次调用长。闭包内部要对分支体再 stamp 一次子模板,
/// 于是**它得把 binders 表一起捕获**。借用做不到(E0521:借用的数据逃出了
/// 它该在的函数),而 `Rc` 克隆一次只是加一个引用计数。
///
/// 今天的 `Binder::Wire` 还没走到这一步,但把签名先定成 `Rc<[Binder]>`
/// 是为了不在 S2/S3 落地时回头改所有调用方 —— 那时表已经在闭包里了。
pub fn stamp(doc: &Doc, parent: ViewId, tpl: &Template, binders: std::rc::Rc<[Binder]>) {
    for node in tpl.roots {
        stamp_node(doc, parent, node, &binders);
    }
}

fn stamp_node(doc: &Doc, parent: ViewId, node: &TNode, binders: &[Binder]) {
    match node {
        TNode::Elem {
            kind,
            label,
            style,
            binds,
            children,
        } => {
            let el = match kind {
                ElementKind::View => doc.create_view(),
                ElementKind::Text => doc.create_text(label),
                ElementKind::Button => doc.create_button(label),
                ElementKind::Checkbox => doc.create_checkbox(),
                ElementKind::TextInput => doc.create_text_input(),
                // 数据面建不出真的动画:内容句柄是运行期才有的东西(注册表在
                // 渲染壳侧)。所以先建占位,再由 `Bind::Wire` 把真素材接上 ——
                // 这正是模块头裁决 2 里"逃生舱"的用途
                ElementKind::Animation => doc.create_animation(crate::AnimData::placeholder()),
            };
            doc.append(parent, el);
            // 静态样式一次性应用:一次 update_style = 一次 bump,
            // 而不是每条声明各 bump 一次
            if !style.is_empty() {
                doc.update_style(el, |s| {
                    for d in *style {
                        d.apply(s);
                    }
                });
            }
            for b in *binds {
                wire_bind(doc, el, *b, binders);
            }
            for c in *children {
                stamp_node(doc, el, c, binders);
            }
        }
        TNode::Block { slot } => {
            // 块的父是**当前 parent**(块自己不建元素);闭包内部照常调
            // if_block/each_block,并对分支体 stamp 子模板
            if let Some(Binder::Wire(f)) = slot_of(binders, *slot, SlotKind::Wire) {
                f(doc, parent);
            }
        }
    }
}

fn wire_bind(doc: &Doc, el: ViewId, b: Bind, binders: &[Binder]) {
    match (b, slot_of(binders, b.slot(), b.kind())) {
        (Bind::Text(_), Some(Binder::Text(f))) => {
            let f = f.clone();
            crate::bind_text(doc, el, move || f());
        }
        (Bind::Style(_), Some(Binder::Style(f))) => {
            let f = f.clone();
            crate::bind_style(doc, el, move |s| f(s));
        }
        (Bind::Click(_), Some(Binder::Click(f))) => {
            let f = f.clone();
            doc.set_on_click(el, move || f());
        }
        (Bind::Wire(_), Some(Binder::Wire(f))) => f(doc, el),
        // slot_of 已经在 debug 下断言过,这里静默跳过:热重载时数据面可能
        // 领先于代码面,崩掉整个 app 是最坏处理(R4 去 panic 同一纪律)
        _ => {}
    }
}

/// 取槽位并校验种类。越界/种类不符:debug 断言 + `None`
fn slot_of(binders: &[Binder], slot: u16, want: SlotKind) -> Option<&Binder> {
    let Some(b) = binders.get(slot as usize) else {
        debug_assert!(
            false,
            "sv-ui::tmpl: 槽位 {slot} 越界(binders 只有 {} 个)——\
             数据面与代码面对不上,本位跳过",
            binders.len()
        );
        return None;
    };
    if b.kind() != want {
        debug_assert!(
            false,
            "sv-ui::tmpl: 槽位 {slot} 种类不符:数据面要 {want:?},代码面是 {:?}",
            b.kind()
        );
        return None;
    }
    Some(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edges, each_block, if_block};
    use sv_reactive::{create_root, state};

    /// 数据面与命令式路径**逐字节等价** —— 这是 codegen 将来切过去的契约。
    /// 一旦 stamp 的建树顺序/样式应用时机跑偏,这条先红
    #[test]
    fn stamp_matches_imperative() {
        // (a) 命令式:codegen 今天发射的形态
        let a = Doc::new();
        let (_, _sa) = create_root(|| {
            let root = a.create_view();
            a.append(a.root(), root);
            a.update_style(root, |s| {
                s.gap = 12.0;
                s.padding = Edges::all(24.0);
            });
            let title = a.create_text("标题");
            a.append(root, title);
            a.update_style(title, |s| s.font_size = 26.0);
            let btn = a.create_button("确定");
            a.append(root, btn);
        });

        // (b) 数据面:同一棵树用 Template 描述
        static ROOTS: &[TNode] = &[TNode::Elem {
            kind: ElementKind::View,
            label: "",
            style: &[
                StyleDecl::Gap(12.0),
                StyleDecl::Padding(Edges {
                    top: 24.0,
                    right: 24.0,
                    bottom: 24.0,
                    left: 24.0,
                }),
            ],
            binds: &[],
            children: &[
                TNode::Elem {
                    kind: ElementKind::Text,
                    label: "标题",
                    style: &[StyleDecl::FontSize(26.0)],
                    binds: &[],
                    children: &[],
                },
                TNode::Elem {
                    kind: ElementKind::Button,
                    label: "确定",
                    style: &[],
                    binds: &[],
                    children: &[],
                },
            ],
        }];
        static TPL: Template = Template {
            id: "test#0",
            roots: ROOTS,
            sig: &[],
        };

        let b = Doc::new();
        let (_, _sb) = create_root(|| stamp(&b, b.root(), &TPL, std::rc::Rc::from(vec![])));

        // dump() **不含样式/回调/focusable**(只有种类+文本+checked+层级),
        // 单靠它对拍会把"样式没设上"这类错放过去 —— 所以再逐节点比 Style
        assert_eq!(a.dump(), b.dump(), "结构与文本应逐字节相同");
        assert_eq!(
            full_shape(&a),
            full_shape(&b),
            "样式也必须一致(dump 看不见样式,这条才是真正的等价性断言)"
        );
    }

    /// 结构 + 每节点完整 `Style` 的快照。`dump()` 覆盖不到样式,而数据面
    /// 最容易出错的恰恰是"StyleDecl 没被应用 / 应用顺序不对"
    fn full_shape(doc: &Doc) -> String {
        fn walk(inner: &crate::DocumentInner, id: crate::ViewId, depth: usize, out: &mut String) {
            let n = &inner.nodes[id];
            out.push_str(&format!(
                "{}{:?} text={:?} style={:?}
",
                "  ".repeat(depth),
                n.kind,
                n.text,
                n.style
            ));
            for c in &n.children {
                walk(inner, *c, depth + 1, out);
            }
        }
        doc.read(|inner| {
            let mut out = String::new();
            walk(inner, inner.root, 0, &mut out);
            out
        })
    }

    /// 动态位:文本/样式/点击三种高频槽都要真的接上响应式
    #[test]
    fn dynamic_slots_are_reactive() {
        let doc = Doc::new();
        let clicks = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = std::cell::RefCell::new(None);

        static ROOTS: &[TNode] = &[TNode::Elem {
            kind: ElementKind::View,
            label: "",
            style: &[],
            binds: &[Bind::Style(1)],
            children: &[
                TNode::Elem {
                    kind: ElementKind::Text,
                    label: "",
                    style: &[],
                    binds: &[Bind::Text(0)],
                    children: &[],
                },
                TNode::Elem {
                    kind: ElementKind::Button,
                    label: "+1",
                    style: &[],
                    binds: &[Bind::Click(2)],
                    children: &[],
                },
            ],
        }];
        static TPL: Template = Template {
            id: "test#1",
            roots: ROOTS,
            sig: &[],
        };

        let c = clicks.clone();
        let (_, _s) = create_root(|| {
            let n = state(0i32);
            *count.borrow_mut() = Some(n);
            let binders = [
                Binder::text(move || format!("计数 {}", n.get())),
                // 样式也读 signal:改 signal 应重算样式
                Binder::style(move |s| s.gap = n.get() as f32),
                Binder::click(move || {
                    c.set(c.get() + 1);
                    n.update(|v| *v += 1);
                }),
            ];
            stamp(&doc, doc.root(), &TPL, std::rc::Rc::from(binders.to_vec()));
        });

        assert!(doc.dump().contains("计数 0"), "\n{}", doc.dump());
        let root_view = doc.read(|i| i.nodes[i.root].children[0]);
        assert_eq!(doc.read(|i| i.nodes[root_view].style.gap), 0.0);

        // 点按钮 → 回调 + signal → 文本与样式一起跟上(定点更新,零 diff)
        let btn = doc.read(|i| i.nodes[root_view].children[1]);
        doc.click_handler(btn).expect("按钮应有点击回调")();
        assert_eq!(clicks.get(), 1);
        assert!(doc.dump().contains("计数 1"), "\n{}", doc.dump());
        assert_eq!(
            doc.read(|i| i.nodes[root_view].style.gap),
            1.0,
            "样式槽也该跟着 signal 重算"
        );
    }

    /// 块槽位:`{#if}`/`{#each}` 的控制流留在代码面,数据面只留一个 Block 位。
    /// 分支体本身又是一份模板 —— 嵌套由此闭合
    #[test]
    fn block_slot_drives_control_flow() {
        static INNER: &[TNode] = &[TNode::Elem {
            kind: ElementKind::Text,
            label: "展开了",
            style: &[],
            binds: &[],
            children: &[],
        }];
        static INNER_TPL: Template = Template {
            id: "test#2.then",
            roots: INNER,
            sig: &[],
        };
        static ROOTS: &[TNode] = &[TNode::Elem {
            kind: ElementKind::View,
            label: "",
            style: &[],
            binds: &[],
            children: &[TNode::Block { slot: 0 }],
        }];
        static TPL: Template = Template {
            id: "test#2",
            roots: ROOTS,
            sig: &[],
        };

        let doc = Doc::new();
        let open_cell = std::cell::RefCell::new(None);
        let (_, _s) = create_root(|| {
            let open = state(false);
            *open_cell.borrow_mut() = Some(open);
            let binders = [Binder::wire(move |d, parent| {
                if_block(
                    d,
                    parent,
                    move || open.get(),
                    // 分支体 = 子模板;真实 codegen 里这里是 stamp(子模板)
                    |d, p| stamp(d, p, &INNER_TPL, std::rc::Rc::from(vec![])),
                    |_, _| {},
                );
            })];
            stamp(&doc, doc.root(), &TPL, std::rc::Rc::from(binders.to_vec()));
        });

        assert!(!doc.dump().contains("展开了"));
        open_cell.borrow().unwrap().set(true);
        assert!(doc.dump().contains("展开了"), "\n{}", doc.dump());
        open_cell.borrow().unwrap().set(false);
        assert!(!doc.dump().contains("展开了"), "块关闭应销毁子树");
    }

    /// each 块同理:行体是子模板,行数据经 binder 闭包进来
    #[test]
    fn each_block_via_slot() {
        static ROW: &[TNode] = &[TNode::Elem {
            kind: ElementKind::Text,
            label: "",
            style: &[],
            binds: &[Bind::Text(0)],
            children: &[],
        }];
        static ROW_TPL: Template = Template {
            id: "test#3.row",
            roots: ROW,
            sig: &[],
        };
        static ROOTS: &[TNode] = &[TNode::Block { slot: 0 }];
        static TPL: Template = Template {
            id: "test#3",
            roots: ROOTS,
            sig: &[],
        };

        let doc = Doc::new();
        let items_cell = std::cell::RefCell::new(None);
        let (_, _s) = create_root(|| {
            let items = state(vec!["甲".to_string(), "乙".to_string()]);
            *items_cell.borrow_mut() = Some(items);
            let binders = [Binder::wire(move |d, parent| {
                each_block(
                    d,
                    parent,
                    move || items.get(),
                    |d, p, item: &String, _i| {
                        let text = item.clone();
                        stamp(
                            d,
                            p,
                            &ROW_TPL,
                            std::rc::Rc::from(vec![Binder::text(move || text.clone())]),
                        );
                    },
                );
            })];
            stamp(&doc, doc.root(), &TPL, std::rc::Rc::from(binders.to_vec()));
        });

        let dump = doc.dump();
        assert!(dump.contains("甲") && dump.contains("乙"), "\n{dump}");
        items_cell.borrow().unwrap().update(|v| v.push("丙".into()));
        assert!(doc.dump().contains("丙"));
    }

    /// 样式快照:与缺省相同的字段不进数据;NAN(继承哨兵)不算差异
    #[test]
    fn style_snapshot_is_minimal_and_roundtrips() {
        let mut s = Style::default();
        assert!(
            StyleDecl::snapshot(&s).is_empty(),
            "缺省样式应产出空声明表(font_size 的 NAN 不算差异)"
        );

        s.gap = 8.0;
        s.bg = Some(Color::rgb(1, 2, 3));
        s.overflow = Overflow::Scroll;
        s.font_size = 18.0;
        let decls = StyleDecl::snapshot(&s);
        assert_eq!(decls.len(), 4, "只该留改动过的四条:{decls:?}");

        // 往返:声明重放回缺省样式应还原
        let mut back = Style::default();
        for d in &decls {
            d.apply(&mut back);
        }
        assert_eq!(back, s, "声明重放应还原样式");
    }

    /// 热重载判据:结构/静态文本/静态样式随便改都能只推数据;
    /// 槽位签名一变(新表达式)就必须重编译
    #[test]
    fn hot_swap_judged_by_slot_signature() {
        static SIG: &[SlotSig] = &[
            SlotSig {
                kind: SlotKind::Text,
                hash: 0xabc,
            },
            SlotSig {
                kind: SlotKind::Click,
                hash: 0xdef,
            },
        ];
        static A: Template = Template {
            id: "src/counter.sv#0",
            roots: &[],
            sig: SIG,
        };
        // 改了结构(roots 不同)但槽位签名一致 → 可热换
        static B: Template = Template {
            id: "src/counter.sv#0",
            roots: &[TNode::Elem {
                kind: ElementKind::Text,
                label: "新加的一行",
                style: &[],
                binds: &[],
                children: &[],
            }],
            sig: SIG,
        };
        assert!(A.hot_swappable_with(&B), "只改结构/静态文本应能热换");

        // 表达式变了(hash 变)→ 必须重编译
        static C: Template = Template {
            id: "src/counter.sv#0",
            roots: &[],
            sig: &[
                SlotSig {
                    kind: SlotKind::Text,
                    hash: 0x999,
                },
                SlotSig {
                    kind: SlotKind::Click,
                    hash: 0xdef,
                },
            ],
        };
        assert!(!A.hot_swappable_with(&C), "表达式改了不能只推数据");
        assert!(matches!(
            A.hot_swap_verdict(&C),
            Verdict::NeedsRustc(NeedsRustc::NewExpr { slot: 0, .. })
        ));

        // 删掉一个插值(新表是旧表的**子集**)→ **能热换**。
        // 【这条以前断言的是反的】旧实现写的是 sig 逐位全等,于是这里断言
        // "不能热换",注释还写成"新增插值" —— 而这个 fixture 其实是**少了**一个槽位。
        // 判据(方案 §5.2)明说子集合法:旧表里多出来的 binder 闲置着,没人引用就没关系
        static D: Template = Template {
            id: "src/counter.sv#0",
            roots: &[],
            sig: &[SlotSig {
                kind: SlotKind::Text,
                hash: 0xabc,
            }],
        };
        assert_eq!(
            A.hot_swap_verdict(&D),
            Verdict::DataOnly { remap: vec![0] },
            "删一个插值是纯数据面改动"
        );

        // 不同模板 id 之间不谈热换
        static E: Template = Template {
            id: "src/other.sv#0",
            roots: &[],
            sig: SIG,
        };
        assert!(!A.hot_swappable_with(&E));
        assert_eq!(
            A.hot_swap_verdict(&E),
            Verdict::NeedsRustc(NeedsRustc::NewTemplate)
        );
    }

    /// 槽位**重排**与**多对一**都合法 —— 匹配靠签名,不靠下标
    #[test]
    fn hot_swap_allows_reorder_and_many_to_one() {
        static SIG: &[SlotSig] = &[
            SlotSig {
                kind: SlotKind::Text,
                hash: 0xaaa,
            },
            SlotSig {
                kind: SlotKind::Click,
                hash: 0xbbb,
            },
        ];
        static OLD: Template = Template {
            id: "t#0",
            roots: &[],
            sig: SIG,
        };

        // 换个顺序
        static REORDERED: Template = Template {
            id: "t#0",
            roots: &[],
            sig: &[
                SlotSig {
                    kind: SlotKind::Click,
                    hash: 0xbbb,
                },
                SlotSig {
                    kind: SlotKind::Text,
                    hash: 0xaaa,
                },
            ],
        };
        assert_eq!(
            OLD.hot_swap_verdict(&REORDERED),
            Verdict::DataOnly { remap: vec![1, 0] },
            "重排要映射回旧下标"
        );

        // 把 {count} 复制成两份:两个新槽位映射到同一个旧 binder。
        // 合法的理由是词法等价 —— 同一模板里同一个表达式两次出现,
        // 共用同一个函数体作用域,绑同一个闭包不会错
        static DUPLICATED: Template = Template {
            id: "t#0",
            roots: &[],
            sig: &[
                SlotSig {
                    kind: SlotKind::Text,
                    hash: 0xaaa,
                },
                SlotSig {
                    kind: SlotKind::Text,
                    hash: 0xaaa,
                },
                SlotSig {
                    kind: SlotKind::Click,
                    hash: 0xbbb,
                },
            ],
        };
        assert_eq!(
            OLD.hot_swap_verdict(&DUPLICATED),
            Verdict::DataOnly {
                remap: vec![0, 0, 1]
            },
            "多对一映射合法:复制一份 {{count}} 不该触发 rustc"
        );

        // 同种类但 hash 不同 = 不同表达式,不能混
        static SAME_KIND_NEW_EXPR: Template = Template {
            id: "t#0",
            roots: &[],
            sig: &[SlotSig {
                kind: SlotKind::Text,
                hash: 0xccc,
            }],
        };
        assert!(matches!(
            OLD.hot_swap_verdict(&SAME_KIND_NEW_EXPR),
            Verdict::NeedsRustc(NeedsRustc::NewExpr { .. })
        ));

        // hash 相同但种类不同 = 不能混(否则会拿 Click 闭包去当 Text 用)
        static SAME_HASH_NEW_KIND: Template = Template {
            id: "t#0",
            roots: &[],
            sig: &[SlotSig {
                kind: SlotKind::Style,
                hash: 0xaaa,
            }],
        };
        assert!(matches!(
            OLD.hot_swap_verdict(&SAME_HASH_NEW_KIND),
            Verdict::NeedsRustc(NeedsRustc::NewExpr { .. })
        ));
    }

    /// **重映射才是真正的难点**:新增一个静态节点会让后面的槽位整体后移,
    /// 而运行中的 app 手里只有旧 binders 表。不改写就会拿新号索引旧表 ——
    /// 取到的是别的表达式,界面显示错东西且**不 panic**。Dioxus 卡的就是这一步
    #[test]
    fn hot_remap_to_old_slot_ids() {
        static OLD: Template = Template {
            id: "t#0",
            roots: &[],
            sig: &[
                SlotSig {
                    kind: SlotKind::Text,
                    hash: 0x111,
                },
                SlotSig {
                    kind: SlotKind::Click,
                    hash: 0x222,
                },
            ],
        };
        // 新版在最前面插了一个**新的**静态节点不会改签名表;真正让槽位后移的是
        // 在前面插了一个引用**已有表达式**的插值。这里模拟后者:
        // 新表 = [Click(0x222), Text(0x111)] —— 新号 0/1,旧号 1/0
        static NEW: Template = Template {
            id: "t#0",
            roots: &[
                TNode::Elem {
                    kind: ElementKind::Button,
                    label: "",
                    style: &[],
                    // 新数据面用的是**新编的**槽位号
                    binds: &[Bind::Click(0)],
                    children: &[],
                },
                TNode::Elem {
                    kind: ElementKind::Text,
                    label: "",
                    style: &[],
                    binds: &[Bind::Text(1)],
                    children: &[],
                },
            ],
            sig: &[
                SlotSig {
                    kind: SlotKind::Click,
                    hash: 0x222,
                },
                SlotSig {
                    kind: SlotKind::Text,
                    hash: 0x111,
                },
            ],
        };

        let Verdict::DataOnly { remap } = OLD.hot_swap_verdict(&NEW) else {
            panic!("应判为可热换");
        };
        assert_eq!(remap, vec![1, 0]);

        let remapped = remap_slots(NEW.roots, &remap);
        // 改写后:Click 用旧号 1、Text 用旧号 0 —— 正好接回旧 binders 表
        let slots: Vec<Bind> = remapped
            .iter()
            .flat_map(|n| match n {
                TNode::Elem { binds, .. } => binds.to_vec(),
                TNode::Block { .. } => vec![],
            })
            .collect();
        assert_eq!(slots, vec![Bind::Click(1), Bind::Text(0)]);

        // 真正跑一遍:用**旧顺序**的 binders 表 stamp 改写后的数据面,
        // 文本必须是旧表 0 号那个闭包的产物。不改写的话这里会取到 Click,
        // 种类不符 → 静默跳过 → 文本是空的
        let hits = std::rc::Rc::new(std::cell::Cell::new(0));
        let h = hits.clone();
        let binders: std::rc::Rc<[Binder]> = std::rc::Rc::from(vec![
            Binder::Text(std::rc::Rc::new(|| "来自旧 0 号".to_string())),
            Binder::Click(std::rc::Rc::new(move || h.set(h.get() + 1))),
        ]);
        let doc = Doc::new();
        let (_, _scope) = sv_reactive::create_root(|| {
            stamp(
                &doc,
                doc.root(),
                &Template {
                    id: NEW.id,
                    roots: Box::leak(remapped.into_boxed_slice()),
                    sig: OLD.sig,
                },
                binders,
            );
        });
        assert!(
            doc.dump().contains("来自旧 0 号"),
            "重映射后旧 binder 必须接在正确的位置上;实际:{}",
            doc.dump()
        );
    }

    /// 槽位对不上时**跳过而不是崩**(release 语义)。
    /// debug 下有断言,所以这条测试只在 release 跑
    #[test]
    #[cfg(not(debug_assertions))]
    fn missing_slot_is_skipped_not_panicked() {
        static ROOTS: &[TNode] = &[TNode::Elem {
            kind: ElementKind::Text,
            label: "静态文本还在",
            style: &[],
            // 槽位 7 不存在
            binds: &[Bind::Text(7)],
            children: &[],
        }];
        static TPL: Template = Template {
            id: "test#4",
            roots: ROOTS,
            sig: &[],
        };
        let doc = Doc::new();
        let (_, _s) = create_root(|| stamp(&doc, doc.root(), &TPL, std::rc::Rc::from(vec![])));
        assert!(
            doc.dump().contains("静态文本还在"),
            "越界槽位应只跳过该位,其余照常建树"
        );
    }
}
