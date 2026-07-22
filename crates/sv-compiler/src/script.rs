//! `<script>` 块的 runes 源变换 —— 编译器路线的核心能力,proc-macro 做不到的部分。
//!
//! 变换规则(对整个 script 作用域):
//! - `let x = $state(expr)`   → `let x = ::sv_reactive::state(expr')`
//! - `let y = $derived(expr)` → `let y = ::sv_reactive::derived(move || expr')`
//! - `$effect(|| ...)`        → `::sv_reactive::effect(move || ...)`
//! - 读位置:裸 `x` → `x.get()`(含 format!/println!/vec! 等常见宏的参数)
//! - 写位置:`x = v` → `x.set(v)`;`x += v` → `x.update(|__v| *__v += v)`
//! - 引用了反应式变量的闭包自动加 `move`(句柄是 Copy,move 零成本)
//! - 显式 `x.get()/.set()/.update()/.with()` 的接收者不再二次包装
//!
//! 遮蔽处理:闭包参数、fn item 参数、match 臂、for 循环、if/while-let 的模式绑定
//! 都会正确遮蔽反应式名字;rune 定位在掩码文本上做(字符串/注释免疫)。
//! 宏参数只对白名单(fmt/断言/vec 系)改写,其它宏里出现反应式变量是硬错误。
//!
//! v0 已知限制(docs/research/08 详述):
//! - 不处理 `let` **重绑定**遮蔽(script 内不要用反应式变量名重新声明普通变量);
//! - `format!("{x}")` 行内捕获形式的 `x` 不会被改写(请用 `{}` + 参数);
//! - 字段/索引级赋值(`pos.x = 1`)不支持,请用 `.update(|v| v.x = 1)`。

use std::collections::{HashMap, HashSet};

use proc_macro2::{TokenStream, TokenTree};
use quote::{ToTokens, quote};
use syn::punctuated::Punctuated;
use syn::visit_mut::{self, VisitMut};
use syn::{BinOp, Block, Expr, Pat, Stmt, Token, parse_quote};

use crate::sfc::Span;
use crate::sourcemap::RuneHit;
use crate::{CompileError, line_col};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VarKind {
    State,
    Derived,
}

/// 反应式变量表:script 里 $state/$derived 声明的名字
#[derive(Default)]
pub struct VarTable {
    map: HashMap<String, VarKind>,
}

impl VarTable {
    pub fn contains(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }
}

pub struct ScriptOutput {
    /// 改写后的 script 语句
    pub stmts: TokenStream,
    pub vars: VarTable,
    /// `$props { ... }` 声明(组件签名)
    pub props: Option<PropsDecl>,
    /// script 顶层的普通 let 绑定名(非 rune)——模板生成的 move 闭包
    /// 捕获它们时需要预克隆,避免所有权被夺走
    pub plain_vars: HashSet<String>,
}

impl ScriptOutput {
    pub fn empty() -> Self {
        ScriptOutput {
            stmts: TokenStream::new(),
            vars: VarTable::default(),
            props: None,
            plain_vars: HashSet::new(),
        }
    }
}

/// `$props { name: Type [= default], ... }` 声明
pub struct PropsDecl {
    pub fields: Vec<PropField>,
}

pub struct PropField {
    pub name: String,
    pub ty: syn::Type,
    pub default: Option<syn::Expr>,
    /// `$bindable(T)`:双向 prop。实际类型是 `Signal<T>`,
    /// 在 callee 里按反应式变量参与 runes 改写(裸读/裸写皆可)
    pub bindable: bool,
}

