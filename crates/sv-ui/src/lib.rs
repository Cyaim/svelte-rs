//! # sv-ui
//!
//! retained 场景树(桌面版的 "DOM")+ 细粒度响应式绑定原语。
//!
//! 这一层是 `view!` 宏的**编译目标**:Svelte 把模板编译成命令式 DOM 操作
//! (`createElement` / `$.template_effect(...)`),我们把模板编译成对 [`Doc`] 的
//! 命令式场景树操作 + [`bind_text`]/[`if_block`]/[`each_block`] 这类绑定调用。
//! 没有虚拟 DOM、没有 diff:哪个值变了,哪个节点被精准更新。
//!
//! 渲染器(sv-shell)只负责把这棵树画出来;树的任何变更会 bump 版本号并触发
//! `on_mutate` 回调(通常接 `request_redraw`)。

use std::cell::RefCell;
use std::rc::Rc;

use slotmap::{SlotMap, new_key_type};
use sv_reactive::{RootHandle, create_root, derived, effect, on_cleanup, untrack};

pub mod anim;
pub mod dirty;
pub mod focus;
pub mod input;
pub mod overlay;
pub mod shortcuts;
pub mod tasks;
/// 模板数据面(ADR-2 ②:生成数据而非生成类型;热重载的承重墙)
pub mod tmpl;

pub use focus::{Key, KeyEvent, KeyPhase, Mods, dispatch_key};
pub use input::{
    Caret, Clipboard, EditOp, ImeEvent, InputState, UndoEntry, apply_edit, clipboard_get,
    clipboard_set, handle_ime, next_word_boundary, prev_word_boundary, set_clipboard,
    word_range_at,
};
pub use overlay::{
    Anchor, CloseBehavior, OverlayEntry, OverlayLayer, OverlayOpts, Side, overlay_block, tooltip,
};
pub use shortcuts::{Shortcut, register_shortcut};
pub use tmpl::{Bind, Binder, SlotKind, SlotSig, StyleDecl, TNode, Template, stamp};

/// 组件 children / 具名 snippet 的类型:接收 (doc, 挂载点) 的可复用构建闭包
pub type Snippet = Rc<dyn Fn(&Doc, ViewId)>;

/// 键盘回调([`Doc::set_on_key`] / [`Doc::key_handler`])
pub type KeyHandler = Rc<dyn Fn(&KeyEvent)>;

new_key_type! {
    pub struct ViewId;
}

// ---------------------------------------------------------------------------
// 样式
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const BLACK: Color = Color::rgb(20, 20, 24);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Direction {
    #[default]
    Column,
    Row,
}

/// 四方向边距(padding/margin;CSS 盒模型的最小载体)
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edges {
    pub const fn all(v: f32) -> Self {
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }
    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }
    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
}

impl From<f32> for Edges {
    fn from(v: f32) -> Self {
        Edges::all(v)
    }
}

/// 边框(v0:实线,单宽单色)
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Border {
    pub width: f32,
    pub color: Color,
}

/// 主轴排布(CSS justify-content 子集;taffy 直通,调研 23)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum JustifyContent {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// 交叉轴对齐(CSS align-items 子集;也用作 align-self 的值)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AlignItems {
    /// CSS 缺省是 stretch;v0 保持既有"顶对齐不拉伸"行为为缺省,
    /// 显式 `align-items: stretch` 才拉伸(迁移零回归优先)
    #[default]
    Start,
    Center,
    End,
    Stretch,
}

/// 换行(flex-wrap 子集)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FlexWrap {
    #[default]
    NoWrap,
    Wrap,
}

/// 文本折行(white-space 子集;Text 叶子用)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TextWrap {
    /// 容器宽内折行(UAX #14 断点;CJK 正确断行)
    #[default]
    Wrap,
    /// 恒单行(Button/Checkbox label 语义)
    NoWrap,
}

/// 文本水平对齐(逐行 x 偏移;justify 永不做,见 CSS-SUPPORT)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// 溢出行为(调研 22:滚动是 View 的正交属性,不是新 ElementKind)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Overflow {
    #[default]
    Visible,
    /// 裁剪但不可滚
    Hidden,
    /// 裁剪 + 滚轮可滚(offset 真源在节点 [`ViewNode::scroll_x`]/`scroll_y`)
    Scroll,
}

/// 鼠标光标(CSS cursor 子集)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cursor {
    Default,
    Pointer,
    Text,
    Grab,
    NotAllowed,
}

/// 极简样式模型(原型阶段够用,后续换 taffy + 完整样式系统)。
/// 继承语义:`fg = None` / `font_size = NAN` 表示"继承"(沿父链解析,
/// 白名单见渲染层 resolve;根 fallback:BLACK / 16.0)
#[derive(Clone, Debug)]
pub struct Style {
    pub direction: Direction,
    pub gap: f32,
    pub padding: Edges,
    pub margin: Edges,
    pub border: Option<Border>,
    pub bg: Option<Color>,
    /// None = 继承
    pub fg: Option<Color>,
    /// NAN = 继承
    pub font_size: f32,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub corner_radius: f32,
    /// 不透明度 0.0-1.0(过渡动画的载体)
    pub opacity: f32,
    pub cursor: Option<Cursor>,
    /// 溢出行为(Hidden/Scroll 的 View 尺寸不被内容撑开,子内容按
    /// scroll 偏移平移并裁剪;调研 22)
    /// 纵轴溢出行为(`overflow-y`);`overflow` 简写同时写两轴
    pub overflow: Overflow,
    /// 横轴溢出行为(`overflow-x`)。分轴的常见用法是"横向裁掉、纵向滚"
    pub overflow_x: Overflow,
    // ---- flex 第一批(调研 23 T3;taffy 直通,渲染层不读) ----
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    /// None = 跟随父的 align_items
    pub align_self: Option<AlignItems>,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_wrap: FlexWrap,
    pub min_width: Option<f32>,
    pub min_height: Option<f32>,
    pub max_width: Option<f32>,
    pub max_height: Option<f32>,
    /// 文本折行(Text 叶子;Button/Checkbox 恒单行)
    pub text_wrap: TextWrap,
    pub text_align: TextAlign,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            direction: Direction::Column,
            gap: 0.0,
            padding: Edges::default(),
            margin: Edges::default(),
            border: None,
            bg: None,
            fg: None,
            font_size: f32::NAN, // 继承
            width: None,
            height: None,
            corner_radius: 0.0,
            opacity: 1.0,
            cursor: None,
            overflow: Overflow::Visible,
            overflow_x: Overflow::Visible,
            justify_content: JustifyContent::Start,
            align_items: AlignItems::Start,
            align_self: None,
            flex_grow: 0.0,
            // CSS 缺省是 1;这里取 0 保持旧引擎"子项不收缩"的行为
            // (迁移零回归优先,与 align_items 缺省 Start 同一裁决;
            // 需要收缩布局时显式 `flex-shrink: 1`)
            flex_shrink: 0.0,
            flex_wrap: FlexWrap::NoWrap,
            min_width: None,
            min_height: None,
            max_width: None,
            max_height: None,
            text_wrap: TextWrap::Wrap,
            text_align: TextAlign::Left,
        }
    }
}

// font_size 的 NAN(继承哨兵)要按"同为 NAN 即相等"比较,
// 否则 set_style 的相等剪枝永远失效、每次重算都 bump 版本
impl PartialEq for Style {
    fn eq(&self, other: &Self) -> bool {
        let fs_eq = (self.font_size.is_nan() && other.font_size.is_nan())
            || self.font_size == other.font_size;
        self.direction == other.direction
            && self.gap == other.gap
            && self.padding == other.padding
            && self.margin == other.margin
            && self.border == other.border
            && self.bg == other.bg
            && self.fg == other.fg
            && fs_eq
            && self.width == other.width
            && self.height == other.height
            && self.corner_radius == other.corner_radius
            && self.opacity == other.opacity
            && self.cursor == other.cursor
            && self.overflow == other.overflow
            && self.overflow_x == other.overflow_x
            && self.justify_content == other.justify_content
            && self.align_items == other.align_items
            && self.align_self == other.align_self
            && self.flex_grow == other.flex_grow
            && self.flex_shrink == other.flex_shrink
            && self.flex_wrap == other.flex_wrap
            && self.min_width == other.min_width
            && self.min_height == other.min_height
            && self.max_width == other.max_width
            && self.max_height == other.max_height
            && self.text_wrap == other.text_wrap
            && self.text_align == other.text_align
    }
}

// ---------------------------------------------------------------------------
// 场景树
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ElementKind {
    /// 容器(相当于 div)
    View,
    /// 文本叶子
    Text,
    /// 按钮叶子(自带 label 文本,可点击)
    Button,
    /// 复选框叶子(勾选状态在 [`ViewNode::checked`],label 复用 text 字段)
    Checkbox,
    /// 单行文本输入(value 复用 text 字段,编辑态在 [`ViewNode::input`])
    TextInput,
}

