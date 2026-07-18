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
pub mod tasks;

/// 组件 children / 具名 snippet 的类型:接收 (doc, 挂载点) 的可复用构建闭包
pub type Snippet = Rc<dyn Fn(&Doc, ViewId)>;

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
        Self { top: v, right: v, bottom: v, left: v }
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
}

pub struct ViewNode {
    pub kind: ElementKind,
    /// Text / Button / Checkbox 的文本内容
    pub text: String,
    /// Checkbox 的勾选状态(其它元素恒为 false)
    pub checked: bool,
    pub style: Style,
    pub parent: Option<ViewId>,
    pub children: Vec<ViewId>,
    pub on_click: Option<Rc<dyn Fn()>>,
    pub on_pointer_enter: Option<Rc<dyn Fn()>>,
    pub on_pointer_down: Option<Rc<dyn Fn()>>,
    pub on_pointer_up: Option<Rc<dyn Fn()>>,
    pub on_pointer_leave: Option<Rc<dyn Fn()>>,
}

pub struct DocumentInner {
    pub nodes: SlotMap<ViewId, ViewNode>,
    pub root: ViewId,
    version: u64,
    on_mutate: Option<Box<dyn Fn()>>,
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
            style: Style::default(),
            parent: None,
            children: Vec::new(),
            on_click: None,
            on_pointer_enter: None,
            on_pointer_down: None,
            on_pointer_up: None,
            on_pointer_leave: None,
        });
        Doc(Rc::new(RefCell::new(DocumentInner {
            nodes,
            root,
            version: 0,
            on_mutate: None,
        })))
    }

    fn bump(&self) {
        let cb = {
            let mut inner = self.0.borrow_mut();
            inner.version += 1;
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
            style: Style::default(),
            parent: None,
            children: Vec::new(),
            on_click: None,
            on_pointer_enter: None,
            on_pointer_down: None,
            on_pointer_up: None,
            on_pointer_leave: None,
        });
        self.bump();
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

    pub fn append(&self, parent: ViewId, child: ViewId) {
        {
            let mut inner = self.0.borrow_mut();
            if let Some(old_parent) = inner.nodes[child].parent {
                let op = old_parent;
                inner.nodes[op].children.retain(|c| *c != child);
            }
            inner.nodes[child].parent = Some(parent);
            inner.nodes[parent].children.push(child);
        }
        self.bump();
    }

    /// 摘除并递归销毁整棵子树
    pub fn remove(&self, id: ViewId) {
        {
            let mut inner = self.0.borrow_mut();
            if let Some(p) = inner.nodes.get(id).and_then(|n| n.parent) {
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
        }
        self.bump();
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
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else { return };
            if n.text == text {
                return;
            }
            n.text = text.to_string();
        }
        self.bump();
    }

    pub fn set_checked(&self, id: ViewId, checked: bool) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else { return };
            if n.checked == checked {
                return; // 相等不 bump:渲染端不用白白重绘
            }
            n.checked = checked;
        }
        self.bump();
    }

    pub fn set_style(&self, id: ViewId, style: Style) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else { return };
            if n.style == style {
                return;
            }
            n.style = style;
        }
        self.bump();
    }

    /// 原地修改样式(比整体替换省事)
    pub fn update_style(&self, id: ViewId, f: impl FnOnce(&mut Style)) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else { return };
            f(&mut n.style);
        }
        self.bump();
    }

    pub fn set_on_click(&self, id: ViewId, f: impl Fn() + 'static) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else { return };
            n.on_click = Some(Rc::new(f));
        }
        self.bump();
    }

    /// 取出点击回调(clone 出来调用,避免调用期间持有树的借用)
    pub fn click_handler(&self, id: ViewId) -> Option<Rc<dyn Fn()>> {
        self.0.borrow().nodes.get(id).and_then(|n| n.on_click.clone())
    }

    pub fn set_on_pointer_enter(&self, id: ViewId, f: impl Fn() + 'static) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else { return };
            n.on_pointer_enter = Some(Rc::new(f));
        }
        self.bump();
    }

    pub fn set_on_pointer_leave(&self, id: ViewId, f: impl Fn() + 'static) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else { return };
            n.on_pointer_leave = Some(Rc::new(f));
        }
        self.bump();
    }

    /// 取出悬停进入回调(同 [`Doc::click_handler`]:clone 出来调用,不持树借用)
    pub fn pointer_enter_handler(&self, id: ViewId) -> Option<Rc<dyn Fn()>> {
        self.0.borrow().nodes.get(id).and_then(|n| n.on_pointer_enter.clone())
    }

    /// 取出悬停离开回调
    pub fn pointer_leave_handler(&self, id: ViewId) -> Option<Rc<dyn Fn()>> {
        self.0.borrow().nodes.get(id).and_then(|n| n.on_pointer_leave.clone())
    }

    pub fn set_on_pointer_down(&self, id: ViewId, f: impl Fn() + 'static) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else { return };
            n.on_pointer_down = Some(Rc::new(f));
        }
        self.bump();
    }

    pub fn set_on_pointer_up(&self, id: ViewId, f: impl Fn() + 'static) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(n) = inner.nodes.get_mut(id) else { return };
            n.on_pointer_up = Some(Rc::new(f));
        }
        self.bump();
    }

    pub fn pointer_down_handler(&self, id: ViewId) -> Option<Rc<dyn Fn()>> {
        self.0.borrow().nodes.get(id).and_then(|n| n.on_pointer_down.clone())
    }

    pub fn pointer_up_handler(&self, id: ViewId) -> Option<Rc<dyn Fn()>> {
        self.0.borrow().nodes.get(id).and_then(|n| n.on_pointer_up.clone())
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
            }
            for c in &n.children {
                walk(inner, *c, depth + 1, out);
            }
        }
        let inner = self.0.borrow();
        let mut out = String::new();
        walk(&inner, inner.root, 0, &mut out);
        out
    }
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
    row: impl Fn(&Doc, ViewId, &T) + 'static,
) where
    T: Clone + 'static,
    K: PartialEq + Clone + 'static,
{
    let container = doc.create_view();
    doc.append(parent, container);
    let doc = doc.clone();
    // 行注册表活在 effect 之外:重跑时复用而不是销毁
    type Rows<K> = Rc<RefCell<Vec<(K, ViewId, RootHandle)>>>;
    let rows: Rows<K> = Rc::new(RefCell::new(Vec::new()));
    let rows_cleanup = rows.clone();
    let doc_cleanup = doc.clone();

    effect(move || {
        let new_items = items(); // 追踪列表本身
        let mut old: Vec<(K, ViewId, RootHandle)> = rows.borrow_mut().drain(..).collect();
        let mut new_rows = Vec::with_capacity(new_items.len());
        for item in &new_items {
            let k = untrack(|| key_of(item));
            if let Some(pos) = old.iter().position(|(ok, _, _)| *ok == k) {
                // 复用:append 即"移动到末尾",按新序逐个 append 完成重排
                let entry = old.remove(pos);
                doc.append(container, entry.1);
                new_rows.push(entry);
            } else {
                let cont = doc.create_view();
                doc.append(container, cont);
                let d = doc.clone();
                let item = item.clone();
                // 行作用域独立于本 effect(不随列表变化销毁);
                // untrack:行构建期的散读不应订阅到本 effect 上
                let (_, scope) = create_root(|| untrack(|| row(&d, cont, &item)));
                new_rows.push((k, cont, scope));
            }
        }
        for (_, cont, scope) in old {
            scope.dispose();
            doc.remove(cont);
        }
        *rows.borrow_mut() = new_rows;
    });

    // 整块卸载时销毁所有行作用域
    on_cleanup(move || {
        for (_, cont, scope) in rows_cleanup.borrow_mut().drain(..) {
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
    MountHandle { doc: d, container, scope }
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
        assert!(doc.dump().contains("Count: 2"), "点击后文本应精准更新:\n{}", doc.dump());
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
                    bind_text(doc, t, move || {
                        slot.get().unwrap_or_else(|| "空".into())
                    });
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
        assert!(dump.contains("行 500000") && dump.contains("行 500029"), "\n{dump}");
        assert_eq!(doc.read(|inner| inner.nodes.len()), before, "滚动不应增删节点");

        // 滚到尾部越界:空槽显示空态
        offset.set(999_990);
        let dump = doc.dump();
        assert!(dump.contains("行 999999") && dump.contains("空"), "\n{dump}");
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
                    let name = it.1;
                    bind_text(doc, t, move || format!("{name}#{my_build}"));
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
        assert!(dump.contains("一号:1") && dump.contains("二号:1"), "\n{dump}");

        h1.unmount();
        let dump = doc.dump();
        assert!(!dump.contains("一号") && dump.contains("二号:1"), "\n{dump}");
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
                    let t = doc.create_text(&format!("{it}:{theme}"));
                    doc.append(parent, t);
                },
            );
        });
        let dump = doc.dump();
        assert!(dump.contains("1:dark") && dump.contains("2:dark"), "\n{dump}");
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
        println!("[probe] ViewNode={vn}B Style={st}B Edges={}B", std::mem::size_of::<Edges>());
        assert!(st <= 128, "Style 超预算: {st}B");
        assert!(vn <= 320, "ViewNode 超预算: {vn}B");
    }
}