/// 把 Rust 源码里的字符串/字符字面量与注释替换成**字节等长**空白(保留换行)。
/// rune 的定位一律在掩码后的文本上做——字面量和注释里的 `$xxx` 不是 rune。
/// 多字节字符按其字节数填空格,保证掩码文本与原文的字节偏移一一对应。
fn mask_rust_source(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let bytes: Vec<char> = content.chars().collect();
    fn push_blank(out: &mut String, c: char) {
        if c == '\n' {
            out.push('\n');
        } else {
            for _ in 0..c.len_utf8() {
                out.push(' ');
            }
        }
    }
    let blank = push_blank;
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        // 行注释
        if c == '/' && bytes.get(i + 1) == Some(&'/') {
            while i < bytes.len() && bytes[i] != '\n' {
                blank(&mut out, bytes[i]);
                i += 1;
            }
            continue;
        }
        // 块注释(可嵌套)
        if c == '/' && bytes.get(i + 1) == Some(&'*') {
            let mut level = 0usize;
            while i < bytes.len() {
                if bytes[i] == '/' && bytes.get(i + 1) == Some(&'*') {
                    level += 1;
                    out.push(' ');
                    out.push(' ');
                    i += 2;
                } else if bytes[i] == '*' && bytes.get(i + 1) == Some(&'/') {
                    level -= 1;
                    out.push(' ');
                    out.push(' ');
                    i += 2;
                    if level == 0 {
                        break;
                    }
                } else {
                    blank(&mut out, bytes[i]);
                    i += 1;
                }
            }
            continue;
        }
        // 原始字符串 r"..." / r#"..."#
        if c == 'r' && matches!(bytes.get(i + 1), Some('"') | Some('#')) {
            let mut j = i + 1;
            let mut hashes = 0usize;
            while bytes.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if bytes.get(j) == Some(&'"') {
                out.push(' '); // r
                for _ in 0..hashes {
                    out.push(' ');
                }
                out.push(' '); // 开引号
                j += 1;
                'raw: while j < bytes.len() {
                    if bytes[j] == '"' {
                        let mut k = 0usize;
                        while k < hashes && bytes.get(j + 1 + k) == Some(&'#') {
                            k += 1;
                        }
                        if k == hashes {
                            for _ in 0..=hashes {
                                out.push(' ');
                            }
                            j += 1 + hashes;
                            break 'raw;
                        }
                    }
                    blank(&mut out, bytes[j]);
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        // 普通字符串
        if c == '"' {
            out.push(' ');
            i += 1;
            while i < bytes.len() {
                match bytes[i] {
                    '\\' => {
                        out.push(' ');
                        out.push(' ');
                        i += 2;
                    }
                    '"' => {
                        out.push(' ');
                        i += 1;
                        break;
                    }
                    other => {
                        blank(&mut out, other);
                        i += 1;
                    }
                }
            }
            continue;
        }
        // 字符字面量('a' 形态才算;'a 生命周期不动)
        if c == '\'' {
            let is_char = match bytes.get(i + 1) {
                Some('\\') => true,
                Some(_) => bytes.get(i + 2) == Some(&'\''),
                None => false,
            };
            if is_char {
                out.push(' ');
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        '\\' => {
                            out.push(' ');
                            out.push(' ');
                            i += 2;
                        }
                        '\'' => {
                            out.push(' ');
                            i += 1;
                            break;
                        }
                        other => {
                            blank(&mut out, other);
                            i += 1;
                        }
                    }
                }
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// 从 script 内容里摘出 `$props { ... }` 声明,原位置替换为等长空白
/// (保持行列不变,syn 错误定位不漂移)。定位与花括号配平都在掩码文本上做,
/// 注释/字符串里的 `$props` 不会被误认。
/// `sv_base` = 该 script 内容在 `.sv` 全文里的起始字节;`None` 表示"只要签名,
/// 不建映射"(build() 的第一遍扫描)。
pub fn extract_props(
    content: &str,
    sv_base: Option<usize>,
) -> Result<(String, Option<PropsDecl>), String> {
    let masked = mask_rust_source(content);
    // 只有后跟 `{` 的 `$props` 才是声明;`$props.id()` 等 rune 变体留给后续替换
    let mut decl_starts = Vec::new();
    let mut at = 0usize;
    while let Some(rel) = masked[at..].find("$props") {
        let pos = at + rel;
        let after = &masked[pos + "$props".len()..];
        if let Some(i) = after.find(|c: char| !c.is_whitespace())
            && after[i..].starts_with('{')
        {
            decl_starts.push(pos);
        }
        at = pos + "$props".len();
    }
    if decl_starts.len() > 1 {
        return Err("每个组件只允许一个 $props 声明".to_string());
    }
    let Some(start) = decl_starts.first().copied() else {
        return Ok((content.to_string(), None));
    };
    let after = &masked[start + "$props".len()..];
    let brace_rel = after
        .find('{')
        .filter(|i| after[..*i].trim().is_empty())
        .ok_or("$props 后应跟 { ... } 块")?;
    let body_start = start + "$props".len() + brace_rel + 1;
    // 在掩码文本上找匹配的 '}'(默认值表达式里的字符串不会干扰配平)
    let mut depth = 1usize;
    let mut body_end = None;
    for (i, c) in masked[body_start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    body_end = Some(body_start + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let body_end = body_end.ok_or("$props { ... } 花括号未闭合")?;
    let inner = &content[body_start..body_end];

    struct Field {
        name: syn::Ident,
        ty: syn::Type,
        default: Option<Expr>,
        bindable: bool,
    }
    impl syn::parse::Parse for Field {
        fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
            let name = input.parse()?;
            input.parse::<Token![:]>()?;
            // `$bindable(T)`(已占位替换成 __sv_bindable):
            // syn 的 Type 不接受任意路径的括号参数,手工识别
            let (ty, bindable) = if input.peek(syn::Ident) && input.peek2(syn::token::Paren) {
                let fork = input.fork();
                let id: syn::Ident = fork.parse()?;
                if id == "__sv_bindable" {
                    input.parse::<syn::Ident>()?;
                    let content;
                    syn::parenthesized!(content in input);
                    let inner: syn::Type = content.parse()?;
                    (syn::parse_quote!(::sv_reactive::Signal<#inner>), true)
                } else {
                    (input.parse()?, false)
                }
            } else {
                (input.parse()?, false)
            };
            let default = if input.peek(Token![=]) {
                input.parse::<Token![=]>()?;
                Some(input.parse()?)
            } else {
                None
            };
            Ok(Field {
                name,
                ty,
                default,
                bindable,
            })
        }
    }
    // `$bindable(T)` 在类型位:$ 进不了 syn,先占位替换,借 Fn-sugar 解析。
    // 替换恒 +4 字节,顺手记漂移表——props 的字段类型与默认值是纯用户代码,
    // 组件库里的类型错误多半就出在这儿,没有映射等于这类组件全靠猜
    let mut inner_pre = String::with_capacity(inner.len());
    let mut drift = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel) = inner[cursor..].find("$bindable") {
        let pos = cursor + rel;
        inner_pre.push_str(&inner[cursor..pos]);
        drift.push(RuneHit {
            out_start: inner_pre.len(),
            out_len: "__sv_bindable".len(),
            src_start: pos,
            src_len: "$bindable".len(),
        });
        inner_pre.push_str("__sv_bindable");
        cursor = pos + "$bindable".len();
    }
    inner_pre.push_str(&inner[cursor..]);
    let parser = Punctuated::<Field, Token![,]>::parse_terminated;
    let fields = match sv_base {
        // build() 的第一遍扫描只要签名、不建映射,那时 sv_base 是 None
        Some(base) => {
            crate::sourcemap::parse_with_drift_at(parser, &inner_pre, base + body_start, drift)
        }
        None => syn::parse::Parser::parse_str(parser, &inner_pre),
    }
    .map_err(|e| format!("$props 声明解析失败: {e}"))?;

    let decl = PropsDecl {
        fields: fields
            .into_iter()
            .map(|f| PropField {
                name: f.name.to_string(),
                ty: f.ty,
                default: f.default,
                bindable: f.bindable,
            })
            .collect(),
    };
    // 字节等长空白替换(保留换行;多字节字符按字节数填,行列不漂移)
    let span_text = &content[start..body_end + 1];
    let blank: String = span_text
        .chars()
        .flat_map(|c| {
            if c == '\n' {
                vec!['\n']
            } else {
                vec![' '; c.len_utf8()]
            }
        })
        .collect();
    let mut cleaned = String::with_capacity(content.len());
    cleaned.push_str(&content[..start]);
    cleaned.push_str(&blank);
    cleaned.push_str(&content[body_end + 1..]);
    Ok((cleaned, Some(decl)))
}

pub fn transform(source: &str, span: &Span) -> Result<ScriptOutput, CompileError> {
    let content = span.text(source);
    let (content, props) = extract_props(content, Some(span.start))
        .map_err(|msg| CompileError::at_offset(source, span.start, msg))?;
    // `$` 不是合法 Rust token,先做占位替换再交给 syn。
    // (`$state.raw` / `$derived.by` / `$effect.pre` 因前缀替换自动变成方法调用形态)
    let (pre, rune_hits) = replace_runes(&content);
    // 垫虚拟行再 parse:整块一次,pad 之外还要减掉 `{\n` 这 2 字节前缀
    // (source map 的 provenance 原料,见 sourcemap.rs 头部)
    let block: Block = crate::sourcemap::parse_script_block(&pre, span.start, rune_hits)
        .map_err(|e| syn_err(source, span, &e, "script 解析失败"))?;

    let mut vars = VarTable::default();
    // $bindable props 在 callee 里就是反应式变量:预注册进变量表,
    // script 与模板对它们的裸读/裸写自动改写
    if let Some(decl) = &props {
        for f in decl.fields.iter().filter(|f| f.bindable) {
            vars.map.insert(f.name.clone(), VarKind::State);
        }
    }
    let mut plain_vars = HashSet::new();
    let mut out = TokenStream::new();
    for stmt in block.stmts {
        let is_rune_local = matches!(&stmt, Stmt::Local(l) if l.init.as_ref().is_some_and(|i| is_rune_init(&i.expr)));
        if !is_rune_local && let Stmt::Local(l) = &stmt {
            // 普通 let:记录为"普通变量"(模板闭包捕获它们时需预克隆)
            collect_pat_idents(&l.pat, &mut plain_vars);
        }
        let tokens = rewrite_stmt(source, span, stmt, &mut vars)?;
        out.extend(tokens);
    }
    // 反应式名字从普通集合剔除
    for name in vars.map.keys() {
        plain_vars.remove(name);
    }
    Ok(ScriptOutput {
        stmts: out,
        vars,
        props,
        plain_vars,
    })
}

/// rune 占位替换:只替换掩码文本上命中的位置(注释/字符串免疫),
/// 且要求 token 后不是标识符字符(`$stateful` 不动)。
///
/// 第二个返回值是**字节漂移表**:6 个 rune 全是 `$xxx` → `__sv_xxx`,每处恒 +4,
/// source map 反查 script 块里的字节位置时要把它减回去。
/// (`extract_props` 做的是字节等长空白替换,不漂,不需要建表。)
fn replace_runes(content: &str) -> (String, Vec<RuneHit>) {
    const RUNES: &[(&str, &str)] = &[
        ("$state", "__sv_state"),
        ("$derived", "__sv_derived"),
        ("$effect", "__sv_effect"),
        ("$inspect", "__sv_inspect"),
        ("$sig", "__sv_sig"),
        ("$props", "__sv_props"), // 残留的 $props(位置非法)交给 syn 报错
    ];
    let masked = mask_rust_source(content);
    let mut hits: Vec<(usize, &str, &str)> = Vec::new();
    for (from, to) in RUNES {
        let mut at = 0usize;
        while let Some(rel) = masked[at..].find(from) {
            let pos = at + rel;
            let boundary = masked[pos + from.len()..]
                .chars()
                .next()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
            if boundary {
                hits.push((pos, from, to));
            }
            at = pos + from.len();
        }
    }
    hits.sort_by_key(|(pos, ..)| *pos);
    let mut out = String::with_capacity(content.len());
    let mut drift = Vec::with_capacity(hits.len());
    let mut cursor = 0usize;
    for (pos, from, to) in hits {
        out.push_str(&content[cursor..pos]);
        drift.push(RuneHit {
            out_start: out.len(),
            out_len: to.len(),
            src_start: pos,
            src_len: from.len(),
        });
        out.push_str(to);
        cursor = pos + from.len();
    }
    out.push_str(&content[cursor..]);
    (out, drift)
}

fn is_rune_init(e: &Expr) -> bool {
    match e {
        Expr::Call(c) => rune_kind(&c.func).is_some(),
        // 只有 $derived.by / $state.raw 是声明形态;$state.snapshot 等产生普通值
        Expr::MethodCall(mc) => {
            matches!(mc.receiver.as_ref(), Expr::Path(p)
                if (p.path.is_ident("__sv_state") && mc.method == "raw")
                    || (p.path.is_ident("__sv_derived") && mc.method == "by"))
        }
        _ => false,
    }
}

fn syn_err(source: &str, span: &Span, e: &syn::Error, ctx: &str) -> CompileError {
    // proc-macro2 span-locations:行/列相对于被解析的 wrapped 字符串。
    // wrapped 第 2 行 = script 内容第 1 行 = 源文件第 line_col(span.start) 行;
    // 建 source map 时前面还垫了 script_pad 个虚拟换行,一并减掉
    let lc = e.span().start();
    let script_line = line_col(source, span.start).0;
    CompileError {
        message: format!("{ctx}: {e}"),
        line: script_line + lc.line.saturating_sub(2 + crate::sourcemap::script_pad()),
        col: lc.column + 1,
    }
}

/// 用一次性 Rewriter 执行改写,并把改写期错误(非白名单宏里用反应式变量等)上抛
fn rewrite_checked(
    source: &str,
    span: &Span,
    vars: &VarTable,
    f: impl FnOnce(&mut Rewriter),
) -> Result<(), CompileError> {
    let mut rw = Rewriter::new(vars);
    f(&mut rw);
    match rw.errors.into_iter().next() {
        Some(e) => Err(syn_err(source, span, &e, "runes 改写")),
        None => Ok(()),
    }
}

fn rewrite_stmt(
    source: &str,
    span: &Span,
    mut stmt: Stmt,
    vars: &mut VarTable,
) -> Result<TokenStream, CompileError> {
    // `let x = $derived.by(|| ...)` / `$state.raw(v)`(声明形态的 rune 变体)。
    // 其它方法($state.snapshot 等表达式变体)放行给通用改写处理
    if let Stmt::Local(local) = &mut stmt
        && let Some(init) = &mut local.init
        && let Expr::MethodCall(mc) = init.expr.as_mut()
        && let Expr::Path(p) = mc.receiver.as_ref()
        && ((p.path.is_ident("__sv_derived") && mc.method == "by")
            || (p.path.is_ident("__sv_state") && mc.method == "raw"))
    {
        let (kind, ctor): (VarKind, fn(&Expr) -> TokenStream) = if p.path.is_ident("__sv_derived") {
            // $derived.by(f):f 就是计算闭包
            (
                VarKind::Derived,
                |arg| quote! { ::sv_reactive::derived(#arg) },
            )
        } else {
            // $state.raw(v):本实现无深层响应,等价于 $state
            (VarKind::State, |arg| quote! { ::sv_reactive::state(#arg) })
        };
        let Pat::Ident(pi) = &mut local.pat else {
            return Err(syn_err(
                source,
                span,
                &syn::Error::new_spanned(&local.pat, "x"),
                "rune 只能绑定到简单变量名",
            ));
        };
        pi.mutability = None;
        let name = pi.ident.clone();
        if mc.args.len() != 1 {
            return Err(syn_err(
                source,
                span,
                &syn::Error::new_spanned(&mc.method, "x"),
                "rune 需要恰好一个参数",
            ));
        }
        let mut arg = mc.args.first().unwrap().clone();
        rewrite_checked(source, span, vars, |rw| rw.visit_expr_mut(&mut arg))?;
        if kind == VarKind::Derived
            && let Expr::Closure(c) = &mut arg
            && c.capture.is_none()
        {
            c.capture = Some(Default::default());
        }
        let init_ts = ctor(&arg);
        let tokens = quote! { let #name = #init_ts; };
        vars.map.insert(name.to_string(), kind);
        return Ok(tokens);
    }

    // `let x = $state(..) / $derived(..)`
    if let Stmt::Local(local) = &mut stmt
        && let Some(init) = &mut local.init
        && let Expr::Call(call) = init.expr.as_mut()
        && let Some(kind) = rune_kind(&call.func)
    {
        let Pat::Ident(pi) = &mut local.pat else {
            return Err(syn_err(
                source,
                span,
                &syn::Error::new_spanned(&local.pat, "x"),
                "$state/$derived 只能绑定到简单变量名",
            ));
        };
        pi.mutability = None; // 句柄本身不需要 mut
        let name = pi.ident.clone();
        if call.args.len() != 1 {
            return Err(syn_err(
                source,
                span,
                &syn::Error::new_spanned(&call.func, "x"),
                "$state/$derived 需要恰好一个参数",
            ));
        }
        let mut arg = call.args.first().unwrap().clone();
        rewrite_checked(source, span, vars, |rw| rw.visit_expr_mut(&mut arg))?;
        let tokens = match kind {
            VarKind::State => quote! { let #name = ::sv_reactive::state(#arg); },
            VarKind::Derived => quote! { let #name = ::sv_reactive::derived(move || #arg); },
        };
        vars.map.insert(name.to_string(), kind);
        return Ok(tokens);
    }

    // `$effect(...)` / `$effect.pre(...)`(pre 的"渲染前"语义待帧调度落地,当前等价)
    let effect_arg = match &mut stmt {
        Stmt::Expr(Expr::Call(call), _) if matches!(call.func.as_ref(), Expr::Path(p) if p.path.is_ident("__sv_effect")) => {
            Some((&mut call.args, "$effect"))
        }
        Stmt::Expr(Expr::MethodCall(mc), _)
            if matches!(mc.receiver.as_ref(), Expr::Path(p) if p.path.is_ident("__sv_effect"))
                && mc.method == "pre" =>
        {
            Some((&mut mc.args, "$effect.pre"))
        }
        _ => None,
    };
    if let Some((args, what)) = effect_arg {
        if args.len() != 1 {
            return Err(syn_err(
                source,
                span,
                &syn::Error::new(proc_macro2::Span::call_site(), "x"),
                &format!("{what} 需要恰好一个闭包参数"),
            ));
        }
        let mut arg = args.first().unwrap().clone();
        rewrite_checked(source, span, vars, |rw| rw.visit_expr_mut(&mut arg))?;
        if let Expr::Closure(c) = &mut arg
            && c.capture.is_none()
        {
            c.capture = Some(Default::default());
        }
        // $effect.pre → 两阶段 flush 的 pre 效应(渲染前)
        let ctor = if what == "$effect.pre" {
            quote! { ::sv_reactive::effect_pre }
        } else {
            quote! { ::sv_reactive::effect }
        };
        return Ok(quote! { #ctor(#arg); });
    }

    // `$inspect(a, b).with(cb)`:自定义观察回调,依赖变化时以值元组调用 cb
    if let Stmt::Expr(Expr::MethodCall(mc), _) = &mut stmt
        && mc.method == "with"
        && let Expr::Call(call) = mc.receiver.as_ref()
        && matches!(call.func.as_ref(), Expr::Path(p) if p.path.is_ident("__sv_inspect"))
    {
        let mut args: Vec<Expr> = call.args.iter().cloned().collect();
        for a in &mut args {
            rewrite_checked(source, span, vars, |rw| rw.visit_expr_mut(a))?;
        }
        let mut cb = mc.args.first().cloned().ok_or_else(|| {
            syn_err(
                source,
                span,
                &syn::Error::new_spanned(&mc.method, "x"),
                "$inspect(...).with 需要回调参数",
            )
        })?;
        rewrite_checked(source, span, vars, |rw| rw.visit_expr_mut(&mut cb))?;
        if let Expr::Closure(c) = &mut cb
            && c.capture.is_none()
        {
            c.capture = Some(Default::default());
        }
        return Ok(quote! {
            ::sv_reactive::effect(move || { (#cb)(( #(#args),* )); });
        });
    }

    // `$inspect(a, b, ...)` → 开发期观察 effect(Debug 打印,依赖变化即输出)
    if let Stmt::Expr(Expr::Call(call), _) = &mut stmt
        && let Expr::Path(p) = call.func.as_ref()
        && p.path.is_ident("__sv_inspect")
    {
        let mut args: Vec<Expr> = call.args.iter().cloned().collect();
        for a in &mut args {
            rewrite_checked(source, span, vars, |rw| rw.visit_expr_mut(a))?;
        }
        return Ok(quote! {
            ::sv_reactive::effect(move || {
                ::std::println!("[inspect] {:?}", ( #(#args),* ));
            });
        });
    }

    // 普通语句:整体改写
    rewrite_checked(source, span, vars, |rw| rw.visit_stmt_mut(&mut stmt))?;
    Ok(stmt.to_token_stream())
}

fn rune_kind(func: &Expr) -> Option<VarKind> {
    let Expr::Path(p) = func else { return None };
    if p.path.is_ident("__sv_state") {
        Some(VarKind::State)
    } else if p.path.is_ident("__sv_derived") {
        Some(VarKind::Derived)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// 读/写位置改写
// ---------------------------------------------------------------------------

pub struct Rewriter<'a> {
    pub vars: &'a VarTable,
    /// 额外的响应式名字(模板 {@const} 产生的块级 derived)
    pub locals: HashSet<String>,
    pub shadowed: HashSet<String>,
    /// 改写过程中发现的必须上报的错误(如非白名单宏里用到反应式变量)
    pub errors: Vec<syn::Error>,
}

impl<'a> Rewriter<'a> {
    pub fn new(vars: &'a VarTable) -> Self {
        Rewriter {
            vars,
            locals: HashSet::new(),
            shadowed: HashSet::new(),
            errors: Vec::new(),
        }
    }

    fn child(&self, shadowed: HashSet<String>) -> Rewriter<'a> {
        Rewriter {
            vars: self.vars,
            locals: self.locals.clone(),
            shadowed,
            errors: Vec::new(),
        }
    }

    fn is_reactive(&self, name: &str) -> bool {
        (self.vars.contains(name) || self.locals.contains(name)) && !self.shadowed.contains(name)
    }

    /// 处理 if/while 条件:改写普通表达式,收集 let-chain 里的模式绑定
    /// (它们遮蔽 then 分支/循环体)
    fn rewrite_cond(&mut self, cond: &mut Expr, shadowed: &mut HashSet<String>) {
        match cond {
            Expr::Let(l) => {
                self.visit_expr_mut(&mut l.expr);
                collect_pat_idents(&l.pat, shadowed);
            }
            Expr::Binary(b) if matches!(b.op, BinOp::And(_)) => {
                self.rewrite_cond(&mut b.left, shadowed);
                self.rewrite_cond(&mut b.right, shadowed);
            }
            other => self.visit_expr_mut(other),
        }
    }
}

/// 参数会被当作表达式列表改写的宏白名单(std 的 fmt/断言/集合系)。
/// 其它宏参数保持原样;若里面出现反应式变量则硬错误,引导先绑快照
const EXPR_ARG_MACROS: &[&str] = &[
    "format",
    "print",
    "println",
    "eprint",
    "eprintln",
    "write",
    "writeln",
    "format_args",
    "panic",
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "vec",
    "dbg",
    "todo",
    "unimplemented",
    "unreachable",
];

/// 模板表达式改写入口(codegen 用)。返回首个改写错误的消息(如有)
pub fn rewrite_template_expr(
    vars: &VarTable,
    locals: &HashSet<String>,
    shadowed: &HashSet<String>,
    expr: &mut Expr,
    force_move_closure: bool,
) -> Result<(), String> {
    let mut rw = Rewriter {
        vars,
        locals: locals.clone(),
        shadowed: shadowed.clone(),
        errors: Vec::new(),
    };
    rw.visit_expr_mut(expr);
    if let Some(e) = rw.errors.first() {
        return Err(e.to_string());
    }
    if force_move_closure
        && let Expr::Closure(c) = expr
        && c.capture.is_none()
    {
        c.capture = Some(Default::default());
    }
    Ok(())
}

fn path_single_ident(e: &Expr) -> Option<String> {
    path_single_ident_spanned(e).map(|(name, _)| name)
}

/// 同上,但顺带把原 Ident 的 span 带出来。
///
/// 为什么需要:下面三处改写(`.set` / `.update` / `.get`)会**重造**用户变量名,
/// 用 `format_ident!` 造出来的是 `Span::call_site()`,在 fallback 下就是
/// `line=1 / byte_range=0..0` —— 恰好等于 source map 判定"胶水"的那个值。
/// 于是每一次对反应式变量的读/写(`.sv` 里最常见、也最常出类型错误的 token)
/// provenance 都会被无声丢弃(全示例实测 98 处)。用原 span 重造即可保住。
fn path_single_ident_spanned(e: &Expr) -> Option<(String, proc_macro2::Span)> {
    if let Expr::Path(p) = e
        && p.qself.is_none()
        && p.path.leading_colon.is_none()
        && p.path.segments.len() == 1
        && p.path.segments[0].arguments.is_none()
    {
        let id = &p.path.segments[0].ident;
        Some((id.to_string(), id.span()))
    } else {
        None
    }
}

fn is_assign_op(op: &BinOp) -> bool {
    matches!(
        op,
        BinOp::AddAssign(_)
            | BinOp::SubAssign(_)
            | BinOp::MulAssign(_)
            | BinOp::DivAssign(_)
            | BinOp::RemAssign(_)
            | BinOp::BitXorAssign(_)
            | BinOp::BitAndAssign(_)
            | BinOp::BitOrAssign(_)
            | BinOp::ShlAssign(_)
            | BinOp::ShrAssign(_)
    )
}

/// token 级扫描:流里是否出现满足谓词的 ident(用于闭包 move 注入判断)
fn tokens_reference(ts: TokenStream, pred: &dyn Fn(&str) -> bool) -> bool {
    for tt in ts {
        match tt {
            TokenTree::Ident(id) => {
                if pred(&id.to_string()) {
                    return true;
                }
            }
            TokenTree::Group(g) if tokens_reference(g.stream(), pred) => return true,
            _ => {}
        }
    }
    false
}

/// token 级扫描:收集满足谓词的 ident(去重)
fn collect_matching_idents(ts: TokenStream, pred: &dyn Fn(&str) -> bool, out: &mut Vec<String>) {
    for tt in ts {
        match tt {
            TokenTree::Ident(id) => {
                let s = id.to_string();
                if pred(&s) && !out.contains(&s) {
                    out.push(s);
                }
            }
            TokenTree::Group(g) => collect_matching_idents(g.stream(), pred, out),
            _ => {}
        }
    }
}

/// 收集模式里绑定的所有 ident(each 的行模式、闭包参数 → 遮蔽表)
pub fn collect_pat_idents(pat: &Pat, out: &mut HashSet<String>) {
    match pat {
        Pat::Ident(pi) => {
            out.insert(pi.ident.to_string());
            if let Some((_, sub)) = &pi.subpat {
                collect_pat_idents(sub, out);
            }
        }
        Pat::Tuple(t) => t.elems.iter().for_each(|p| collect_pat_idents(p, out)),
        Pat::TupleStruct(t) => t.elems.iter().for_each(|p| collect_pat_idents(p, out)),
        Pat::Struct(s) => s
            .fields
            .iter()
            .for_each(|f| collect_pat_idents(&f.pat, out)),
        Pat::Slice(s) => s.elems.iter().for_each(|p| collect_pat_idents(p, out)),
        Pat::Or(o) => o.cases.iter().for_each(|p| collect_pat_idents(p, out)),
        Pat::Paren(p) => collect_pat_idents(&p.pat, out),
        Pat::Reference(r) => collect_pat_idents(&r.pat, out),
        Pat::Type(t) => collect_pat_idents(&t.pat, out),
        _ => {}
    }
}

impl VisitMut for Rewriter<'_> {
    fn visit_expr_mut(&mut self, e: &mut Expr) {
        match e {
            // $sig(x) 逃生舱:取出裸句柄,内部不做任何改写
            Expr::Call(call) if matches!(call.func.as_ref(), Expr::Path(p) if p.path.is_ident("__sv_sig")) =>
            {
                if call.args.len() == 1 {
                    *e = call.args.first().unwrap().clone();
                }
                return;
            }
            // x = v  →  x.set(v)
            Expr::Assign(assign) => {
                if let Some((name, sp)) = path_single_ident_spanned(&assign.left)
                    && self.is_reactive(&name)
                {
                    self.visit_expr_mut(&mut assign.right);
                    // 用原 span 重造,别用 format_ident!(会丢 provenance,见函数注释)
                    let ident = syn::Ident::new(&name, sp);
                    let rhs = &assign.right;
                    *e = parse_quote!( #ident.set(#rhs) );
                    return;
                }
            }
            // x += v  →  { let __sv_rhs = v; x.update(|__v| *__v += __sv_rhs) }
            // RHS 必须在 update 闭包外预求值:否则 `count += count` 这类
            // RHS 读同一 signal 的写法会在闭包内重入读取而 panic(调研 08 §2.3)
            Expr::Binary(b) if is_assign_op(&b.op) => {
                if let Some((name, sp)) = path_single_ident_spanned(&b.left)
                    && self.is_reactive(&name)
                {
                    self.visit_expr_mut(&mut b.right);
                    let ident = syn::Ident::new(&name, sp);
                    let op = &b.op;
                    let rhs = &b.right;
                    *e = parse_quote!( { let __sv_rhs = #rhs; #ident.update(|__v| *__v #op __sv_rhs) } );
                    return;
                }
            }
            // 表达式位的 rune 变体:$state.snapshot / $props.id / $effect.tracking
            // / $effect.root / $effect.pending / $inspect.trace
            Expr::MethodCall(mc)
                if matches!(mc.receiver.as_ref(), Expr::Path(p)
                    if ["__sv_state", "__sv_props", "__sv_effect", "__sv_inspect"]
                        .iter().any(|r| p.path.is_ident(r))) =>
            {
                let recv = match mc.receiver.as_ref() {
                    Expr::Path(p) => p.path.segments[0].ident.to_string(),
                    _ => unreachable!(),
                };
                let method = mc.method.to_string();
                match (recv.as_str(), method.as_str()) {
                    // $state.snapshot(x):本实现无 Proxy,读出来就是普通值
                    ("__sv_state", "snapshot") if mc.args.len() == 1 => {
                        let mut arg = mc.args.first().unwrap().clone();
                        self.visit_expr_mut(&mut arg);
                        *e = parse_quote!( (#arg) );
                    }
                    ("__sv_props", "id") => {
                        *e = parse_quote!(::sv_reactive::unique_id());
                    }
                    ("__sv_effect", "tracking") => {
                        *e = parse_quote!(::sv_reactive::is_tracking());
                    }
                    // $effect.root(f) → 返回销毁闭包
                    ("__sv_effect", "root") if mc.args.len() == 1 => {
                        let mut arg = mc.args.first().unwrap().clone();
                        self.visit_expr_mut(&mut arg);
                        if let Expr::Closure(c) = &mut arg
                            && c.capture.is_none()
                        {
                            c.capture = Some(Default::default());
                        }
                        *e = parse_quote!({
                            let (_, __sv_root) = ::sv_reactive::create_root(#arg);
                            move || __sv_root.dispose()
                        });
                    }
                    // $effect.pending():进行中的 {#await}/后台任务数(响应式)
                    ("__sv_effect", "pending") => {
                        *e = parse_quote!(::sv_ui::tasks::pending_count());
                    }
                    // $inspect.trace(标签):所在 effect 每次重跑时打印
                    ("__sv_inspect", "trace") => {
                        let label = mc
                            .args
                            .first()
                            .cloned()
                            .map(|a| quote! { #a })
                            .unwrap_or_else(|| quote! { "trace" });
                        *e = parse_quote!(::std::println!("[trace] {} 重跑", #label));
                    }
                    // 这些留给 rewrite_stmt 的语句级处理($derived.by/$state.raw 等)
                    ("__sv_state", "raw") | ("__sv_derived", _) => {
                        visit_mut::visit_expr_mut(self, e);
                    }
                    _ => {
                        self.errors.push(syn::Error::new_spanned(
                            &mc.method,
                            format!("未知 rune 变体 `.{method}`"),
                        ));
                    }
                }
                return;
            }
            // 显式句柄方法调用的接收者不再包 .get()
            Expr::MethodCall(mc) => {
                let m = mc.method.to_string();
                if matches!(
                    m.as_str(),
                    "get" | "set" | "update" | "with" | "get_untracked" | "with_untracked"
                ) && path_single_ident(&mc.receiver).is_some_and(|n| self.is_reactive(&n))
                {
                    for arg in mc.args.iter_mut() {
                        self.visit_expr_mut(arg);
                    }
                    return;
                }
            }
            // 闭包:参数遮蔽 + 引用反应式变量时自动 move
            Expr::Closure(c) => {
                let mut shadowed = self.shadowed.clone();
                for p in &c.inputs {
                    collect_pat_idents(p, &mut shadowed);
                }
                let mut inner = self.child(shadowed);
                inner.visit_expr_mut(&mut c.body);
                if c.capture.is_none() {
                    let refs =
                        tokens_reference(c.body.to_token_stream(), &|n| inner.is_reactive(n));
                    if refs {
                        c.capture = Some(Default::default());
                    }
                }
                self.errors.extend(inner.errors);
                return;
            }
            // for 循环:模式遮蔽循环体
            Expr::ForLoop(fl) => {
                self.visit_expr_mut(&mut fl.expr);
                let mut shadowed = self.shadowed.clone();
                collect_pat_idents(&fl.pat, &mut shadowed);
                let mut inner = self.child(shadowed);
                inner.visit_block_mut(&mut fl.body);
                self.errors.extend(inner.errors);
                return;
            }
            // if let / let-chain:let 的模式遮蔽 then 分支
            Expr::If(i) => {
                let mut shadowed = self.shadowed.clone();
                self.rewrite_cond(&mut i.cond, &mut shadowed);
                let mut inner = self.child(shadowed);
                inner.visit_block_mut(&mut i.then_branch);
                self.errors.extend(inner.errors);
                if let Some((_, else_e)) = &mut i.else_branch {
                    self.visit_expr_mut(else_e);
                }
                return;
            }
            // while let 同理
            Expr::While(w) => {
                let mut shadowed = self.shadowed.clone();
                self.rewrite_cond(&mut w.cond, &mut shadowed);
                let mut inner = self.child(shadowed);
                inner.visit_block_mut(&mut w.body);
                self.errors.extend(inner.errors);
                return;
            }
            // 裸读:x → x.get()
            Expr::Path(_) => {
                if let Some((name, sp)) = path_single_ident_spanned(e)
                    && self.is_reactive(&name)
                {
                    let ident = syn::Ident::new(&name, sp);
                    *e = parse_quote!( #ident.get() );
                    return;
                }
            }
            _ => {}
        }
        visit_mut::visit_expr_mut(self, e);
    }

    // match 臂:模式遮蔽 guard 与臂体
    fn visit_arm_mut(&mut self, arm: &mut syn::Arm) {
        let mut shadowed = self.shadowed.clone();
        collect_pat_idents(&arm.pat, &mut shadowed);
        let mut inner = self.child(shadowed);
        if let Some((_, guard)) = &mut arm.guard {
            inner.visit_expr_mut(guard);
        }
        inner.visit_expr_mut(&mut arm.body);
        self.errors.extend(inner.errors);
    }

    // script 内的 fn item:参数遮蔽函数体
    fn visit_item_fn_mut(&mut self, f: &mut syn::ItemFn) {
        let mut shadowed = self.shadowed.clone();
        for input in &f.sig.inputs {
            if let syn::FnArg::Typed(pt) = input {
                collect_pat_idents(&pt.pat, &mut shadowed);
            }
        }
        let mut inner = self.child(shadowed);
        inner.visit_block_mut(&mut f.block);
        self.errors.extend(inner.errors);
    }

    // 结构体字面量简写 `Foo { count }`:改写前补上冒号,避免生成非法简写
    fn visit_field_value_mut(&mut self, fv: &mut syn::FieldValue) {
        if fv.colon_token.is_none()
            && path_single_ident(&fv.expr).is_some_and(|n| self.is_reactive(&n))
        {
            fv.colon_token = Some(Default::default());
        }
        visit_mut::visit_field_value_mut(self, fv);
    }

    // 宏参数:**只对白名单宏**(fmt/断言/vec 等,参数确定是表达式列表)做改写;
    // 其它宏(matches!/stringify!/自定义宏)参数可能是模式或字面 token,
    // 改写会破坏语义——保持原样,但若里面出现反应式变量则硬错误引导显式处理
    fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
        let name = mac
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        if !EXPR_ARG_MACROS.contains(&name.as_str()) {
            let mut used = Vec::new();
            collect_matching_idents(mac.tokens.clone(), &|n| self.is_reactive(n), &mut used);
            if let Some(first) = used.first() {
                self.errors.push(syn::Error::new_spanned(
                    &mac.path,
                    format!(
                        "宏 {name}! 的参数不会被 runes 改写,但里面用到了响应式变量 `{first}`:\
                         请先 `let 快照 = {first};` 再传入,或改用显式 `.get()`"
                    ),
                ));
            }
            return;
        }
        if let Ok(mut args) = mac.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated) {
            for a in args.iter_mut() {
                self.visit_expr_mut(a);
            }
            mac.tokens = args.to_token_stream();
            return;
        }
        struct Repeat(Expr, Expr);
        impl syn::parse::Parse for Repeat {
            fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
                let a = input.parse()?;
                input.parse::<Token![;]>()?;
                let b = input.parse()?;
                Ok(Repeat(a, b))
            }
        }
        if let Ok(mut rep) = mac.parse_body::<Repeat>() {
            self.visit_expr_mut(&mut rep.0);
            self.visit_expr_mut(&mut rep.1);
            let (a, b) = (&rep.0, &rep.1);
            mac.tokens = quote! { #a; #b };
        }
    }
}