pub struct ViewNode {
    pub kind: ElementKind,
    /// Text / Button / Checkbox 的文本内容
    pub text: String,
    /// Checkbox 的勾选状态(其它元素恒为 false)
    pub checked: bool,
    /// 可获焦位(Tab 遍历只走 focusable 节点;Button/Checkbox 默认 true)。
    /// opt-in 布尔位、无数值 tabindex——egui/floem/Masonry/Slint 四家交集
    pub focusable: bool,
    /// 获焦即开 IME 会话(Masonry `accepts_text_input` 同款)。
    /// 本切片恒 false,R1 第 2 步 TextInput 用它触发 `set_ime_allowed`
    pub accepts_text: bool,
    pub style: Style,
    pub parent: Option<ViewId>,
    pub children: Vec<ViewId>,
    pub on_click: Option<Rc<dyn Fn()>>,
    pub on_pointer_enter: Option<Rc<dyn Fn()>>,
    pub on_pointer_down: Option<Rc<dyn Fn()>>,
    pub on_pointer_up: Option<Rc<dyn Fn()>>,
    pub on_pointer_leave: Option<Rc<dyn Fn()>>,
    pub on_key: Option<KeyHandler>,
    /// 捕获段回调(root → 焦点,先于冒泡;祖先拦截后代用)
    pub on_key_capture: Option<KeyHandler>,
    /// 焦点变化回调(true=获焦,false=失焦;`:focus` 伪类接线用)
    pub on_focus_change: Option<Rc<dyn Fn(bool)>>,
    /// TextInput 的编辑态(其它元素恒 None;Box 控制节点大小预算)
    pub input: Option<Box<input::InputState>>,
    /// 滚动偏移(overflow: Scroll 的 View;真源在树上,与 checked 同款)
    pub scroll_x: f32,
    pub scroll_y: f32,
    /// 滚动偏移变化回调(新 (x, y);virtual_scroll 桥与 onscroll 的载体)
    pub on_scroll: Option<Rc<dyn Fn(f32, f32)>>,
    /// 虚拟内容尺寸覆盖(virtual_scroll 用:滚动范围/滚动条比例按它算,
    /// 不按实际子树尺寸)
    pub content_override: Option<(f32, f32)>,
    /// 无障碍名称覆盖(`aria-label`;None 时语义树取 text;调研 24 §4.1)
    pub accessible_label: Option<String>,
}

pub struct DocumentInner {
    pub nodes: SlotMap<ViewId, ViewNode>,
    pub root: ViewId,
    /// 弹层注册表(调研 25:游离子树根;注册序即层内叠序,渲染壳把它们的
    /// Placed 追加在基础层之后)
    pub overlays: Vec<overlay::OverlayEntry>,
    /// 单一焦点点(egui/iced/floem/Masonry/Slint 五家共识;不做成 signal——
    /// 它是树状态,经版本号驱动重绘,全局 signal 会破坏细粒度订阅)
    pub focused: Option<ViewId>,
    version: u64,
    on_mutate: Option<Box<dyn Fn()>>,
    /// 本帧的变更日志(见 [`dirty`])。渲染壳 `take_dirty` 走;
    /// 没人取就靠上限兜底,不会无限涨
    dirty: dirty::DirtyLog,
}

/// 场景树句柄。`Clone` 共享同一棵树,可自由塞进事件闭包
#[derive(Clone)]
pub struct Doc(Rc<RefCell<DocumentInner>>);

impl Default for Doc {
    fn default() -> Self {
        Self::new()
    }
}

impl Doc {
    pub fn new() -> Self {
        let mut nodes = SlotMap::with_key();
        let root = nodes.insert(ViewNode {
            kind: ElementKind::View,
            text: String::new(),
            checked: false,
            focusable: false,
            accepts_text: false,
            style: Style::default(),
            parent: None,
            children: Vec::new(),
            on_click: None,
            on_pointer_enter: None,
            on_pointer_down: None,
            on_pointer_up: None,
            on_pointer_leave: None,
            on_key: None,
            on_key_capture: None,
            on_focus_change: None,
            input: None,
            scroll_x: 0.0,
            scroll_y: 0.0,
            on_scroll: None,
            content_override: None,
            accessible_label: None,
        });
        Doc(Rc::new(RefCell::new(DocumentInner {
            nodes,
            root,
            overlays: Vec::new(),
            focused: None,
            version: 0,
            on_mutate: None,
            dirty: dirty::DirtyLog::default(),
        })))
    }

    /// 版本 +1 并记一条**分级过的**变更。
    ///
    /// 参数不是可选的:漏了分级就编译不过。前一版设计是"事后按写入口查表",
    /// 数了一遍才发现全仓有 34 个 `bump` 点而人工整理的表只覆盖了 26 个 ——
    /// 漏一个就画错一帧,且没有任何报错。把它挪进类型系统之后,
    /// "新增写方法忘了定级"从线上 bug 变成编译错误。
    fn bump(&self, item: dirty::DirtyItem) {
        let cb = {
            let mut inner = self.0.borrow_mut();
            inner.version += 1;
            inner.dirty.push(item);
            // 借用释放后再调回调,回调里可以继续操作树
            inner.on_mutate.take()
        };
        if let Some(cb) = cb {
            cb();
            self.0.borrow_mut().on_mutate.get_or_insert(cb);
        }
    }

    pub fn version(&self) -> u64 {
        self.0.borrow().version
    }

    /// 取走并清空这一帧的变更日志。渲染壳每帧调一次。
    ///
    /// 没有消费者时日志靠 [`dirty::DIRTY_LOG_CAP`] 兜底(丢弃 + 置
    /// `overflowed`),所以离屏渲染、单测、直调布局这些没有渲染壳的场景
    /// 不会无限吃内存 —— 代价只是它们始终走全量,而它们本来就走全量。
    pub fn take_dirty(&self) -> dirty::DirtyLog {
        std::mem::take(&mut self.0.borrow_mut().dirty)
    }

    /// 某个节点的 `set_text` 到底是 Measure 还是 Paint —— **取决于 kind**。
    ///
    /// 同一个写方法对 Text/Button 进测量,对 TextInput/Checkbox 完全不进:
    /// 前者的尺寸就是文本尺寸,后者的测量恒定(输入框 `200 × 行高×rows`、
    /// 复选框 `字号见方`)。**所以分级必须读 kind,不能只看方法名。**
    /// 而且只能在**记录时**读 —— 节点删掉之后 kind 就查不回来了。
    fn text_dirty(inner: &DocumentInner, id: ViewId) -> dirty::DirtyItem {
        match inner.nodes.get(id).map(|n| n.kind) {
            // 输入框的 value 复用 text 字段;测量与内容无关。
            // 【做 auto-size input 时这一条要改成 Measure】
            Some(ElementKind::TextInput) | Some(ElementKind::Checkbox) => dirty::DirtyItem::Paint,
            _ => dirty::DirtyItem::Measure { id },
        }
    }

