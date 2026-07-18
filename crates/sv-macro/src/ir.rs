//! 模板 IR:`view!` 的解析产物,解析(parse)与生成(codegen)之间的中间表示。
//!
//! 这里刻意不保存多余的语法细节(尖括号、闭合标签位置……),只留生成代码
//! 需要的结构;用户表达式原样保存为 `syn::Expr` / `syn::Pat`,span 全程保真。

use syn::{Expr, Ident, LitStr, Pat};

/// `view! { doc_expr, parent_expr => 模板... }` 整体
pub struct ViewInput {
    pub doc: Expr,
    pub parent: Expr,
    pub nodes: Vec<Node>,
}

/// 模板节点(view 子层级可出现的东西)
pub enum Node {
    /// `<view ...> 子节点... </view>`
    View(ViewElem),
    /// `<text ...>段...</text>` / `<button ...>段...</button>`
    Leaf(LeafElem),
    /// 直接出现在 view 子层级的连续文本段(解析期已合并成一个文本节点)
    Text(Vec<Segment>),
    /// `if cond { ... } else if ... { ... } else { ... }`(else-if 已脱糖为嵌套 If)
    If(IfNode),
    /// `for pat[, index] in expr { ... }`
    For(ForNode),
}

pub struct ViewElem {
    pub attrs: Vec<Attr>,
    pub children: Vec<Node>,
}

pub struct LeafElem {
    pub kind: LeafKind,
    pub attrs: Vec<Attr>,
    pub segments: Vec<Segment>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LeafKind {
    Text,
    Button,
    /// `<input placeholder(...) bind_value(sig) on_input(...) on_submit(...) />`
    /// (叶子无文本段:value 走绑定,不走内容)
    Input,
}

pub struct Attr {
    pub kind: AttrKind,
    pub expr: Expr,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AttrKind {
    /// `style(闭包)` → `::sv_ui::bind_style(&__doc, el, 闭包)`
    Style,
    /// `on_click(闭包)` → `__doc.set_on_click(el, 闭包)`
    OnClick,
    /// `on_key_down(闭包)` → `set_focusable(el, true) + set_on_key(el, 闭包)`
    OnKeyDown,
    /// `on_focus(闭包)` / `on_blur(闭包)` → 合成进单一 `set_on_focus_change`
    OnFocus,
    OnBlur,
    /// `placeholder("...")` → `__doc.set_placeholder(el, ...)`(仅 input)
    Placeholder,
    /// `bind_value(signal)` → effect 写 + `set_on_input` 读(仅 input)
    BindValue,
    /// `on_input(闭包)` / `on_submit(闭包)`(仅 input,签名 Fn(&str))
    OnInput,
    OnSubmit,
    /// `on_scroll(闭包)` → `set_on_scroll`(签名 Fn(f32, f32))
    OnScroll,
    /// `aria_label(表达式)` → 响应式 `set_accessible_label`(无障碍名称)
    AriaLabel,
    /// `bind_scroll_y(signal)` → `::sv_ui::bind_scroll_y`(纵向滚动双向桥)
    BindScrollY,
}

/// 文本段:字符串字面量或 `{表达式}` 插值
pub enum Segment {
    Lit(LitStr),
    Expr(Expr),
}

pub struct IfNode {
    pub cond: Expr,
    pub then_nodes: Vec<Node>,
    pub else_nodes: Vec<Node>,
}

pub struct ForNode {
    pub pat: Pat,
    pub index: Option<Ident>,
    pub items: Expr,
    pub body: Vec<Node>,
}
