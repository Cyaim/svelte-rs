//! 共享内核的宏路径(`ExprSrc::Tokens`)契约测试——ADR-2 内核合并的
//! span 硬约束在 codegen 侧的防护:
//!
//! 1. **span 恒等**:用户表达式 token 进 IR、出 codegen,span 必须逐一原样
//!    (任何 to_string()/re-parse 的偷懒都会把 span 退化成 call_site,在此翻红);
//! 2. **不过 runes 改写**:宏表达式是最终 Rust,`n` 不得被改写成 `n.get()`;
//! 3. 宏专属分叉(style 闭包 / placeholder 表达式)发射正确的原语。
//!
//! 输入 token 用 `TokenStream::from_str` 构造:fallback span 携带字符串内的
//! 真实字节区间(span-locations),`{:?}` 可比较。

use proc_macro2::{TokenStream, TokenTree};
use std::str::FromStr;
use sv_compiler::template::{Arm, Attr, AttrValue, ExprSrc, Node, Segment, Tag};

/// 在 token 流里找第一个名为 `name` 的 ident,返回其 span 的 Debug 表示
fn find_ident_span(ts: &TokenStream, name: &str) -> Option<String> {
    for tt in ts.clone() {
        match tt {
            TokenTree::Ident(id) if id == name => return Some(format!("{:?}", id.span())),
            TokenTree::Group(g) => {
                if let Some(s) = find_ident_span(&g.stream(), name) {
                    return Some(s);
                }
            }
            _ => {}
        }
    }
    None
}

#[test]
fn tokens_expr_spans_survive_codegen_verbatim() {
    // 唯一名字,避免与脚手架 ident 撞车
    let expr = TokenStream::from_str("uniq_source_marker + 1").unwrap();
    let span_in = find_ident_span(&expr, "uniq_source_marker").expect("输入里应有该 ident");
    // fallback span 应携带真实字节区间(不是 call_site 的 0..0)——
    // 若环境未启 span-locations 此断言会先揭穿测试自身失效
    assert_ne!(
        span_in,
        format!("{:?}", proc_macro2::Span::call_site()),
        "输入 span 不应等于 call_site,否则本测试测不到直通"
    );

    let nodes = vec![Node::Text {
        segments: vec![Segment::Expr(ExprSrc::Tokens(expr))],
    }];
    let out = sv_compiler::generate_template(&nodes).expect("宏路径 codegen 不应失败");
    let span_out = find_ident_span(&out, "uniq_source_marker").expect("输出里应保留该 ident");
    assert_eq!(span_in, span_out, "用户表达式 token 的 span 必须原样直通");
}

#[test]
fn tokens_exprs_are_never_rune_rewritten() {
    // `.svelte` 路径会把反应式变量 `n` 改写成 `n.get()`;宏路径不许发生
    let expr = TokenStream::from_str("n").unwrap();
    let nodes = vec![Node::Text {
        segments: vec![Segment::Expr(ExprSrc::Tokens(expr))],
    }];
    let out = sv_compiler::generate_template(&nodes).unwrap().to_string();
    assert!(
        !out.contains("n . get ()"),
        "宏表达式不得过 runes 改写:{out}"
    );
    assert!(out.contains("bind_text"), "插值应发射 bind_text:{out}");
}

#[test]
fn tokens_closures_keep_user_capture_mode() {
    // .svelte 的事件处理器会被 force-move;宏闭包的 capture 由用户自己定,
    // 不许被共享 codegen 加 move
    let handler = TokenStream::from_str("|| count.set(1)").unwrap();
    let nodes = vec![Node::Element {
        tag: Tag::Button,
        attrs: vec![Attr {
            name: "onclick".into(),
            value: AttrValue::Expr(ExprSrc::Tokens(handler)),
            offset: 0,
        }],
        children: vec![Node::Text {
            segments: vec![Segment::Static("x".into())],
        }],
        offset: 0,
    }];
    let out = sv_compiler::generate_template(&nodes).unwrap().to_string();
    assert!(out.contains("set_on_click"), "{out}");
    assert!(
        out.contains("| | count . set (1)") && !out.contains("move | |"),
        "闭包应原样直通、不被强加 move:{out}"
    );
}

#[test]
fn macro_style_closure_emits_bind_style() {
    let closure = TokenStream::from_str("move |s| s.padding = 4.0").unwrap();
    let nodes = vec![Node::Element {
        tag: Tag::View,
        attrs: vec![Attr {
            name: "style".into(),
            value: AttrValue::Expr(ExprSrc::Tokens(closure)),
            offset: 0,
        }],
        children: vec![],
        offset: 0,
    }];
    let out = sv_compiler::generate_template(&nodes).unwrap().to_string();
    assert!(
        out.contains("bind_style"),
        "style 闭包应直发 bind_style:{out}"
    );
}

#[test]
fn macro_placeholder_expr_passes_through() {
    let value = TokenStream::from_str("hint_text()").unwrap();
    let nodes = vec![Node::Element {
        tag: Tag::Input,
        attrs: vec![Attr {
            name: "placeholder".into(),
            value: AttrValue::Expr(ExprSrc::Tokens(value)),
            offset: 0,
        }],
        children: vec![],
        offset: 0,
    }];
    let out = sv_compiler::generate_template(&nodes).unwrap().to_string();
    assert!(
        out.contains("set_placeholder") && out.contains("hint_text ()"),
        "placeholder 表达式应直传:{out}"
    );
}