    /// 无障碍名称覆盖(`aria-label` 编译目标;空串清除)
    pub fn set_accessible_label(&self, id: ViewId, label: &str) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            let new = (!label.is_empty()).then(|| label.to_string());
            if n.accessible_label == new {
                return;
            }
            n.accessible_label = new;
        }
        // a11y 名称不进布局也不进绘制几何
        self.bump(dirty::DirtyItem::Paint);
    }

    /// 本树的身份标识(布局/绘制缓存键;同一棵树的所有 Doc 克隆同值)
    pub fn identity(&self) -> usize {
        Rc::as_ptr(&self.0) as usize
    }

    /// 树被修改后的回调(渲染壳用它接 request_redraw)
    pub fn set_on_mutate(&self, f: impl Fn() + 'static) {
        self.0.borrow_mut().on_mutate = Some(Box::new(f));
    }

    pub fn root(&self) -> ViewId {
        self.0.borrow().root
    }

    /// 只读访问整棵树(渲染器遍历用)
    pub fn read<R>(&self, f: impl FnOnce(&DocumentInner) -> R) -> R {
        f(&self.0.borrow())
    }

    fn create(&self, kind: ElementKind, text: &str) -> ViewId {
        let id = self.0.borrow_mut().nodes.insert(ViewNode {
            kind,
            text: text.to_string(),
            checked: false,
            // 交互叶子默认可获焦(floem 教训:不自动设位是新手第一坑);
            // View/Text 用 set_focusable 手动开
            focusable: matches!(
                kind,
                ElementKind::Button | ElementKind::Checkbox | ElementKind::TextInput
            ),
            // 获焦即开 IME 会话的位(Masonry accepts_text_input 同款)
            accepts_text: kind == ElementKind::TextInput,
            style: Style::default(),
            parent: None,
            children: Vec::new(),
            on_click: None,
            on_pointer_enter: None,
            on_pointer_down: None,
            on_pointer_up: None,
            on_pointer_leave: None,
            on_key: None,
            on_key_capture: None,
            on_focus_change: None,
            input: (kind == ElementKind::TextInput).then(Default::default),
            scroll_x: 0.0,
            scroll_y: 0.0,
            on_scroll: None,
            content_override: None,
            accessible_label: None,
        });
        // **游离节点**:还没 append,不在任何父的 children 里,布局上什么都不做。
        // 显式定为 Paint 而不是漏掉,是因为一次 `{#each}` 建表会 create+append
        // 各一条 —— 若这里也记结构脏,日志预算白白少一半、且每条都是假的
        self.bump(dirty::DirtyItem::Paint);
        id
    }

    pub fn create_view(&self) -> ViewId {
        self.create(ElementKind::View, "")
    }

    pub fn create_text(&self, initial: &str) -> ViewId {
        self.create(ElementKind::Text, initial)
    }

    pub fn create_button(&self, label: &str) -> ViewId {
        self.create(ElementKind::Button, label)
    }

    /// label 用 [`Doc::set_text`] 设置,勾选状态用 [`Doc::set_checked`]
    pub fn create_checkbox(&self) -> ViewId {
        self.create(ElementKind::Checkbox, "")
    }

    /// 单行文本输入(value 复用 text 字段;focusable + accepts_text 默认开,
    /// 编辑操作走 [`input::apply_edit`],IME 走 [`input::handle_ime`])
    pub fn create_text_input(&self) -> ViewId {
        self.create(ElementKind::TextInput, "")
    }

    /// 可变访问树(crate 内部:编辑内核用;借用期间不得调用户回调)
    pub(crate) fn with_inner_mut<R>(&self, f: impl FnOnce(&mut DocumentInner) -> R) -> R {
        f(&mut self.0.borrow_mut())
    }

    pub fn append(&self, parent: ViewId, child: ViewId) {
        let from = {
            let mut inner = self.0.borrow_mut();
            let from = inner.nodes[child].parent;
            if let Some(op) = from {
                inner.nodes[op].children.retain(|c| *c != child);
            }
            inner.nodes[child].parent = Some(parent);
            inner.nodes[parent].children.push(child);
            from
        };
        // **旧父必须现在就记下来** —— 日志被消费时 Doc 里只剩新父了,
        // 消费端无从知道该把节点从哪儿摘走(taffy 的 `add_child` 不摘旧父,
        // 于是同一个节点会挂在两个父下、被布局两次)
        self.bump(dirty::DirtyItem::Structure {
            id: child,
            from,
            to: Some(parent),
        });
        // 换了父就可能换了继承字号:子树里所有"自己没设字号"的叶子的测量都变了,
        // 而它们自己一个字段都没动。这条与 `set_style(font_size)` 是同一条路径,
        // 只记结构会漏掉它
        self.bump(dirty::DirtyItem::InheritFontSize {
            subtree_root: child,
        });
    }

    /// 摘除并递归销毁整棵子树
    pub fn remove(&self, id: ViewId) {
        let (blur_cb, parent) = {
            let mut inner = self.0.borrow_mut();
            // 被删子树含焦点节点:清焦点并留住失焦回调(否则 if_block 重建
            // 会留下悬空焦点)。回调在借用释放后再调
            let mut blur_cb = None;
            if let Some(f) = inner.focused {
                let mut cur = Some(f);
                let contained = loop {
                    match cur {
                        Some(c) if c == id => break true,
                        Some(c) => cur = inner.nodes.get(c).and_then(|n| n.parent),
                        None => break false,
                    }
                };
                if contained {
                    blur_cb = inner.nodes.get(f).and_then(|n| n.on_focus_change.clone());
                    inner.focused = None;
                }
            }
            let parent = inner.nodes.get(id).and_then(|n| n.parent);
            if let Some(p) = parent {
                inner.nodes[p].children.retain(|c| *c != id);
            }
            fn drop_subtree(inner: &mut DocumentInner, id: ViewId) {
                if let Some(n) = inner.nodes.remove(id) {
                    for c in n.children {
                        drop_subtree(inner, c);
                    }
                }
            }
            drop_subtree(&mut inner, id);
            (blur_cb, parent)
        };
        if let Some(cb) = blur_cb {
            cb(false);
        }
        // `to: None` = 删除。父在借用块里就取好了(出了那个块节点已从
        // slotmap 消失,`nodes.get(id)` 只会返回 None)
        self.bump(dirty::DirtyItem::Structure {
            id,
            from: parent,
            to: None,
        });
    }

    /// 清空容器的所有子节点(if/each 块重建用)
    pub fn clear_children(&self, id: ViewId) {
        let children = {
            let inner = self.0.borrow();
            match inner.nodes.get(id) {
                Some(n) => n.children.clone(),
                None => return,
            }
        };
        for c in children {
            self.remove(c);
        }
    }

    pub fn set_text(&self, id: ViewId, text: &str) {
        let item = {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            if n.text == text {
                return;
            }
            n.text = text.to_string();
            Self::text_dirty(&inner, id)
        };
        self.bump(item);
    }

    pub fn set_checked(&self, id: ViewId, checked: bool) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            if n.checked == checked {
                return; // 相等不 bump:渲染端不用白白重绘
            }
            n.checked = checked;
        }
        // 复选框的测量只看字号(方框恒为字号见方),勾不勾是纯绘制
        self.bump(dirty::DirtyItem::Paint);
    }

    pub fn set_style(&self, id: ViewId, style: Style) {
        let (relevant, font_size_changed) = {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            if n.style == style {
                return;
            }
            let relevant = dirty::layout_relevant(&n.style, &style);
            let font_size_changed = !dirty::font_size_eq(n.style.font_size, style.font_size);
            n.style = style;
            (relevant, font_size_changed)
        };
        self.bump(if relevant {
            dirty::DirtyItem::Measure { id }
        } else {
            dirty::DirtyItem::Paint
        });
        if font_size_changed {
            // 字号继承在建树时就地解析,taffy 不知道它 —— 改一个 View 的字号,
            // 子树里所有没设字号的叶子都要重测。**这是整套分级里最容易漏的一条**
            self.bump(dirty::DirtyItem::InheritFontSize { subtree_root: id });
        }
    }

    /// 原地修改样式(比整体替换省事)
    pub fn update_style(&self, id: ViewId, f: impl FnOnce(&mut Style)) {
        let (relevant, font_size_changed) = {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            // 这里以前**无条件 bump**(不像 set_style 比了 PartialEq),
            // 于是平滑滚动这类"每帧调一次 update_style、多数帧其实没变"的
            // 用法在制造假脏帧。先留旧值再比,不等才 bump
            let before = n.style.clone();
            f(&mut n.style);
            if before == n.style {
                return;
            }
            (
                dirty::layout_relevant(&before, &n.style),
                !dirty::font_size_eq(before.font_size, n.style.font_size),
            )
        };
        self.bump(if relevant {
            dirty::DirtyItem::Measure { id }
        } else {
            dirty::DirtyItem::Paint
        });
        if font_size_changed {
            self.bump(dirty::DirtyItem::InheritFontSize { subtree_root: id });
        }
    }

    pub fn set_on_click(&self, id: ViewId, f: impl Fn() + 'static) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            n.on_click = Some(Rc::new(f));
        }
        // 注册回调不改任何几何
        self.bump(dirty::DirtyItem::Paint);
    }

    /// 取出点击回调(clone 出来调用,避免调用期间持有树的借用)
    pub fn click_handler(&self, id: ViewId) -> Option<Rc<dyn Fn()>> {
        self.0
            .borrow()
            .nodes
            .get(id)
            .and_then(|n| n.on_click.clone())
    }

    pub fn set_on_pointer_enter(&self, id: ViewId, f: impl Fn() + 'static) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            n.on_pointer_enter = Some(Rc::new(f));
        }
        // 同上
        self.bump(dirty::DirtyItem::Paint);
    }

    pub fn set_on_pointer_leave(&self, id: ViewId, f: impl Fn() + 'static) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            n.on_pointer_leave = Some(Rc::new(f));
        }
        // 同上
        self.bump(dirty::DirtyItem::Paint);
    }

    /// 取出悬停进入回调(同 [`Doc::click_handler`]:clone 出来调用,不持树借用)
    pub fn pointer_enter_handler(&self, id: ViewId) -> Option<Rc<dyn Fn()>> {
        self.0
            .borrow()
            .nodes
            .get(id)
            .and_then(|n| n.on_pointer_enter.clone())
    }

    /// 取出悬停离开回调
    pub fn pointer_leave_handler(&self, id: ViewId) -> Option<Rc<dyn Fn()>> {
        self.0
            .borrow()
            .nodes
            .get(id)
            .and_then(|n| n.on_pointer_leave.clone())
    }

    pub fn set_on_pointer_down(&self, id: ViewId, f: impl Fn() + 'static) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            n.on_pointer_down = Some(Rc::new(f));
        }
        // 同上
        self.bump(dirty::DirtyItem::Paint);
    }

    pub fn set_on_pointer_up(&self, id: ViewId, f: impl Fn() + 'static) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            n.on_pointer_up = Some(Rc::new(f));
        }
        // 同上
        self.bump(dirty::DirtyItem::Paint);
    }

    pub fn pointer_down_handler(&self, id: ViewId) -> Option<Rc<dyn Fn()>> {
        self.0
            .borrow()
            .nodes
            .get(id)
            .and_then(|n| n.on_pointer_down.clone())
    }

    pub fn pointer_up_handler(&self, id: ViewId) -> Option<Rc<dyn Fn()>> {
        self.0
            .borrow()
            .nodes
            .get(id)
            .and_then(|n| n.on_pointer_up.clone())
    }

    // -----------------------------------------------------------------------
    // 焦点系统(调研 20;单一焦点点 + opt-in focusable 位 + 树序 Tab 遍历)
    // -----------------------------------------------------------------------

    /// 父节点(键盘冒泡沿它上行)
    pub fn parent(&self, id: ViewId) -> Option<ViewId> {
        self.0.borrow().nodes.get(id).and_then(|n| n.parent)
    }

    /// 设置可获焦位(编译器 onkeydown 自动开;View/Text 手动开)
    pub fn set_focusable(&self, id: ViewId, focusable: bool) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            if n.focusable == focusable {
                return;
            }
            n.focusable = focusable;
        }
        // 可获焦位不进 to_taffy
        self.bump(dirty::DirtyItem::Paint);
    }

    pub fn focusable(&self, id: ViewId) -> bool {
        self.0.borrow().nodes.get(id).is_some_and(|n| n.focusable)
    }

    /// 获焦即开 IME 会话的位(R1 第 2 步 TextInput 用;本切片只预留)
    pub fn set_accepts_text(&self, id: ViewId, accepts: bool) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            if n.accepts_text == accepts {
                return;
            }
            n.accepts_text = accepts;
        }
        // 同上
        self.bump(dirty::DirtyItem::Paint);
    }

    pub fn set_on_key(&self, id: ViewId, f: impl Fn(&KeyEvent) + 'static) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            n.on_key = Some(Rc::new(f));
        }
        // 同上
        self.bump(dirty::DirtyItem::Paint);
    }

    /// 取出键盘回调(同 [`Doc::click_handler`]:clone 出来调用,不持树借用)
    pub fn key_handler(&self, id: ViewId) -> Option<KeyHandler> {
        self.0.borrow().nodes.get(id).and_then(|n| n.on_key.clone())
    }

    /// 捕获段回调:**先于**冒泡、从 root 往焦点走(DOM capture 语义)。
    /// 用途是祖先要在后代之前拦截(快捷键守卫、模态吞键)
    pub fn set_on_key_capture(&self, id: ViewId, f: impl Fn(&KeyEvent) + 'static) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            n.on_key_capture = Some(Rc::new(f));
        }
        // 同上
        self.bump(dirty::DirtyItem::Paint);
    }

    pub fn key_capture_handler(&self, id: ViewId) -> Option<KeyHandler> {
        self.0
            .borrow()
            .nodes
            .get(id)
            .and_then(|n| n.on_key_capture.clone())
    }

    pub fn set_on_focus_change(&self, id: ViewId, f: impl Fn(bool) + 'static) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            n.on_focus_change = Some(Rc::new(f));
        }
        // 同上
        self.bump(dirty::DirtyItem::Paint);
    }

    pub fn focus_change_handler(&self, id: ViewId) -> Option<Rc<dyn Fn(bool)>> {
        self.0
            .borrow()
            .nodes
            .get(id)
            .and_then(|n| n.on_focus_change.clone())
    }

    /// 当前焦点节点(单一焦点点,per-Doc 天然 per-window)
    pub fn focused(&self) -> Option<ViewId> {
        self.0.borrow().focused
    }

    /// 移焦到指定节点:先旧节点失焦回调、再新节点获焦回调、bump 一次
    /// (相等剪枝;节点不存在则不动)
    pub fn focus(&self, id: ViewId) {
        let (old_cb, new_cb) = {
            let mut inner = self.0.borrow_mut();
            if inner.focused == Some(id) || inner.nodes.get(id).is_none() {
                return;
            }
            let old = inner.focused.replace(id);
            let old_cb = old
                .and_then(|o| inner.nodes.get(o))
                .and_then(|n| n.on_focus_change.clone());
            let new_cb = inner.nodes.get(id).and_then(|n| n.on_focus_change.clone());
            (old_cb, new_cb)
        };
        if let Some(cb) = old_cb {
            cb(false);
        }
        if let Some(cb) = new_cb {
            cb(true);
        }
        // 焦点环是渲染壳合成绘制,不进树
        self.bump(dirty::DirtyItem::Paint);
    }

    /// 清焦点(Esc 的默认行为)
    pub fn blur(&self) {
        let old_cb = {
            let mut inner = self.0.borrow_mut();
            let Some(old) = inner.focused.take() else {
                return;
            };
            inner.nodes.get(old).and_then(|n| n.on_focus_change.clone())
        };
        if let Some(cb) = old_cb {
            cb(false);
        }
        // 同上
        self.bump(dirty::DirtyItem::Paint);
    }

    /// 树 DFS 序收集所有 focusable 节点(与 `Placed` 绘制序同构 → Tab 序
    /// 即视觉序;隐藏分支被 if_block 物理移除,天然不在结果里)
    fn focusables(&self) -> Vec<ViewId> {
        fn walk(inner: &DocumentInner, id: ViewId, out: &mut Vec<ViewId>) {
            let Some(n) = inner.nodes.get(id) else {
                return;
            };
            if n.focusable {
                out.push(id);
            }
            for c in &n.children {
                walk(inner, *c, out);
            }
        }
        let inner = self.0.borrow();
        let mut out = Vec::new();
        // 焦点陷阱(调研 25 O3):有 modal 弹层时,Tab 环限定在最上层
        // modal 子树内;否则基础层 + 各弹层按注册序
        if let Some(top_modal) = inner.overlays.iter().rev().find(|e| e.modal) {
            walk(&inner, top_modal.root, &mut out);
            return out;
        }
        walk(&inner, inner.root, &mut out);
        for e in &inner.overlays {
            if e.layer == overlay::OverlayLayer::Popup {
                walk(&inner, e.root, &mut out);
            }
        }
        out
    }

    /// Tab:焦点移到树序下一个 focusable(环绕;无焦点时落到第一个)
    pub fn focus_next(&self) {
        let list = self.focusables();
        if list.is_empty() {
            return;
        }
        let next = match self
            .focused()
            .and_then(|f| list.iter().position(|&x| x == f))
        {
            Some(i) => list[(i + 1) % list.len()],
            None => list[0],
        };
        self.focus(next);
    }

    /// Shift+Tab:焦点移到树序上一个 focusable(环绕;无焦点时落到最后一个)
    pub fn focus_prev(&self) {
        let list = self.focusables();
        if list.is_empty() {
            return;
        }
        let prev = match self
            .focused()
            .and_then(|f| list.iter().position(|&x| x == f))
        {
            Some(i) => list[(i + list.len() - 1) % list.len()],
            None => *list.last().unwrap(),
        };
        self.focus(prev);
    }

    // -----------------------------------------------------------------------
    // TextInput(调研 21;编辑操作在 input 模块,这里是树侧存取器)
    // -----------------------------------------------------------------------

    pub fn set_placeholder(&self, id: ViewId, placeholder: &str) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(input) = inner.nodes.get_mut(id).and_then(|n| n.input.as_deref_mut()) else {
                return;
            };
            if input.placeholder == placeholder {
                return;
            }
            input.placeholder = placeholder.to_string();
        }
        // 输入框测量恒为 200×行高×rows,与内容/占位符无关
        self.bump(dirty::DirtyItem::Paint);
    }

    /// 多行模式(`<textarea>`):Enter 换行、粘贴保留换行、按内容宽折行。
    /// `rows` 是可见行数(布局高度 = rows × 行高)
    pub fn set_multiline(&self, id: ViewId, multiline: bool, rows: u16) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(input) = inner.nodes.get_mut(id).and_then(|n| n.input.as_deref_mut()) else {
                return;
            };
            if input.multiline == multiline && input.rows == rows {
                return;
            }
            input.multiline = multiline;
            input.rows = rows.max(1);
        }
        // rows 进 MeasureCtx:多行输入框的高 = rows × 行高,真布局脏
        self.bump(dirty::DirtyItem::Measure { id });
    }

    /// 输入框当前值(非 TextInput 返回 None)
    pub fn input_value(&self, id: ViewId) -> Option<String> {
        self.0
            .borrow()
            .nodes
            .get(id)
            .filter(|n| n.kind == ElementKind::TextInput)
            .map(|n| n.text.clone())
    }

    /// bind:value 写端:相等剪枝;外部写入清预编辑(调研 21 风险 5 裁决)、
    /// 光标钳制到新值内的 char 边界——防 effect↔回调回声
    pub fn set_input_value(&self, id: ViewId, value: &str) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            if n.kind != ElementKind::TextInput || n.text == value {
                return;
            }
            n.text = value.to_string();
            if let Some(input) = n.input.as_deref_mut() {
                input.preedit = None;
                input.cursor = snap_boundary(&n.text, input.cursor);
                input.anchor = snap_boundary(&n.text, input.anchor);
                // 程序化赋值不进撤销栈(浏览器 input 同款):否则 Ctrl+Z
                // 会把外部写入回滚成用户从没打过的中间态
                input.clear_history();
            }
        }
        // 同上 —— 这是打字帧的大红利
        self.bump(dirty::DirtyItem::Paint);
    }

    /// 当前选中文本(无选区返回 Some("");非 TextInput 返回 None)
    pub fn selected_text(&self, id: ViewId) -> Option<String> {
        let inner = self.0.borrow();
        let n = inner.nodes.get(id)?;
        let input = n.input.as_deref()?;
        let (lo, hi) = (
            input.cursor.min(input.anchor),
            input.cursor.max(input.anchor),
        );
        Some(n.text[lo..hi].to_string())
    }

    /// 定光标(渲染壳点击命中后换算字节偏移调用;extend = 拖选)
    pub fn set_caret(&self, id: ViewId, byte: usize, extend: bool) {
        input::apply_edit(self, id, input::EditOp::MoveTo(byte, extend));
    }

    /// 选中字节区间(渲染壳三击全选走这里)
    pub fn select_range(&self, id: ViewId, lo: usize, hi: usize) {
        input::apply_edit(self, id, input::EditOp::SelectRange(lo, hi));
    }

    /// 选中 `byte` 处的词(渲染壳双击选词;词规则见 [`input::word_range_at`])
    pub fn select_word_at(&self, id: ViewId, byte: usize) {
        let Some(text) = self.input_value(id) else {
            return;
        };
        let (lo, hi) = input::word_range_at(&text, byte);
        self.select_range(id, lo, hi);
    }

    pub fn set_on_input(&self, id: ViewId, f: impl Fn(&str) + 'static) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(input) = inner.nodes.get_mut(id).and_then(|n| n.input.as_deref_mut()) else {
                return;
            };
            input.on_input = Some(Rc::new(f));
        }
        // 同上
        self.bump(dirty::DirtyItem::Paint);
    }

    pub fn set_on_submit(&self, id: ViewId, f: impl Fn(&str) + 'static) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(input) = inner.nodes.get_mut(id).and_then(|n| n.input.as_deref_mut()) else {
                return;
            };
            input.on_submit = Some(Rc::new(f));
        }
        // 同上
        self.bump(dirty::DirtyItem::Paint);
    }

    // -----------------------------------------------------------------------
    // 滚动系统(调研 22;offset 真源在节点内,Signal 只作可选桥)
    // -----------------------------------------------------------------------

    /// 写滚动偏移(负值钳到 0;上界钳制由布局侧调用方负责——内容尺寸
    /// 只有布局知道)。相等剪枝;变化时先调 on_scroll 再 bump
    pub fn set_scroll(&self, id: ViewId, x: f32, y: f32) {
        let (x, y) = (x.max(0.0), y.max(0.0));
        let cb = {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            if n.scroll_x == x && n.scroll_y == y {
                return;
            }
            n.scroll_x = x;
            n.scroll_y = y;
            n.on_scroll.clone()
        };
        if let Some(cb) = cb {
            cb(x, y);
        }
        // 滚动偏移根本不进 taffy —— 它是产出坐标时对子原点的一次平移。
        // 布局树整棵可以复用
        self.bump(dirty::DirtyItem::Position { id });
    }

    pub fn scroll_of(&self, id: ViewId) -> (f32, f32) {
        self.0
            .borrow()
            .nodes
            .get(id)
            .map_or((0.0, 0.0), |n| (n.scroll_x, n.scroll_y))
    }

    pub fn set_on_scroll(&self, id: ViewId, f: impl Fn(f32, f32) + 'static) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            n.on_scroll = Some(Rc::new(f));
        }
        // 同上
        self.bump(dirty::DirtyItem::Paint);
    }

    pub fn scroll_handler(&self, id: ViewId) -> Option<Rc<dyn Fn(f32, f32)>> {
        self.0
            .borrow()
            .nodes
            .get(id)
            .and_then(|n| n.on_scroll.clone())
    }

    /// 虚拟内容尺寸覆盖(virtual_scroll 桥用;None = 按实际子树尺寸)
    pub fn set_content_override(&self, id: ViewId, size: Option<(f32, f32)>) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else {
                return;
            };
            if n.content_override == size {
                return;
            }
            n.content_override = size;
        }
        // 只影响产出坐标时的 ScrollArea content/max 与钳制,不进 taffy
        self.bump(dirty::DirtyItem::Position { id });
    }

    /// 调试:把树 dump 成缩进文本
    pub fn dump(&self) -> String {
        fn walk(inner: &DocumentInner, id: ViewId, depth: usize, out: &mut String) {
            let n = &inner.nodes[id];
            let pad = "  ".repeat(depth);
            match n.kind {
                ElementKind::View => out.push_str(&format!("{pad}<view>\n")),
                ElementKind::Text => out.push_str(&format!("{pad}\"{}\"\n", n.text)),
                ElementKind::Button => out.push_str(&format!("{pad}[button \"{}\"]\n", n.text)),
                ElementKind::Checkbox => out.push_str(&format!(
                    "{pad}{} \"{}\"\n",
                    if n.checked { "[x]" } else { "[ ]" },
                    n.text
                )),
                ElementKind::TextInput => out.push_str(&format!("{pad}[input \"{}\"]\n", n.text)),
            }
            for c in &n.children {
                walk(inner, *c, depth + 1, out);
            }
        }
        let inner = self.0.borrow();
        let mut out = String::new();
        walk(&inner, inner.root, 0, &mut out);
        for e in &inner.overlays {
            out.push_str(&format!(
                "== overlay {:?}{} ==
",
                e.layer,
                if e.modal { " (modal)" } else { "" }
            ));
            walk(&inner, e.root, 1, &mut out);
        }
        out
    }
}

