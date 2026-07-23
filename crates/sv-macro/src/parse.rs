//! 手写递归下降解析:`ParseStream` → 共享模板 IR(`sv_compiler::template`)。
//!
//! 语法(v0):
//! ```text
//! view! { doc_expr, parent_expr => 节点* }
//! 节点     := 元素 | if 块 | for 块 | 文本段+
//! 元素     := "<view" 属性* (">" 节点* "</view>" | "/>")
//!           | "<text" 属性* (">" 段* "</text>" | "/>")
//!           | "<button" 属性* (">" 段* "</button>" | "/>")
//!           | "<input" 属性* "/>"
//! 属性     := 属性名 "(" 表达式 ")"(名表见 ATTRS:style/on_click/… 13 个)
//! 段       := 字符串字面量 | "{" 表达式 "}"
//! if 块    := "if" 表达式 "{" 节点* "}" ("else" (if 块 | "{" 节点* "}"))?
//! for 块   := "for" 模式 ("," 索引ident)? "in" 表达式 "{" 节点* "}"
//! ```
//!
//! 所有错误都是带准确 span 的 [`syn::Error`],绝不 panic。**表面语法的全部
//! 校验都发生在这里**(带真 span);产出的 IR 进入共享 codegen 后不会再有
//! 用户可见的错误(ADR-2 内核合并的 span 精度约束)。
//!
//! 表达式与模式以 [`ExprSrc::Tokens`] 形态进 IR:原 token 原 span 直通,
//! 共享 codegen 不做任何改写(runes/预克隆是 `.svelte` 前端的语义)。

use quote::ToTokens;
use syn::parse::{Parse, ParseStream};
use syn::{Error, Expr, Ident, LitStr, Pat, Result, Token, braced, parenthesized, token};

use sv_compiler::template::{Arm, Attr, AttrValue, ExprSrc, Node, Segment, Tag};

/// `view! { doc_expr, parent_expr => 模板... }` 整体
pub struct ViewInput {
    pub doc: Expr,
    pub parent: Expr,
    pub nodes: Vec<Node>,
}

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

/// 表达式 → IR 载荷(带真 span 的 token 直通)
fn tokens(expr: &Expr) -> ExprSrc {
    ExprSrc::Tokens(expr.to_token_stream())
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
            return Err(Error::new(
                input.span(),
                "多余的闭合标签:此处没有待闭合的元素",
            ));
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
        Ok(Node::Text {
            segments: parse_text_run(input)?,
        })
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
            let lit: LitStr = input.parse()?;
            segments.push(Segment::Static(lit.value()));
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
    Ok(Segment::Expr(tokens(&expr)))
}

fn parse_element(input: ParseStream) -> Result<Node> {
    input.parse::<Token![<]>()?;
    let name: Ident = input
        .parse()
        .map_err(|e| Error::new(e.span(), "期望标签名(view/text/button)"))?;
    let (tag, is_leaf) = match name.to_string().as_str() {
        "view" => (Tag::View, false),
        "text" => (Tag::Text, true),
        "button" => (Tag::Button, true),
        "input" => (Tag::Input, true),
        other => {
            return Err(Error::new(
                name.span(),
                format!("未知标签 <{other}>:仅支持 <view>/<text>/<button>/<input>"),
            ));
        }
    };
    let attrs = parse_attrs(input, &tag)?;

    // 自闭合:`<view />`、`<button ... />`(button label 为空串)
    if input.peek(Token![/]) {
        input.parse::<Token![/]>()?;
        input.parse::<Token![>]>()?;
        return Ok(Node::Element {
            tag,
            attrs,
            children: Vec::new(),
            offset: 0,
        });
    }

    input.parse::<Token![>]>()?;
    let children = if is_leaf {
        let segments = parse_leaf_segments(input, &name)?;
        if tag == Tag::Input && !segments.is_empty() {
            return Err(Error::new(
                name.span(),
                "<input> 无内容(值走 bind_value 绑定),请自闭合",
            ));
        }
        if segments.is_empty() {
            Vec::new()
        } else {
            vec![Node::Text { segments }]
        }
    } else {
        parse_children(input, true)?
    };
    parse_closing_tag(input, &name)?;
    Ok(Node::Element {
        tag,
        attrs,
        children,
        offset: 0,
    })
}

/// 属性名表:宏表面语法(`on_click(闭包)` 方法形态)→ 共享 IR 属性名
/// (`.svelte` 的 `onclick={闭包}` 词汇)。第三列 = 仅限 `<input>` 的属性
/// (共享 codegen 有同款标签守卫,但**必须在这里先拦**——错误要带属性名的
/// 真 span,进了 codegen 就只剩宏调用点整体 span 了)。
/// 表面语法校验在此,发射在共享 codegen
const ATTRS: &[(&str, &str, bool)] = &[
    ("style", "style", false),
    ("on_click", "onclick", false),
    ("on_key_down", "onkeydown", false),
    ("on_key_up", "onkeyup", false),
    ("on_focus", "onfocus", false),
    ("on_blur", "onblur", false),
    ("placeholder", "placeholder", true),
    ("bind_value", "bind:value", true),
    ("on_input", "oninput", true),
    ("on_submit", "onsubmit", true),
    ("on_scroll", "onscroll", false),
    ("aria_label", "aria-label", false),
    ("bind_scroll_y", "bind:scrolly", false),
];

