//! 手写递归下降解析:`ParseStream` → IR。
//!
//! 语法(v0):
//! ```text
//! view! { doc_expr, parent_expr => 节点* }
//! 节点     := 元素 | if 块 | for 块 | 文本段+
//! 元素     := "<view" 属性* (">" 节点* "</view>" | "/>")
//!           | "<text" 属性* (">" 段* "</text>" | "/>")
//!           | "<button" 属性* (">" 段* "</button>" | "/>")
//! 属性     := ("style" | "on_click") "(" 表达式 ")"
//! 段       := 字符串字面量 | "{" 表达式 "}"
//! if 块    := "if" 表达式 "{" 节点* "}" ("else" (if 块 | "{" 节点* "}"))?
//! for 块   := "for" 模式 ("," 索引ident)? "in" 表达式 "{" 节点* "}"
//! ```
//!
//! 所有错误都是带准确 span 的 [`syn::Error`],绝不 panic。

use syn::parse::{Parse, ParseStream};
use syn::{Error, Expr, Ident, LitStr, Pat, Result, Token, braced, parenthesized, token};

use crate::ir::{
    Attr, AttrKind, ForNode, IfNode, LeafElem, LeafKind, Node, Segment, ViewElem, ViewInput,
};

impl Parse for ViewInput {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.is_empty() {
            return Err(input.error("view! 语法:view! { doc表达式, 父节点表达式 => 模板... }"));
        }
        let doc: Expr = input.parse()?;
        input.parse::<Token![,]>()?;
        let parent: Expr = input.parse()?;
        input.parse::<Token![=>]>()?;
        let nodes = parse_children(input, false)?;
        Ok(ViewInput { doc, parent, nodes })
    }
}

/// 解析一串兄弟节点,直到流结束(或在元素内部遇到闭合标签 `</...`)。
///
/// `in_element` 为 true 时遇到 `</` 停下、交给调用方消费闭合标签;
/// 为 false(顶层 / if / for 花括号体内)时 `</` 属于多余的闭合标签,报错。
fn parse_children(input: ParseStream, in_element: bool) -> Result<Vec<Node>> {
    let mut nodes = Vec::new();
    loop {
        if input.is_empty() {
            break;
        }
        if input.peek(Token![<]) && input.peek2(Token![/]) {
            if in_element {
                break;
            }
            return Err(Error::new(input.span(), "多余的闭合标签:此处没有待闭合的元素"));
        }
        nodes.push(parse_node(input)?);
    }
    Ok(nodes)
}

fn parse_node(input: ParseStream) -> Result<Node> {
    if input.peek(Token![<]) {
        parse_element(input)
    } else if input.peek(Token![if]) {
        parse_if(input)
    } else if input.peek(Token![for]) {
        parse_for(input)
    } else if input.peek(LitStr) || input.peek(token::Brace) {
        Ok(Node::Text(parse_text_run(input)?))
    } else {
        Err(Error::new(
            input.span(),
            "期望元素(<view>/<text>/<button>)、if/for 块、字符串字面量或 {插值}",
        ))
    }
}

/// 连续文本段:字符串字面量与 `{表达式}` 的任意交替,遇到其它内容停止(合并规则)
fn parse_text_run(input: ParseStream) -> Result<Vec<Segment>> {
    let mut segments = Vec::new();
    loop {
        if input.peek(LitStr) {
            segments.push(Segment::Lit(input.parse()?));
        } else if input.peek(token::Brace) {
            segments.push(parse_interp(input)?);
        } else {
            break;
        }
    }
    Ok(segments)
}

/// `{表达式}` 插值
fn parse_interp(input: ParseStream) -> Result<Segment> {
    let content;
    braced!(content in input);
    let expr: Expr = content.parse()?;
    if !content.is_empty() {
        return Err(content.error("插值花括号内只允许单个表达式"));
    }
    Ok(Segment::Expr(expr))
}

fn parse_element(input: ParseStream) -> Result<Node> {
    input.parse::<Token![<]>()?;
    let name: Ident = input
        .parse()
        .map_err(|e| Error::new(e.span(), "期望标签名(view/text/button)"))?;
    let leaf_kind = match name.to_string().as_str() {
        "view" => None,
        "text" => Some(LeafKind::Text),
        "button" => Some(LeafKind::Button),
        other => {
            return Err(Error::new(
                name.span(),
                format!("未知标签 <{other}>:仅支持 <view>/<text>/<button>"),
            ));
        }
    };
    let attrs = parse_attrs(input)?;

    // 自闭合:`<view />`、`<button ... />`(button label 为空串)
    if input.peek(Token![/]) {
        input.parse::<Token![/]>()?;
        input.parse::<Token![>]>()?;
        return Ok(match leaf_kind {
            None => Node::View(ViewElem { attrs, children: Vec::new() }),
            Some(kind) => Node::Leaf(LeafElem { kind, attrs, segments: Vec::new() }),
        });
    }

    input.parse::<Token![>]>()?;
    let node = match leaf_kind {
        None => {
            let children = parse_children(input, true)?;
            Node::View(ViewElem { attrs, children })
        }
        Some(kind) => {
            let segments = parse_leaf_segments(input, &name)?;
            Node::Leaf(LeafElem { kind, attrs, segments })
        }
    };
    parse_closing_tag(input, &name)?;
    Ok(node)
}