/// ViewId → u64(含世代号;AccessKit NodeId 稳定映射,节点删除后不复用)
pub fn view_id_ffi(id: ViewId) -> u64 {
    use slotmap::Key;
    id.data().as_ffi()
}

/// u64 → ViewId(AccessKit 动作回派的反查;世代不符时后续 get 自然落空)
pub fn view_id_from_ffi(raw: u64) -> ViewId {
    use slotmap::KeyData;
    KeyData::from_ffi(raw).into()
}

/// 把字节偏移吸附到 ≤ 它的最近 char 边界(并钳制进字符串长度)
fn snap_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

// ---------------------------------------------------------------------------
// 响应式绑定原语(view! 宏的编译输出会调用这些)
// ---------------------------------------------------------------------------

/// 文本节点绑定:`f` 里读到的任何 signal/derived 变化时,精准更新这个文本节点
pub fn bind_text(doc: &Doc, id: ViewId, f: impl Fn() -> String + 'static) {
    let doc = doc.clone();
    effect(move || {
        let s = f();
        doc.set_text(id, &s);
    });
}

/// 样式绑定:依赖变化时重算该节点样式
pub fn bind_style(doc: &Doc, id: ViewId, f: impl Fn(&mut Style) + 'static) {
    let doc = doc.clone();
    effect(move || {
        let mut style = Style::default();
        f(&mut style);
        doc.set_style(id, style);
    });
}

