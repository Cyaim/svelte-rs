//! IR → TokenStream:把模板编译成对 sv-ui 绑定原语的命令式调用。
//!
//! **调用形状不在这里**:所有对 sv-ui 的发射统一走
//! [`sv_compiler::emit`](sv_compiler::emit) —— 与 `.sv` 前端共享同一份词汇表
//! (ADR-2 无悔三步 ①)。本文件只负责"宏 IR → 该词汇表的调用序列",
//! 于是原语签名变更只需改 emit 一处,不再两边同步。
//!
//! 生成代码的约定(与 emit 一致):
//! - 所有 sv-ui 引用走 `::sv_ui::` 绝对路径;
//! - 外层 `__doc: ::sv_ui::Doc`(owned)、`__parent: ::sv_ui::ViewId`;
//!   if/each 回调参数是 `&Doc`,回调体开头 `let __doc = __doc.clone();`
//!   归一为 owned,子节点代码因此在任何层级都长一个样;
//! - 每个创建出的节点用全局计数器命名(`__el0`、`__el1`…),避免遮蔽混乱;
//! - 用户表达式(插值 / 条件 / 列表 / 闭包)原样嵌入,span 保真
//!   (这是宏路径独有的好处,也是 ADR-2 不把 IR 也合并的原因)。

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use syn::Ident;

use sv_compiler::emit::{self, ElemKind, TextPart};

use crate::ir::{
    Attr, AttrKind, ForNode, IfNode, LeafElem, LeafKind, Node, Segment, ViewElem, ViewInput,
};

