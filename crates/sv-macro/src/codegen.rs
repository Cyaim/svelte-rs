//! IR → TokenStream:把模板编译成对 sv-ui 绑定原语的命令式调用。
//!
//! 生成代码的约定:
//! - 所有 sv-ui 引用走 `::sv_ui::` 绝对路径;
//! - 外层 `__doc: ::sv_ui::Doc`(owned)、`__parent: ::sv_ui::ViewId`;
//!   if/each 回调参数是 `&Doc`,回调体开头 `let __doc = __doc.clone();`
//!   归一为 owned,子节点代码因此在任何层级都长一个样;
//! - 每个创建出的节点用全局计数器命名(`__el0`、`__el1`…),避免遮蔽混乱;
//! - 用户表达式(插值 / 条件 / 列表 / 闭包)原样嵌入,span 保真。

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::Ident;

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
    let attrs = gen_attrs(&el.attrs, &var);
    let children = gen_nodes(cx, &el.children, &var);
    quote! {
        let #var = __doc.create_view();
        __doc.append(#parent, #var);
        #attrs
        #children
    }
}

fn gen_leaf(cx: &mut Cx, el: &LeafElem, parent: &Ident) -> TokenStream {
    let var = cx.fresh_el();
    // input:无文本段(value 走 bind_value 绑定,不走内容)
    if el.kind == LeafKind::Input {
        let attrs = gen_attrs(&el.attrs, &var);
        return quote! {
            let #var = __doc.create_text_input();
            __doc.append(#parent, #var);
            #attrs
        };
    }
    let ctor = match el.kind {
        LeafKind::Text => Ident::new("create_text", Span::call_site()),
        LeafKind::Button => Ident::new("create_button", Span::call_site()),
        LeafKind::Input => unreachable!(),
    };
    let (create, bind) = match static_text(&el.segments) {
        // 纯字面量:合并成静态文本 / 按钮 label,无绑定
        Some(s) => (quote! { let #var = __doc.#ctor(#s); }, TokenStream::new()),
        // 含插值:先建空节点,再挂 bind_text
        None => (
            quote! { let #var = __doc.#ctor(""); },
            gen_bind_text(&var, &el.segments),
        ),
    };
    let attrs = gen_attrs(&el.attrs, &var);
    quote! {
        #create
        __doc.append(#parent, #var);
        #attrs
        #bind
    }
}

/// `<view>` 子层级直接出现的文本段(解析期已合并)
fn gen_bare_text(cx: &mut Cx, segments: &[Segment], parent: &Ident) -> TokenStream {
    let var = cx.fresh_el();
    match static_text(segments) {
        Some(s) => quote! {
            let #var = __doc.create_text(#s);
            __doc.append(#parent, #var);
        },
        None => {
            let bind = gen_bind_text(&var, segments);
            quote! {
                let #var = __doc.create_text("");
                __doc.append(#parent, #var);
                #bind
            }
        }
    }
}

/// 全部是字面量时返回拼接结果,否则 None(需要 bind_text)
fn static_text(segments: &[Segment]) -> Option<String> {
    let mut s = String::new();
    for seg in segments {
        match seg {
            Segment::Lit(lit) => s.push_str(&lit.value()),
            Segment::Expr(_) => return None,
        }
    }
    Some(s)
}

fn gen_bind_text(var: &Ident, segments: &[Segment]) -> TokenStream {
    let pushes = segments.iter().map(|seg| match seg {
        Segment::Lit(lit) => quote! { __s.push_str(#lit); },
        Segment::Expr(expr) => quote! { __s.push_str(&(#expr).to_string()); },
    });
    quote! {
        ::sv_ui::bind_text(&__doc, #var, move || {
            let mut __s = ::std::string::String::new();
            #(#pushes)*
            __s
        });
    }
}

fn gen_attrs(attrs: &[Attr], var: &Ident) -> TokenStream {
    let mut ts: TokenStream = attrs
        .iter()
        .map(|attr| {
            let expr = &attr.expr;
            match attr.kind {
                // 用户闭包表达式原样传给 bind_style(响应式:闭包里读 signal 会自动追踪)
                AttrKind::Style => quote! { ::sv_ui::bind_style(&__doc, #var, #expr); },
                AttrKind::OnClick => quote! { __doc.set_on_click(#var, #expr); },
                // 自动设 focusable(不设位回调永远收不到事件,floem 教训)
                AttrKind::OnKeyDown => quote! {
                    __doc.set_focusable(#var, true);
                    __doc.set_on_key(#var, #expr);
                },
                AttrKind::Placeholder => quote! { __doc.set_placeholder(#var, #expr); },
                // bind_value:effect 写(signal→树)+ on_input 读(树→signal)
                AttrKind::BindValue => quote! {
                    {
                        let __b_sig = #expr;
                        let __b_doc = __doc.clone();
                        let __b_el = #var;
                        ::sv_reactive::effect(move || { __b_doc.set_input_value(__b_el, &__b_sig.get()); });
                        __doc.set_on_input(#var, move |__v| __b_sig.set(__v.to_string()));
                    }
                },
                AttrKind::OnInput => quote! { __doc.set_on_input(#var, #expr); },
                AttrKind::OnSubmit => quote! { __doc.set_on_submit(#var, #expr); },
                AttrKind::OnScroll => quote! { __doc.set_on_scroll(#var, #expr); },
                AttrKind::AriaLabel => quote! {
                    {
                        let __a_doc = __doc.clone();
                        let __a_el = #var;
                        ::sv_reactive::effect(move || {
                            __a_doc.set_accessible_label(__a_el, &(#expr));
                        });
                    }
                },
                // 延后发射(桥链式保留既有 on_scroll,与 on_scroll 共存)
                AttrKind::BindScrollY => TokenStream::new(),
                // 下方合成进单一 set_on_focus_change
                AttrKind::OnFocus | AttrKind::OnBlur => TokenStream::new(),
            }
        })
        .collect();
    if let Some(a) = attrs.iter().find(|a| a.kind == AttrKind::BindScrollY) {
        let e = &a.expr;
        ts.extend(quote! { ::sv_ui::bind_scroll_y(&__doc, #var, #e); });
    }
    let focus = attrs.iter().find(|a| a.kind == AttrKind::OnFocus);
    let blur = attrs.iter().find(|a| a.kind == AttrKind::OnBlur);
    if focus.is_some() || blur.is_some() {
        let f = focus.map_or(quote! { let __uf = || {}; }, |a| {
            let e = &a.expr;
            quote! { let __uf = #e; }
        });
        let b = blur.map_or(quote! { let __ub = || {}; }, |a| {
            let e = &a.expr;
            quote! { let __ub = #e; }
        });
        ts.extend(quote! {
            {
                #f
                #b
                __doc.set_on_focus_change(#var, move |__fc| if __fc { __uf(); } else { __ub(); });
            }
        });
    }
    ts
}

fn gen_if(cx: &mut Cx, node: &IfNode, parent: &Ident) -> TokenStream {
    let cond = &node.cond;
    let then_closure = gen_branch_closure(cx, &node.then_nodes);
    let else_closure = gen_branch_closure(cx, &node.else_nodes);
    quote! {
        ::sv_ui::if_block(&__doc, #parent, move || #cond, #then_closure, #else_closure);
    }
}

/// if 分支回调:`Fn(&Doc, ViewId)`。体内先把 `&Doc` clone 成 owned,
/// 让子节点代码与外层保持同一形态。空分支给 `|_, _| {}` 免警告。
fn gen_branch_closure(cx: &mut Cx, nodes: &[Node]) -> TokenStream {
    if nodes.is_empty() {
        return quote! { |_, _| {} };
    }
    let parent = parent_ident();
    let body = gen_nodes(cx, nodes, &parent);
    quote! {
        move |__doc, __parent| {
            let __doc: ::sv_ui::Doc = __doc.clone();
            #body
        }
    }
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
    quote! {
        ::sv_ui::each_block(&__doc, #parent, move || #items, #row);
    }
}