/// `{#if cond} ... {:else} ...{/if}`
///
/// 在 parent 下放一个稳定的锚点容器;cond 经过 derived 做相等剪枝,只有
/// 真值翻转时才重建分支。分支内创建的 signal/effect 挂在重建 effect 的
/// 作用域下,重建时自动销毁——与 Svelte 的块级作用域语义一致。
pub fn if_block(
    doc: &Doc,
    parent: ViewId,
    cond: impl Fn() -> bool + 'static,
    then_b: impl Fn(&Doc, ViewId) + 'static,
    else_b: impl Fn(&Doc, ViewId) + 'static,
) {
    let container = doc.create_view();
    doc.append(parent, container);
    let c = derived(cond);
    let doc = doc.clone();
    effect(move || {
        doc.clear_children(container);
        if c.get() {
            then_b(&doc, container);
        } else {
            else_b(&doc, container);
        }
        let d = doc.clone();
        on_cleanup(move || d.clear_children(container));
    });
}

/// `{#each items as item, i} ... {/each}`
///
/// 原型版:列表值(PartialEq)变化时整块重建。每行的构建闭包运行在重建
/// effect 的作用域里,行内 signal/effect 会被正确回收。
/// TODO:keyed diff(移动/复用行作用域),这是后续必做的优化。
pub fn each_block<T: Clone + PartialEq + 'static>(
    doc: &Doc,
    parent: ViewId,
    items: impl Fn() -> Vec<T> + 'static,
    row: impl Fn(&Doc, ViewId, &T, usize) + 'static,
) {
    each_block_else(doc, parent, items, row, |_, _| {});
}