fn parse_attrs(input: ParseStream, tag: &Tag) -> Result<Vec<Attr>> {
    let mut attrs: Vec<Attr> = Vec::new();
    while !(input.peek(Token![>]) || input.peek(Token![/])) {
        if input.is_empty() {
            return Err(input.error("标签未闭合:期望属性、`>` 或 `/>`"));
        }
        let name: Ident = input
            .parse()
            .map_err(|e| Error::new(e.span(), "期望属性名(style/on_click)、`>` 或 `/>`"))?;
        let surface = name.to_string();
        let Some((_, ir_name, input_only)) = ATTRS.iter().find(|(s, _, _)| *s == surface) else {
            return Err(Error::new(
                name.span(),
                format!(
                    "未知属性 `{surface}`:仅支持 style/on_click/on_key_down/on_key_up/on_focus/on_blur/placeholder/bind_value/on_input/on_submit/on_scroll/bind_scroll_y/aria_label"
                ),
            ));
        };
        if *input_only && *tag != Tag::Input {
            return Err(Error::new(
                name.span(),
                format!("属性 `{surface}` 只能用在 <input> 上"),
            ));
        }
        if attrs.iter().any(|a| a.name == *ir_name) {
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
        attrs.push(Attr {
            name: (*ir_name).to_string(),
            value: AttrValue::Expr(tokens(&expr)),
            offset: 0,
        });
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
            return Err(Error::new(
                tag.span(),
                format!("<{tag}> 缺少闭合标签 </{tag}>"),
            ));
        }
        if input.peek(LitStr) {
            let lit: LitStr = input.parse()?;
            segments.push(Segment::Static(lit.value()));
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
        return Err(Error::new(
            open.span(),
            format!("<{open}> 缺少闭合标签 </{open}>"),
        ));
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
            // else-if 脱糖:else 分支就是一个嵌套 if 节点(共享 codegen
            // 对单臂 If 的 else 递归发射,与 .svelte 的多臂形态语义一致)
            else_nodes.push(parse_if(input)?);
        } else if input.peek(token::Brace) {
            let body;
            braced!(body in input);
            else_nodes = parse_children(&body, false)?;
        } else {
            return Err(input.error("else 后期望 `if` 或 `{ ... }`"));
        }
    }
    Ok(Node::If {
        arms: vec![Arm {
            cond: tokens(&cond),
            children: then_nodes,
        }],
        else_children: else_nodes,
        offset: 0,
    })
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
    Ok(Node::Each {
        list: tokens(&items),
        pat: ExprSrc::Tokens(pat.to_token_stream()),
        index: index.map(|id| ExprSrc::Tokens(id.to_token_stream())),
        key: None,
        children: body,
        else_children: Vec::new(),
        offset: 0,
    })
}

/// span 精度回归:错误必须指到用户源码的准确 token(行列),而不是整个宏。
/// 这是 ADR-2 内核合并的硬约束,此前无任何防护。
/// 输入用 `TokenStream::from_str` 构造——fallback span 携带该字符串内的真实
/// 行列(proc-macro2 span-locations),断言得以钉死具体位置(列 0-based)。
#[cfg(test)]
mod tests {
    use super::ViewInput;

    fn err_at(src: &str) -> (usize, usize, String) {
        let ts: proc_macro2::TokenStream = src.parse().expect("词法应合法");
        let Err(err) = syn::parse2::<ViewInput>(ts) else {
            panic!("应解析失败:{src}");
        };
        let start = err.span().start();
        (start.line, start.column, err.to_string())
    }

    #[test]
    fn unknown_tag_error_points_at_tag_name() {
        let (line, col, msg) = err_at("doc, parent => <viw>x</viw>");
        assert!(msg.contains("未知标签"), "{msg}");
        assert_eq!((line, col), (1, 16), "应指到 viw 而非宏整体");
    }

    #[test]
    fn unknown_attr_error_points_at_attr_name() {
        let (line, col, msg) = err_at("doc, parent => <view zzz(1) />");
        assert!(msg.contains("未知属性"), "{msg}");
        assert_eq!((line, col), (1, 21), "应指到 zzz");
    }

    #[test]
    fn mismatched_closing_tag_points_at_closing_name() {
        let (line, col, msg) = err_at("doc, parent => <view></text>");
        assert!(msg.contains("不匹配"), "{msg}");
        assert_eq!((line, col), (1, 23), "应指到闭合标签名 text");
    }

    #[test]
    fn duplicate_attr_error_points_at_second_occurrence() {
        let (line, col, msg) =
            err_at("doc, parent => <button on_click(a) on_click(b)>\"x\"</button>");
        assert!(msg.contains("重复"), "{msg}");
        assert_eq!((line, col), (1, 35), "应指到第二个 on_click");
    }

    #[test]
    fn second_line_errors_keep_line_number() {
        let (line, _col, msg) = err_at("doc, parent =>\n<view badattr(1)></view>");
        assert!(msg.contains("未知属性"), "{msg}");
        assert_eq!(line, 2, "行号应落在用户源码第 2 行");
    }

    /// input 族属性用错标签必须在解析期拦下,错误指到属性名——
    /// 穿透到共享 codegen 只会得到"view! 内部错误"+ 整宏 span(评审发现 #0)
    #[test]
    fn input_only_attr_on_wrong_tag_points_at_attr_name() {
        let (line, col, msg) = err_at(r#"doc, parent => <view placeholder("x") />"#);
        assert!(msg.contains("只能用在"), "{msg}");
        assert_eq!((line, col), (1, 21), "应指到 placeholder 属性名");
        let (_, _, msg) = err_at("doc, parent => <button on_input(f)>\"x\"</button>");
        assert!(msg.contains("只能用在"), "{msg}");
        let (_, _, msg) = err_at("doc, parent => <text bind_value(sig) />");
        assert!(msg.contains("只能用在"), "{msg}");
    }
}