#[test]
fn each_pat_and_index_tokens_survive() {
    let list = TokenStream::from_str("items.clone()").unwrap();
    let pat = TokenStream::from_str("(a, uniq_pat_marker)").unwrap();
    let idx = TokenStream::from_str("uniq_idx_marker").unwrap();
    let span_pat = find_ident_span(&pat, "uniq_pat_marker").unwrap();
    let span_idx = find_ident_span(&idx, "uniq_idx_marker").unwrap();
    let body_expr = TokenStream::from_str("uniq_pat_marker").unwrap();
    let nodes = vec![Node::Each {
        list: ExprSrc::Tokens(list),
        pat: ExprSrc::Tokens(pat),
        index: Some(ExprSrc::Tokens(idx)),
        key: None,
        children: vec![Node::Text {
            segments: vec![Segment::Expr(ExprSrc::Tokens(body_expr))],
        }],
        else_children: vec![],
        offset: 0,
    }];
    let out = sv_compiler::generate_template(&nodes).expect("each 宏路径应成功");
    assert_eq!(
        find_ident_span(&out, "uniq_pat_marker")
            .as_deref()
            .map(|s| s.split("..").next().unwrap().to_string()),
        Some(span_pat.split("..").next().unwrap().to_string()),
        "each 模式的 token span 应直通(绑定处)"
    );
    assert_eq!(
        find_ident_span(&out, "uniq_idx_marker").as_deref(),
        Some(span_idx.as_str()),
        "索引 ident 的 span 应直通"
    );
    assert!(out.to_string().contains("each_block"), "应发射 each_block");
}

/// 宏路径不过预克隆(硬约束):each 行值只有行绑定的那一次克隆,
/// 不得出现 `.svelte` 式的节点级/闭包级 `Clone::clone(&名字)` 注入——
/// 兄弟节点复用行值应交给 rustc 报借用错误,而不是隐式克隆
#[test]
fn tokens_each_row_gets_exactly_one_clone() {
    let nodes = vec![Node::Each {
        list: ExprSrc::Tokens(TokenStream::from_str("items.get()").unwrap()),
        pat: ExprSrc::Tokens(TokenStream::from_str("row_item").unwrap()),
        index: None,
        key: None,
        children: vec![
            Node::Text {
                segments: vec![Segment::Expr(ExprSrc::Tokens(
                    TokenStream::from_str("row_item").unwrap(),
                ))],
            },
            Node::If {
                arms: vec![Arm {
                    cond: ExprSrc::Tokens(TokenStream::from_str("cond_flag").unwrap()),
                    children: vec![Node::Text {
                        segments: vec![Segment::Static("x".into())],
                    }],
                }],
                else_children: vec![],
                offset: 0,
            },
        ],
        else_children: vec![],
        offset: 0,
    }];
    let out = sv_compiler::generate_template(&nodes).unwrap().to_string();
    assert_eq!(
        out.matches("Clone :: clone").count(),
        1,
        "each 行应只有行绑定一次克隆,不得注入预克隆:{out}"
    );
    assert!(
        !out.contains("Clone :: clone (& row_item)"),
        "宏路径不得对行绑定做预克隆:{out}"
    );
}

/// 空 for 体退化为空行闭包(与旧宏一致):不发行绑定,宏用户不会收到
/// unused_variables 告警(宏展开没有 SFC 的 #[allow] 伞)
#[test]
fn tokens_each_empty_body_emits_empty_row() {
    let nodes = vec![Node::Each {
        list: ExprSrc::Tokens(TokenStream::from_str("xs.clone()").unwrap()),
        pat: ExprSrc::Tokens(TokenStream::from_str("unused_row").unwrap()),
        index: None,
        key: None,
        children: vec![],
        else_children: vec![],
        offset: 0,
    }];
    let out = sv_compiler::generate_template(&nodes).unwrap().to_string();
    assert!(
        out.contains("| _ , _ , _ , _ | { }"),
        "空行体应发空闭包:{out}"
    );
    assert!(
        !out.contains("Clone :: clone"),
        "空行体不应发行绑定克隆:{out}"
    );
}

/// 单臂 If + else 嵌套(宏的 else-if 脱糖形态)在共享内核上语义成立
#[test]
fn macro_if_else_chain_emits_nested_if_blocks() {
    let mk_text = |s: &str| Node::Text {
        segments: vec![Segment::Static(s.into())],
    };
    let nodes = vec![Node::If {
        arms: vec![Arm {
            cond: ExprSrc::Tokens(TokenStream::from_str("flag_a").unwrap()),
            children: vec![mk_text("a")],
        }],
        else_children: vec![Node::If {
            arms: vec![Arm {
                cond: ExprSrc::Tokens(TokenStream::from_str("flag_b").unwrap()),
                children: vec![mk_text("b")],
            }],
            else_children: vec![mk_text("c")],
            offset: 0,
        }],
        offset: 0,
    }];
    let out = sv_compiler::generate_template(&nodes).unwrap().to_string();
    assert_eq!(
        out.matches("if_block").count(),
        2,
        "else-if 脱糖应发射两层 if_block:{out}"
    );
}