/// `{#each items as item, i} ... {:else} 空状态 {/each}`
pub fn each_block_else<T: Clone + PartialEq + 'static>(
    doc: &Doc,
    parent: ViewId,
    items: impl Fn() -> Vec<T> + 'static,
    row: impl Fn(&Doc, ViewId, &T, usize) + 'static,
    empty: impl Fn(&Doc, ViewId) + 'static,
) {
    let container = doc.create_view();
    doc.append(parent, container);
    let list = derived(items);
    let doc = doc.clone();
    effect(move || {
        doc.clear_children(container);
        list.with(|items| {
            if items.is_empty() {
                empty(&doc, container);
            } else {
                for (i, item) in items.iter().enumerate() {
                    row(&doc, container, item, i);
                }
            }
        });
        let d = doc.clone();
        on_cleanup(move || d.clear_children(container));
    });
}

/// `{#each items as item (key)} ... {/each}` —— **keyed** 版本。
///
/// 行按 key 复用:列表变化时,key 相同的行**不重建**,其场景子树与行内
/// 响应式状态(signal/effect)原样保留,只按新顺序重排;新 key 建行、
/// 消失的 key 销毁行作用域。这是 Svelte keyed each 的核心语义。
///
/// v0 约定:同 key 即同身份——行内容要随 item 变化的部分应放 signal;
/// key 重复时后者按新行处理(行为等同 Svelte 的 duplicate key 未定义域)。
pub fn each_block_keyed<T, K>(
    doc: &Doc,
    parent: ViewId,
    items: impl Fn() -> Vec<T> + 'static,
    key_of: impl Fn(&T) -> K + 'static,
    row: impl Fn(&Doc, ViewId, sv_reactive::Signal<T>) + 'static,
) where
    T: Clone + PartialEq + 'static,
    K: PartialEq + Clone + 'static,
{
    let container = doc.create_view();
    doc.append(parent, container);
    let doc = doc.clone();
    // 行的宿主作用域:建在**调用方**的 owner 链上(context 可达),且不在
    // 下面那个 effect 名下 —— 否则列表一变,effect 重跑会先销毁子树,
    // 把所有行的 signal/effect 一起带走(表现为"复用的行从此不再更新")
    let (_, host) = create_root(|| ());
    // 行注册表活在 effect 之外:重跑时复用而不是销毁。
    // 每行带一个 `Signal<T>`(ADR-7 目标形态):**内容变化走原地 set**,
    // reconcile 只管 key 的增删移 —— 否则同 key 换内容的行会一直显示旧数据
    type Rows<K, T> = Rc<RefCell<Vec<(K, ViewId, RootHandle, sv_reactive::Signal<T>)>>>;
    let rows: Rows<K, T> = Rc::new(RefCell::new(Vec::new()));
    let rows_cleanup = rows.clone();
    let doc_cleanup = doc.clone();

    effect(move || {
        let new_items = items(); // 追踪列表本身
        let mut old: Vec<(K, ViewId, RootHandle, sv_reactive::Signal<T>)> =
            rows.borrow_mut().drain(..).collect();
        let mut new_rows = Vec::with_capacity(new_items.len());
        for item in &new_items {
            let k = untrack(|| key_of(item));
            if let Some(pos) = old.iter().position(|(ok, _, _, _)| *ok == k) {
                // 复用:先不动位置,顺序统一在末尾对齐(见下)
                let entry = old.remove(pos);
                // 内容变了才 set:等值不写 = 行内绑定不重跑(纯重排零开销)。
                // 读写都 untrack —— 行信号绝不能成为本 effect 的依赖,
                // 否则改一行会重跑整个列表 reconcile
                let changed = untrack(|| entry.3.with(|old_item| old_item != item));
                if changed {
                    let v = item.clone();
                    untrack(|| entry.3.set(v));
                }
                new_rows.push(entry);
            } else {
                let cont = doc.create_view();
                doc.append(container, cont);
                let d = doc.clone();
                let item = item.clone();
                // 行作用域独立于本 effect(不随列表变化销毁);行信号建在该
                // 作用域内 —— 行销毁时一并释放。
                // untrack:行构建期的散读不应订阅到本 effect 上
                // 闭包不能 move:`row` 是被外层 effect 捕获的 `Fn`,借用即可
                let (sig, scope) = sv_reactive::with_owner(&host, || {
                    create_root(|| {
                        let sig = sv_reactive::state(item.clone());
                        untrack(|| row(&d, cont, sig));
                        sig
                    })
                });
                new_rows.push((k, cont, scope, sig));
            }
        }
        for (_, cont, scope, _) in old {
            scope.dispose();
            doc.remove(cont);
        }
        // 顺序对齐:**只在真的乱序时**才动树。逐行 append 虽然结果正确,
        // 但每次都会 bump 版本号触发重绘 —— 而"内容变、顺序没变"是列表最
        // 常见的更新形态,不该为它重排一遍
        let desired: Vec<ViewId> = new_rows.iter().map(|(_, id, _, _)| *id).collect();
        let in_order = doc.read(|inner| {
            inner
                .nodes
                .get(container)
                .is_some_and(|c| c.children == desired)
        });
        if !in_order {
            // append 即"移到末尾":按新序逐个 append 完成重排
            for id in &desired {
                doc.append(container, *id);
            }
        }
        *rows.borrow_mut() = new_rows;
    });

    // 整块卸载时销毁所有行作用域
    on_cleanup(move || {
        for (_, cont, scope, _) in rows_cleanup.borrow_mut().drain(..) {
            scope.dispose();
            doc_cleanup.remove(cont);
        }
    });
}

/// 虚拟列表(百万级列表的正解,ADR/调研 18):**逻辑 N 行、实例化只有视口**。
///
/// 结构:固定 `viewport_rows` 个槽位,每槽一次性建行 + 一个 `Signal<Option<T>>`;
/// 滚动(offset 变化)= 逐槽 `set` 新数据 → 行内绑定原地更新,
/// **零节点创建/销毁、零布局结构变化**——这是 1% low 稳定性的来源。
/// `item_at` 惰性取数:总量百万也不物化整表。
/// 槽值为 `None` 表示越界空槽(行内自行渲染空态)。
pub fn virtual_list<T: Clone + 'static>(
    doc: &Doc,
    parent: ViewId,
    count: impl Fn() -> usize + 'static,
    offset: sv_reactive::Signal<usize>,
    viewport_rows: usize,
    item_at: impl Fn(usize) -> T + 'static,
    row: impl Fn(&Doc, ViewId, sv_reactive::Signal<Option<T>>, usize) + 'static,
) {
    use sv_reactive::state;

    let container = doc.create_view();
    doc.append(parent, container);

    // 槽位一次性建行(行内容绑定到槽信号,之后只走数据更新)
    let mut slots: Vec<sv_reactive::Signal<Option<T>>> = Vec::with_capacity(viewport_rows);
    for i in 0..viewport_rows {
        let slot = state::<Option<T>>(None);
        row(doc, container, slot, i);
        slots.push(slot);
    }

    // 数据填充:offset/count 任一变化 → 逐槽写入(细粒度,无结构变化)
    effect(move || {
        let off = offset.get();
        let n = count();
        for (i, slot) in slots.iter().enumerate() {
            let idx = off + i;
            slot.set(if idx < n { Some(item_at(idx)) } else { None });
        }
    });
}

/// `bind:scrolly` 的编译目标:Signal ↔ 纵向滚动偏移双向桥。
/// signal 写 → set_scroll(相等剪枝防回声);滚动(滚轮/程序)→ signal 更新。
/// 既有 on_scroll 回调被链式保留(桥后挂;编译器保证 onscroll 先于本桥发射)
pub fn bind_scroll_y(doc: &Doc, id: ViewId, sig: sv_reactive::Signal<f32>) {
    let d = doc.clone();
    effect(move || {
        let y = sig.get();
        let (x, _) = d.scroll_of(id);
        d.set_scroll(id, x, y);
    });
    let prev = doc.scroll_handler(id);
    doc.set_on_scroll(id, move |x, y| {
        sig.set(y);
        if let Some(p) = &prev {
            p(x, y);
        }
    });
}

/// virtual_list 与真实滚动输入的合流桥(调研 22 §2.6):
/// ① 维护容器的虚拟内容高度 `count × row_h`(滚动范围/滚动条比例由它决定);
/// ② 滚动偏移(像素域)→ 行号(`offset` 槽信号)——像素到行域的唯一换算点。
/// 用法:`overflow: Scroll` 容器(显式高)内放 [`virtual_list`] 槽位,再调本桥
pub fn virtual_scroll(
    doc: &Doc,
    container: ViewId,
    count: impl Fn() -> usize + 'static,
    row_h: f32,
    offset: sv_reactive::Signal<usize>,
) {
    let d = doc.clone();
    effect(move || {
        d.set_content_override(container, Some((0.0, count() as f32 * row_h)));
    });
    doc.set_on_scroll(container, move |_x, y| {
        offset.set((y / row_h) as usize);
    });
}

/// `{#key expr} ... {/key}`:key 值(PartialEq)变化时销毁重建整块,
/// 块内状态随之重置——与 Svelte 的 {#key} 语义一致
pub fn key_block<K: PartialEq + 'static>(
    doc: &Doc,
    parent: ViewId,
    key: impl Fn() -> K + 'static,
    build: impl Fn(&Doc, ViewId) + 'static,
) {
    let container = doc.create_view();
    doc.append(parent, container);
    let k = derived(key);
    let doc = doc.clone();
    effect(move || {
        k.with(|_| {}); // 只订阅 key,值本身不用
        doc.clear_children(container);
        build(&doc, container);
        let d = doc.clone();
        on_cleanup(move || d.clear_children(container));
    });
}