pub fn generate(input: &ViewInput) -> TokenStream {
    let mut cx = Cx::default();
    let doc = &input.doc;
    let parent_expr = &input.parent;
    let root_parent = parent_ident();
    let body = gen_nodes(&mut cx, &input.nodes, &root_parent);
    // 空模板时抑制 unused 警告(正常模板中两者必然被使用)
    let silence = if input.nodes.is_empty() {
        quote! { let _ = (&__doc, __parent); }
    } else {
        TokenStream::new()
    };
    quote! {{
        let __doc: ::sv_ui::Doc = (#doc).clone();
        let __parent: ::sv_ui::ViewId = #parent_expr;
        #silence
        #body
    }}
}

#[derive(Default)]
struct Cx {
    counter: usize,
}

impl Cx {
    /// 唯一的元素变量名:__el0、__el1…
    fn fresh_el(&mut self) -> Ident {
        let id = self.counter;
        self.counter += 1;
        format_ident!("__el{}", id)
    }
}

fn parent_ident() -> Ident {
    Ident::new("__parent", Span::call_site())
}

fn gen_nodes(cx: &mut Cx, nodes: &[Node], parent: &Ident) -> TokenStream {
    nodes.iter().map(|n| gen_node(cx, n, parent)).collect()
}

fn gen_node(cx: &mut Cx, node: &Node, parent: &Ident) -> TokenStream {
    match node {
        Node::View(el) => gen_view(cx, el, parent),
        Node::Leaf(el) => gen_leaf(cx, el, parent),
        Node::Text(segments) => gen_bare_text(cx, segments, parent),
        Node::If(node) => gen_if(cx, node, parent),
        Node::For(node) => gen_for(cx, node, parent),
    }
}

fn gen_view(cx: &mut Cx, el: &ViewElem, parent: &Ident) -> TokenStream {
    let var = cx.fresh_el();
    let create = emit::create(&var, ElemKind::View, "");
    let append = emit::append(parent, &var);
    let attrs = gen_attrs(&el.attrs, &var);
    let children = gen_nodes(cx, &el.children, &var);
    quote! {
        #create
        #append
        #attrs
        #children
    }
}

fn gen_leaf(cx: &mut Cx, el: &LeafElem, parent: &Ident) -> TokenStream {
    let var = cx.fresh_el();
    let append = emit::append(parent, &var);
    // input:无文本段(value 走 bind_value 绑定,不走内容)
    if el.kind == LeafKind::Input {
        let create = emit::create(&var, ElemKind::TextInput, "");
        let attrs = gen_attrs(&el.attrs, &var);
        return quote! {
            #create
            #append
            #attrs
        };
    }
    let kind = match el.kind {
        LeafKind::Text => ElemKind::Text,
        LeafKind::Button => ElemKind::Button,
        LeafKind::Input => unreachable!(),
    };
    let parts = text_parts(&el.segments);
    let (create, bind) = match emit::static_text(&parts) {
        // 纯字面量:合并成静态文本 / 按钮 label,无绑定
        Some(s) => (emit::create(&var, kind, &s), TokenStream::new()),
        // 含插值:先建空节点,再挂 bind_text
        None => (emit::create(&var, kind, ""), emit::bind_text(&var, &parts)),
    };
    let attrs = gen_attrs(&el.attrs, &var);
    quote! {
        #create
        #append
        #attrs
        #bind
    }
}

/// `<view>` 子层级直接出现的文本段(解析期已合并)
fn gen_bare_text(cx: &mut Cx, segments: &[Segment], parent: &Ident) -> TokenStream {
    let var = cx.fresh_el();
    let append = emit::append(parent, &var);
    let parts = text_parts(segments);
    match emit::static_text(&parts) {
        Some(s) => {
            let create = emit::create(&var, ElemKind::Text, &s);
            quote! { #create #append }
        }
        None => {
            let create = emit::create(&var, ElemKind::Text, "");
            let bind = emit::bind_text(&var, &parts);
            quote! { #create #append #bind }
        }
    }
}

/// 宏 IR 的文本段 → 共享词汇表的文本段(表达式原样带 span 过去)
fn text_parts(segments: &[Segment]) -> Vec<TextPart> {
    segments
        .iter()
        .map(|seg| match seg {
            Segment::Lit(lit) => TextPart::Lit(lit.value()),
            Segment::Expr(expr) => TextPart::Expr(expr.to_token_stream()),
        })
        .collect()
}

fn gen_attrs(attrs: &[Attr], var: &Ident) -> TokenStream {
    let mut ts: TokenStream = attrs
        .iter()
        .map(|attr| {
            let expr = attr.expr.to_token_stream();
            match attr.kind {
                // 用户闭包表达式原样传给 bind_style(响应式:闭包里读 signal 会自动追踪)。
                // 宏路径的 style 是"闭包直传",与 .sv 的"编译期样式表"是两种
                // 表面语法,故不进共享词汇表
                AttrKind::Style => quote! { ::sv_ui::bind_style(&__doc, #var, #expr); },
                AttrKind::OnClick => emit::on_click(var, expr),
                // 按下/抬起在下面合成(sv-ui 只有一个 on_key 槽位)
                AttrKind::OnKeyDown | AttrKind::OnKeyUp => TokenStream::new(),
                AttrKind::Placeholder => emit::placeholder(var, expr),
                AttrKind::BindValue => emit::bind_value(var, expr),
                AttrKind::OnInput => emit::on_input(var, expr),
                AttrKind::OnSubmit => emit::on_submit(var, expr),
                AttrKind::OnScroll => emit::on_scroll(var, expr),
                AttrKind::AriaLabel => emit::aria_label(var, expr, true),
                // 延后发射(桥链式保留既有 on_scroll,与 on_scroll 共存)
                AttrKind::BindScrollY => TokenStream::new(),
                // 下方合成进单一 set_on_focus_change
                AttrKind::OnFocus | AttrKind::OnBlur => TokenStream::new(),
            }
        })
        .collect();
    if let Some(a) = attrs.iter().find(|a| a.kind == AttrKind::BindScrollY) {
        ts.extend(emit::bind_scroll_y(var, a.expr.to_token_stream()));
    }
    let find = |k: AttrKind| {
        attrs
            .iter()
            .find(|a| a.kind == k)
            .map(|a| a.expr.to_token_stream())
    };
    let (focus, blur) = (find(AttrKind::OnFocus), find(AttrKind::OnBlur));
    if focus.is_some() || blur.is_some() {
        // 宏前端没有 `<style>` 块,因而没有 `:focus` 伪类状态要写
        ts.extend(emit::focus_change(var, focus, blur, None));
    }
    ts.extend(emit::key_handlers(
        var,
        find(AttrKind::OnKeyDown),
        find(AttrKind::OnKeyUp),
    ));
    ts
}

fn gen_if(cx: &mut Cx, node: &IfNode, parent: &Ident) -> TokenStream {
    let cond = node.cond.to_token_stream();
    let then_closure = gen_branch_closure(cx, &node.then_nodes);
    let else_closure = gen_branch_closure(cx, &node.else_nodes);
    emit::if_block(parent, cond, then_closure, else_closure)
}

/// if 分支回调:`Fn(&Doc, ViewId)`(闭包协议在共享词汇表里)
fn gen_branch_closure(cx: &mut Cx, nodes: &[Node]) -> TokenStream {
    let parent = parent_ident();
    let body = gen_nodes(cx, nodes, &parent);
    emit::rebuild_closure(body, TokenStream::new())
}

fn gen_for(cx: &mut Cx, node: &ForNode, parent: &Ident) -> TokenStream {
    let items = &node.items;
    let row = if node.body.is_empty() {
        quote! { |_, _, _, _| {} }
    } else {
        let pat = &node.pat;
        let index_binding = match &node.index {
            Some(ident) => quote! { let #ident = __index; },
            None => quote! { let _ = __index; },
        };
        let parent = parent_ident();
        let body = gen_nodes(cx, &node.body, &parent);
        quote! {
            move |__doc, __parent, __item, __index| {
                let __doc: ::sv_ui::Doc = __doc.clone();
                let #pat = ::std::clone::Clone::clone(__item);
                #index_binding
                #body
            }
        }
    };
    emit::each_block(parent, items.to_token_stream(), row)
}