fn parse_attrs(input: ParseStream) -> Result<Vec<Attr>> {
    let mut attrs: Vec<Attr> = Vec::new();
    while !(input.peek(Token![>]) || input.peek(Token![/])) {
        if input.is_empty() {
            return Err(input.error("标签未闭合:期望属性、`>` 或 `/>`"));
        }
        let name: Ident = input
            .parse()
            .map_err(|e| Error::new(e.span(), "期望属性名(style/on_click)、`>` 或 `/>`"))?;
        let kind = match name.to_string().as_str() {
            "style" => AttrKind::Style,
            "on_click" => AttrKind::OnClick,
            other => {
                return Err(Error::new(
                    name.span(),
                    format!("未知属性 `{other}`:仅支持 style(...) 与 on_click(...)"),
                ));
            }
        };
        if attrs.iter().any(|a| a.kind == kind) {
            return Err(Error::new(name.span(), format!("属性 `{name}` 重复")));
        }
        if !input.peek(token::Paren) {
            return Err(Error::new(
                name.span(),
                format!("属性 `{name}` 需要参数:`{name}(表达式)`"),
            ));
        }
        let content;
        parenthesized!(content in input);
        let expr: Expr = content.parse()?;
        if !content.is_empty() {
            return Err(content.error("属性括号内只允许单个表达式"));
        }
        attrs.push(Attr { kind, expr });
    }
    Ok(attrs)
}

/// `<text>` / `<button>` 的内容:只允许文本段,不允许元素 / if / for
fn parse_leaf_segments(input: ParseStream, tag: &Ident) -> Result<Vec<Segment>> {
    let mut segments = Vec::new();
    loop {
        if input.peek(Token![<]) && input.peek2(Token![/]) {
            break;
        }
        if input.is_empty() {
            return Err(Error::new(tag.span(), format!("<{tag}> 缺少闭合标签 </{tag}>")));
        }
        if input.peek(LitStr) {
            segments.push(Segment::Lit(input.parse()?));
        } else if input.peek(token::Brace) {
            segments.push(parse_interp(input)?);
        } else if input.peek(Token![<]) || input.peek(Token![if]) || input.peek(Token![for]) {
            return Err(Error::new(
                input.span(),
                format!("<{tag}> 内只允许字符串字面量与 {{插值}},不允许子元素或 if/for 块"),
            ));
        } else {
            return Err(Error::new(input.span(), "期望字符串字面量或 {插值}"));
        }
    }
    Ok(segments)
}

/// 消费 `</tag>`,并检查与开标签匹配
fn parse_closing_tag(input: ParseStream, open: &Ident) -> Result<()> {
    if input.is_empty() {
        return Err(Error::new(open.span(), format!("<{open}> 缺少闭合标签 </{open}>")));
    }
    input.parse::<Token![<]>()?;
    input.parse::<Token![/]>()?;
    let name: Ident = input
        .parse()
        .map_err(|e| Error::new(e.span(), "期望闭合标签名"))?;
    if name != *open {
        return Err(Error::new(
            name.span(),
            format!("闭合标签 </{name}> 与开标签 <{open}> 不匹配"),
        ));
    }
    input.parse::<Token![>]>()?;
    Ok(())
}

fn parse_if(input: ParseStream) -> Result<Node> {
    input.parse::<Token![if]>()?;
    // 与 rustc 的 if 条件同款:不贪心吃 `{`,否则条件会把块体当成结构体字面量
    let cond = input.call(Expr::parse_without_eager_brace)?;
    let body;
    braced!(body in input);
    let then_nodes = parse_children(&body, false)?;
    let mut else_nodes = Vec::new();
    if input.peek(Token![else]) {
        input.parse::<Token![else]>()?;
        if input.peek(Token![if]) {
            // else-if 脱糖:else 分支就是一个嵌套 if 节点
            else_nodes.push(parse_if(input)?);
        } else if input.peek(token::Brace) {
            let body;
            braced!(body in input);
            else_nodes = parse_children(&body, false)?;
        } else {
            return Err(input.error("else 后期望 `if` 或 `{ ... }`"));
        }
    }
    Ok(Node::If(IfNode { cond, then_nodes, else_nodes }))
}

fn parse_for(input: ParseStream) -> Result<Node> {
    input.parse::<Token![for]>()?;
    let pat = input.call(Pat::parse_single)?;
    let index = if input.peek(Token![,]) {
        input.parse::<Token![,]>()?;
        Some(input.parse::<Ident>()?)
    } else {
        None
    };
    input.parse::<Token![in]>()?;
    let items = input.call(Expr::parse_without_eager_brace)?;
    let body;
    braced!(body in input);
    let body = parse_children(&body, false)?;
    Ok(Node::For(ForNode { pat, index, items, body }))
}