/// `style:prop={expr}` 指令:响应式**修补**单个样式字段(不重置其它字段,
/// 与 [`bind_style`] 的整体重算语义互补,可与静态 style 属性叠加)
pub fn bind_style_patch(doc: &Doc, id: ViewId, patch: impl Fn(&mut Style) + 'static) {
    let doc = doc.clone();
    effect(move || {
        doc.update_style(id, &patch);
    });
}

// ---------------------------------------------------------------------------
// 命令式挂载(对应 Svelte 的 mount / unmount)
// ---------------------------------------------------------------------------

/// [`mount`] 返回的句柄:整个挂载体的生命周期由它掌控
pub struct MountHandle {
    doc: Doc,
    container: ViewId,
    scope: RootHandle,
}

impl MountHandle {
    /// 对应 Svelte 的 `unmount`:先销毁响应式作用域(effect 停跑、cleanup 执行),
    /// 再移除容器子树——顺序不能反,否则销毁期间的 effect 还可能触碰已删的节点
    pub fn unmount(self) {
        self.scope.dispose();
        self.doc.remove(self.container);
    }
}

/// 对应 Svelte 的 `mount`:在 parent 下建一个容器节点,并在**独立 root 作用域**
/// 里执行构建闭包。内部创建的 signal/effect 不挂在调用方作用域上,生命周期完全
/// 由返回的 [`MountHandle`] 掌控——与 keyed each 行作用域是同一套做法
pub fn mount(doc: &Doc, parent: ViewId, f: impl FnOnce(&Doc, ViewId) + 'static) -> MountHandle {
    let container = doc.create_view();
    doc.append(parent, container);
    let d = doc.clone();
    // untrack:若在某个 effect 内调用 mount,构建期的散读不应订阅到该 effect 上
    let (_, scope) = create_root(|| untrack(|| f(&d, container)));
    MountHandle {
        doc: d,
        container,
        scope,
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sv_reactive::state;

    /// 手写"编译输出"构建计数器——这正是 view! 宏未来要生成的代码形态
    #[test]
    fn counter_headless() {
        let doc = Doc::new();
        let count = state(0);

        let root = doc.root();
        let label = doc.create_text("");
        doc.append(root, label);
        bind_text(&doc, label, move || format!("Count: {}", count.get()));

        let btn = doc.create_button("+1");
        doc.append(root, btn);
        doc.set_on_click(btn, move || count.update(|c| *c += 1));

        assert!(doc.dump().contains("Count: 0"));

        // 模拟点击
        let h = doc.click_handler(btn).unwrap();
        h();
        h();
        assert!(
            doc.dump().contains("Count: 2"),
            "点击后文本应精准更新:\n{}",
            doc.dump()
        );
    }

    #[test]
    fn text_update_is_fine_grained() {
        let doc = Doc::new();
        let a = state(String::from("hello"));
        let label = doc.create_text("");
        doc.append(doc.root(), label);
        bind_text(&doc, label, move || a.get());

        let v_before = doc.version();
        a.set("world".into());
        assert!(doc.version() > v_before);
        // 相同文本不应 bump 版本(渲染端不用重绘)
        let v = doc.version();
        a.set("world".into());
        assert_eq!(doc.version(), v, "写入相同文本不应触发树变更");
    }

    #[test]
    fn if_block_switches_and_disposes() {
        let doc = Doc::new();
        let show = state(true);
        if_block(
            &doc,
            doc.root(),
            move || show.get(),
            |doc, parent| {
                let t = doc.create_text("visible");
                doc.append(parent, t);
            },
            |doc, parent| {
                let t = doc.create_text("hidden");
                doc.append(parent, t);
            },
        );
        assert!(doc.dump().contains("visible"));
        show.set(false);
        assert!(doc.dump().contains("hidden"));
        assert!(!doc.dump().contains("visible"));
    }

    #[test]
    fn if_block_inner_state_disposed() {
        let doc = Doc::new();
        let show = state(true);
        let ticker = state(0);
        if_block(
            &doc,
            doc.root(),
            move || show.get(),
            move |doc, parent| {
                let t = doc.create_text("");
                doc.append(parent, t);
                bind_text(doc, t, move || format!("tick {}", ticker.get()));
            },
            |_, _| {},
        );
        ticker.set(1);
        assert!(doc.dump().contains("tick 1"));
        show.set(false); // 分支销毁,内部 bind_text effect 应一并销毁
        let v = doc.version();
        ticker.set(2); // 不应再有 effect 去改树
        assert_eq!(doc.version(), v, "分支销毁后其内部绑定不应再触发树变更");
    }

    #[test]
    fn each_block_rebuilds() {
        let doc = Doc::new();
        let items = state(vec!["a".to_string(), "b".to_string()]);
        each_block(
            &doc,
            doc.root(),
            move || items.get(),
            |doc, parent, item, i| {
                let t = doc.create_text(&format!("{i}:{item}"));
                doc.append(parent, t);
            },
        );
        let dump = doc.dump();
        assert!(dump.contains("0:a") && dump.contains("1:b"));
        items.update(|v| v.push("c".into()));
        assert!(doc.dump().contains("2:c"));
        items.update(|v| {
            v.remove(0);
        });
        let dump = doc.dump();
        assert!(!dump.contains(":a") && dump.contains("0:b") && dump.contains("1:c"));
    }

    #[test]
    fn virtual_list_million_rows_few_nodes() {
        let doc = Doc::new();
        let offset = state(0usize);
        let (_, _scope) = create_root(|| {
            virtual_list(
                &doc,
                doc.root(),
                || 1_000_000usize, // 逻辑百万行
                offset,
                30, // 视口只有 30 槽
                |i| format!("行 {i}"),
                |doc, parent, slot, _i| {
                    let t = doc.create_text("");
                    doc.append(parent, t);
                    bind_text(doc, t, move || slot.get().unwrap_or_else(|| "空".into()));
                },
            );
        });
        // 百万逻辑行,场景树只有 视口槽位 + 容器 + root
        let nodes = doc.read(|inner| inner.nodes.len());
        assert!(nodes <= 32 + 2, "虚拟化应只实例化视口:{nodes} 节点");
        assert!(doc.dump().contains("行 0") && doc.dump().contains("行 29"));

        // 滚动到 50 万:同一批节点原地更新,零结构变化
        let before = nodes;
        offset.set(500_000);
        let dump = doc.dump();
        assert!(
            dump.contains("行 500000") && dump.contains("行 500029"),
            "\n{dump}"
        );
        assert_eq!(
            doc.read(|inner| inner.nodes.len()),
            before,
            "滚动不应增删节点"
        );

        // 滚到尾部越界:空槽显示空态
        offset.set(999_990);
        let dump = doc.dump();
        assert!(
            dump.contains("行 999999") && dump.contains("空"),
            "\n{dump}"
        );
    }

    #[test]
    fn keyed_each_preserves_row_state() {
        let doc = Doc::new();
        let items = state(vec![(1i32, "甲"), (2, "乙"), (3, "丙")]);
        let builds = std::rc::Rc::new(std::cell::RefCell::new(0));
        let b = builds.clone();
        let (_, _scope) = create_root(|| {
            each_block_keyed(
                &doc,
                doc.root(),
                move || items.get(),
                |it| it.0,
                move |doc, parent, it| {
                    *b.borrow_mut() += 1;
                    // 行内状态:构建序号,重排/增删后不应重置
                    let my_build = *b.borrow();
                    let t = doc.create_text("");
                    doc.append(parent, t);
                    // 读行信号 → 内容变化原地更新(ADR-7)
                    bind_text(doc, t, move || format!("{}#{my_build}", it.get().1));
                },
            );
        });
        assert_eq!(*builds.borrow(), 3);
        assert!(doc.dump().contains("甲#1") && doc.dump().contains("丙#3"));

        // 倒序:零重建,行内状态保留,顺序翻转
        items.set(vec![(3, "丙"), (2, "乙"), (1, "甲")]);
        assert_eq!(*builds.borrow(), 3, "重排不应重建任何行");
        let dump = doc.dump();
        let pos_c = dump.find("丙#3").unwrap();
        let pos_a = dump.find("甲#1").unwrap();
        assert!(pos_c < pos_a, "顺序应翻转:\n{dump}");

        // 删 2、加 4:只建一行,只销一行
        items.set(vec![(3, "丙"), (1, "甲"), (4, "丁")]);
        assert_eq!(*builds.borrow(), 4, "只应新建 key=4 一行");
        let dump = doc.dump();
        assert!(!dump.contains("乙") && dump.contains("丁#4"), "\n{dump}");

        // 同 key 换内容:**原地更新且不重建**(ADR-7 目标形态)。
        // 改前行只在构建时读一次 T,同 key 换内容会永远显示旧数据
        items.set(vec![(3, "丙丙"), (1, "甲"), (4, "丁")]);
        assert_eq!(*builds.borrow(), 4, "内容变化不该重建行");
        let dump = doc.dump();
        assert!(dump.contains("丙丙#3"), "同 key 换内容应原地更新:\n{dump}");

        // 等值再设:不写行信号 → 行内绑定不重跑(纯重排/无变化零树改动)
        let v = doc.version();
        items.set(vec![(3, "丙丙"), (1, "甲"), (4, "丁")]);
        assert_eq!(doc.version(), v, "内容没变不该产生任何树改动");
    }

    #[test]
    fn each_block_else_shows_empty_state() {
        let doc = Doc::new();
        let items = state(Vec::<String>::new());
        each_block_else(
            &doc,
            doc.root(),
            move || items.get(),
            |doc, parent, item, _| {
                let t = doc.create_text(item);
                doc.append(parent, t);
            },
            |doc, parent| {
                let t = doc.create_text("空空如也");
                doc.append(parent, t);
            },
        );
        assert!(doc.dump().contains("空空如也"));
        items.update(|v| v.push("甲".into()));
        let dump = doc.dump();
        assert!(dump.contains("甲") && !dump.contains("空空如也"));
        items.update(|v| {
            v.clear();
        });
        assert!(doc.dump().contains("空空如也"), "清空后应回到空状态");
    }

    #[test]
    fn key_block_recreates_on_key_change() {
        let doc = Doc::new();
        let user = state(1i32);
        let unrelated = state(0i32);
        let builds = std::rc::Rc::new(std::cell::RefCell::new(0));
        let b = builds.clone();
        key_block(
            &doc,
            doc.root(),
            move || user.get(),
            move |doc, parent| {
                *b.borrow_mut() += 1;
                let t = doc.create_text("面板");
                doc.append(parent, t);
            },
        );
        assert_eq!(*builds.borrow(), 1);
        unrelated.set(9); // 与 key 无关,不应重建
        assert_eq!(*builds.borrow(), 1);
        user.set(2);
        assert_eq!(*builds.borrow(), 2, "key 变化应销毁重建");
        user.set(2); // 相同值,相等剪枝
        assert_eq!(*builds.borrow(), 2);
    }

    #[test]
    fn bind_style_patch_is_additive() {
        let doc = Doc::new();
        let size = state(10.0f32);
        let el = doc.create_text("x");
        doc.append(doc.root(), el);
        // 静态样式先落一笔
        doc.update_style(el, |s| s.gap = 3.0);
        bind_style_patch(&doc, el, move |s| s.padding = size.get().into());
        let read = |f: fn(&Style) -> f32| doc.read(|inner| f(&inner.nodes[el].style));
        assert_eq!(read(|s| s.padding.left), 10.0);
        assert_eq!(read(|s| s.gap), 3.0, "patch 不应重置其它字段");
        size.set(20.0);
        assert_eq!(read(|s| s.padding.left), 20.0, "patch 应响应式更新");
        assert_eq!(read(|s| s.gap), 3.0);
    }

    #[test]
    fn checkbox_toggle_and_dump() {
        let doc = Doc::new();
        let cb = doc.create_checkbox();
        doc.append(doc.root(), cb);
        doc.set_text(cb, "同意条款");
        assert!(doc.dump().contains("[ ] \"同意条款\""), "\n{}", doc.dump());
        doc.set_checked(cb, true);
        assert!(doc.dump().contains("[x] \"同意条款\""), "\n{}", doc.dump());
        // 相等不 bump:渲染端不用重绘
        let v = doc.version();
        doc.set_checked(cb, true);
        assert_eq!(doc.version(), v, "写入相同勾选状态不应触发树变更");
        doc.set_checked(cb, false);
        assert!(doc.dump().contains("[ ] \"同意条款\""));
    }

    #[test]
    fn pointer_hover_handlers_roundtrip() {
        let doc = Doc::new();
        let el = doc.create_view();
        doc.append(doc.root(), el);
        let log: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>> = Default::default();
        let l = log.clone();
        doc.set_on_pointer_enter(el, move || l.borrow_mut().push("enter"));
        let l = log.clone();
        doc.set_on_pointer_leave(el, move || l.borrow_mut().push("leave"));
        // 模拟指针进出(渲染壳命中测试后会这样取回调调用)
        let enter = doc.pointer_enter_handler(el).unwrap();
        let leave = doc.pointer_leave_handler(el).unwrap();
        enter();
        leave();
        enter();
        assert_eq!(*log.borrow(), vec!["enter", "leave", "enter"]);
        // 未设置回调的节点取不到
        let other = doc.create_view();
        assert!(doc.pointer_enter_handler(other).is_none());
        assert!(doc.pointer_leave_handler(other).is_none());
    }

    #[test]
    fn mount_twice_unmount_cleans_tree_and_scopes() {
        let doc = Doc::new();
        let tick = state(0);
        let n0 = sv_reactive::debug_node_count();

        let h1 = mount(&doc, doc.root(), move |doc, parent| {
            let t = doc.create_text("");
            doc.append(parent, t);
            bind_text(doc, t, move || format!("一号:{}", tick.get()));
        });
        let h2 = mount(&doc, doc.root(), move |doc, parent| {
            let t = doc.create_text("");
            doc.append(parent, t);
            bind_text(doc, t, move || format!("二号:{}", tick.get()));
        });
        tick.set(1);
        let dump = doc.dump();
        assert!(
            dump.contains("一号:1") && dump.contains("二号:1"),
            "\n{dump}"
        );

        h1.unmount();
        let dump = doc.dump();
        assert!(
            !dump.contains("一号") && dump.contains("二号:1"),
            "\n{dump}"
        );
        tick.set(2); // 一号的作用域已销毁,只有二号还在响应
        assert!(doc.dump().contains("二号:2"));

        h2.unmount();
        assert_eq!(
            sv_reactive::debug_node_count(),
            n0,
            "unmount 后挂载体的响应式节点应全部回收"
        );
        let v = doc.version();
        tick.set(3);
        assert_eq!(doc.version(), v, "全部卸载后不应再有绑定触碰树");
    }

    #[test]
    fn context_reaches_keyed_each_rows() {
        use sv_reactive::{provide_context, use_context};
        struct Theme(&'static str);

        let doc = Doc::new();
        let items = state(vec![1i32, 2]);
        // 组件层作用域提供 context;行作用域是独立 create_root,
        // 仍应沿"创建时 owner"链穿过去取到
        let (_, _scope) = create_root(|| {
            provide_context(Theme("dark"));
            each_block_keyed(
                &doc,
                doc.root(),
                move || items.get(),
                |it| *it,
                |doc, parent, it| {
                    let theme = use_context::<Theme>().map_or("取不到", |t| t.0);
                    let t = doc.create_text(&format!("{}:{theme}", it.get()));
                    doc.append(parent, t);
                },
            );
        });
        let dump = doc.dump();
        assert!(
            dump.contains("1:dark") && dump.contains("2:dark"),
            "\n{dump}"
        );
        // 列表更新后新建的行(effect 重跑里建的 root)同样取得到
        items.update(|v| v.push(3));
        assert!(doc.dump().contains("3:dark"), "\n{}", doc.dump());
    }

    #[test]
    fn cond_flip_only_rebuilds_on_change() {
        let doc = Doc::new();
        let n = state(0);
        let builds = std::rc::Rc::new(std::cell::RefCell::new(0));
        let b = builds.clone();
        if_block(
            &doc,
            doc.root(),
            move || n.get() > 5,
            move |doc, parent| {
                *b.borrow_mut() += 1;
                let t = doc.create_text("big");
                doc.append(parent, t);
            },
            |_, _| {},
        );
        n.set(1);
        n.set(2);
        n.set(3); // cond 始终 false,不应重建
        assert_eq!(*builds.borrow(), 0);
        n.set(6);
        assert_eq!(*builds.borrow(), 1);
        n.set(7); // cond 仍 true,不应重建
        assert_eq!(*builds.borrow(), 1);
    }
}

#[cfg(test)]
mod memory_probe {
    use super::*;

    /// 内存预算护栏:核心结构体大小(轻量场景关注点,docs/research/15)。
    /// 变大要有充分理由——这是每个节点/信号都要付的钱
    #[test]
    fn core_struct_sizes_within_budget() {
        let vn = std::mem::size_of::<ViewNode>();
        let st = std::mem::size_of::<Style>();
        println!(
            "[probe] ViewNode={vn}B Style={st}B Edges={}B",
            std::mem::size_of::<Edges>()
        );
        // 2026-07-18 预算两次上调:R1 焦点/输入(focusable/accepts_text/
        // on_key/on_focus_change/input/scroll/on_scroll/content_override
        // ≈ +100B)与 R2 flex 第一批(justify/align/grow/shrink/min-max
        // ≈ +48B,调研 23 §2.2 冷字段 Box 化留作超线时的收缩手段)
        assert!(st <= 192, "Style 超预算: {st}B");
        assert!(vn <= 448, "ViewNode 超预算: {vn}B");
    }
}
