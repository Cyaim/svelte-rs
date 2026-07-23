//! IR + 变换后的 script → Rust 源码(prettyplease 格式化,人类可读)
//!
//! 所有权纪律(对抗审查后的定型设计):模板里引用的**普通变量**(props 解构、
//! script 普通 let、each 模式绑定)会被生成的 `move` 闭包捕获。为避免
//! use-after-move(E0382)与 `Fn` 闭包移出捕获(E0507),采取两层预克隆:
//! 1. **节点级**:每个节点的生成代码若引用普通变量,包一层
//!    `{ let x = Clone::clone(&x); ... }`——兄弟节点各拿各的克隆,原变量保活;
//! 2. **重建闭包体级**:if/each/key 的重建闭包每次调用开头重新克隆——闭包环境
//!    保有原值,反复调用各拿新克隆。
//!
//! 代价:模板引用的普通变量需要 `Clone`(文档化约束)。

use std::collections::HashSet;

use proc_macro2::{TokenStream, TokenTree};
use quote::{ToTokens, format_ident, quote};

use crate::emit::{self, ElemKind, TextPart};
use crate::script::{self, ScriptOutput, collect_pat_idents};
use crate::style::StyleSheet;
use crate::template::{Arm, Attr, AttrValue, ExprSrc, Node, Segment, Tag};
use crate::{CompileError, PropsRegistry, style};

/// 模板一处作用域
#[derive(Clone, Default)]
struct Scope {
    /// 被模式绑定遮蔽的名字(改写抑制)
    shadowed: HashSet<String>,
    /// {@const} 引入的块级 derived(视为反应式)
    locals: HashSet<String>,
    /// 普通变量(props/script 普通 let/each 绑定)——move 闭包捕获前需预克隆
    plain: HashSet<String>,
    /// {#snippet} 定义的名字(组件 prop 传递时自动包 Rc)
    snippets: HashSet<String>,
}

/// 生成代码的文件头。source map 的生成侧偏移是**最终文件**里的字节偏移,
/// 所以并行走拿到的 unparse 侧偏移要整体加上这一段的长度。
const HEADER: &str = "// 由 sv-compiler 生成,请勿手改。\n";

/// 返回 (生成代码, source map 锚点表)。第二项只在
/// [`crate::sourcemap::begin`] 之后非空;否则是空表,且整条路径与开启前
/// **逐字节一致**(`mapped_output_is_byte_identical` 守这条)。
pub fn generate(
    source: &str,
    fn_name: &str,
    script: &ScriptOutput,
    nodes: &[Node],
    registry: &PropsRegistry,
    sheet: &StyleSheet,
) -> Result<(String, crate::sourcemap::Anchors), CompileError> {
    let mut root_scope = Scope::default();
    root_scope.plain.extend(script.plain_vars.iter().cloned());

    // $props → 组件签名:pub struct XxxProps(+ 默认值关联函数)+ 第三个参数 + 解构
    let (props_struct, props_param, props_destructure) = match &script.props {
        Some(decl) => {
            let props_ty = format_ident!("{}Props", pascal(fn_name));
            let fnames: Vec<_> = decl
                .fields
                .iter()
                .map(|f| format_ident!("{}", f.name))
                .collect();
            let ftys: Vec<_> = decl.fields.iter().map(|f| &f.ty).collect();
            for f in &decl.fields {
                // $bindable 的解构名是反应式变量(script::transform 已预注册),
                // 不进普通变量集合
                if !f.bindable {
                    root_scope.plain.insert(f.name.clone());
                }
            }
            // 默认值在 callee 侧生成关联函数:caller 不接触默认值表达式源码,
            // 求值语境单一且不经过 caller 的 runes 改写
            let mut default_fns = TokenStream::new();
            for f in &decl.fields {
                if let Some(default) = &f.default {
                    let fn_id = format_ident!("default_{}", f.name);
                    let ty = &f.ty;
                    default_fns.extend(quote! {
                        pub fn #fn_id() -> #ty { #default }
                    });
                }
            }
            let impl_block = if default_fns.is_empty() {
                TokenStream::new()
            } else {
                quote! { impl #props_ty { #default_fns } }
            };
            (
                quote! {
                    #[allow(dead_code)]
                    pub struct #props_ty { #( pub #fnames: #ftys ),* }
                    #impl_block
                },
                quote! { , props: #props_ty },
                quote! { let #props_ty { #(#fnames),* } = props; },
            )
        }
        None => (TokenStream::new(), TokenStream::new(), TokenStream::new()),
    };

    let mut cg = Cg {
        source,
        script,
        registry,
        sheet,
        n: 0,
    };
    let body = cg.emit_nodes(nodes, &root_scope)?;
    let script_stmts = &script.stmts;
    let fn_ident = format_ident!("{fn_name}");

    let file_ts = quote! {
        #props_struct
        // 生成代码不参与 lint 门禁:`clippy::all` 之外还要挡 rustc 侧的
        // unused_braces/unused_parens(codegen 为了 hygiene 恒加括号)
        #[allow(
            unused_variables,
            unused_mut,
            unused_braces,
            unused_parens,
            clippy::all
        )]
        pub fn #fn_ident(doc: &::sv_ui::Doc, parent: ::sv_ui::ViewId #props_param) {
            let __doc: ::sv_ui::Doc = doc.clone();
            let __parent = parent;
            #props_destructure
            #script_stmts
            #body
        }
    };
    let file: syn::File = syn::parse2(file_ts).map_err(|e| CompileError {
        message: format!("内部错误:生成代码不合法: {e}"),
        line: 1,
        col: 1,
    })?;
    let formatted = prettyplease::unparse(&file);
    let anchors = crate::sourcemap::build_segs(&file, &formatted, HEADER.len(), source);
    Ok((format!("{HEADER}{formatted}"), anchors))
}

fn pascal(snake: &str) -> String {
    snake
        .split('_')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut c = s.chars();
            let head: String = c
                .next()
                .into_iter()
                .flat_map(|c| c.to_uppercase())
                .collect();
            format!("{head}{}", c.as_str())
        })
        .collect()
}

/// 收集 token 流里出现的、属于 `within` 的 ident(去重,排序保证输出稳定)
fn idents_within(ts: TokenStream, within: &HashSet<String>) -> Vec<String> {
    fn walk(ts: TokenStream, within: &HashSet<String>, out: &mut Vec<String>) {
        for tt in ts {
            match tt {
                TokenTree::Ident(id) => {
                    let s = id.to_string();
                    if within.contains(&s) && !out.contains(&s) {
                        out.push(s);
                    }
                }
                TokenTree::Group(g) => walk(g.stream(), within, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(ts, within, &mut out);
    out.sort();
    out
}

/// 为 `tokens` 里引用到的普通变量生成预克隆语句(无引用则为空)
fn preclones(tokens: &TokenStream, scope: &Scope) -> TokenStream {
    let used = idents_within(tokens.clone(), &scope.plain);
    if used.is_empty() {
        return TokenStream::new();
    }
    let ids: Vec<_> = used.iter().map(|s| format_ident!("{s}")).collect();
    quote! { #( let #ids = ::std::clone::Clone::clone(&#ids); )* }
}

struct Cg<'a> {
    source: &'a str,
    script: &'a ScriptOutput,
    registry: &'a PropsRegistry,
    /// `<style>` 块编译产物(类 + 元素规则,含伪类变体)
    sheet: &'a StyleSheet,
    n: usize,
}

impl Cg<'_> {
    fn fresh(&mut self, prefix: &str) -> syn::Ident {
        self.n += 1;
        format_ident!("__{prefix}{}", self.n)
    }

    fn parse_expr(&self, e: &ExprSrc) -> Result<syn::Expr, CompileError> {
        crate::sourcemap::parse_str_at(&e.src, e.offset).map_err(|err| {
            CompileError::at_offset(self.source, e.offset, format!("表达式解析失败: {err}"))
        })
    }

    /// 解析模板表达式并做 runes 读写改写
    fn expr(
        &self,
        e: &ExprSrc,
        scope: &Scope,
        force_move: bool,
    ) -> Result<syn::Expr, CompileError> {
        let mut expr = self.parse_expr(e)?;
        script::rewrite_template_expr(
            &self.script.vars,
            &scope.locals,
            &scope.shadowed,
            &mut expr,
            force_move,
        )
        .map_err(|msg| CompileError::at_offset(self.source, e.offset, msg))?;
        Ok(expr)
    }

    /// 值闭包(if 条件 / each 列表 / key)的表达式:若引用普通变量,
    /// 包 Clone::clone(&(..)) —— 闭包是被反复调用的 Fn,返回值不能移出环境
    fn value_closure_expr(&self, e: &ExprSrc, scope: &Scope) -> Result<TokenStream, CompileError> {
        let expr = self.expr(e, scope, false)?;
        let ts = expr.to_token_stream();
        if idents_within(ts.clone(), &scope.plain).is_empty() {
            Ok(ts)
        } else {
            Ok(quote! { ::std::clone::Clone::clone(&(#expr)) })
        }
    }

    fn emit_nodes(&mut self, nodes: &[Node], scope: &Scope) -> Result<TokenStream, CompileError> {
        // {@const} 对后续兄弟节点可见 → 逐节点推进本层作用域
        let mut scope = scope.clone();
        let mut ts = TokenStream::new();
        for n in nodes {
            match n {
                Node::Const { name, expr, .. } => {
                    let e = self.expr(expr, &scope, false)?;
                    // 被 move 进 derived 闭包的普通变量先克隆一份(不夺走原变量)
                    let pre = preclones(&e.to_token_stream(), &scope);
                    let name_id = format_ident!("{name}");
                    ts.extend(quote! {
                        let #name_id = { #pre ::sv_reactive::derived(move || #e) };
                    });
                    scope.locals.insert(name.clone());
                }
                // {#snippet}:模板级可复用闭包(先声明后使用,不做 Svelte 式提升)
                Node::Snippet {
                    name,
                    params,
                    children,
                    offset,
                } => {
                    let mut inner = scope.clone();
                    let mut pnames = Vec::new();
                    let mut ptys = Vec::new();
                    for (pname, pty_src) in params {
                        inner.shadowed.insert(pname.clone());
                        inner.plain.insert(pname.clone());
                        pnames.push(format_ident!("{pname}"));
                        let pty: syn::Type = syn::parse_str(pty_src).map_err(|e| {
                            CompileError::at_offset(
                                self.source,
                                *offset,
                                format!("snippet 参数 `{pname}` 类型解析失败: {e}"),
                            )
                        })?;
                        ptys.push(pty);
                    }
                    let children_ts = self.emit_nodes(children, &inner)?;
                    // 定义处克隆一次(捕获),调用处每次再克隆(Fn 反复调用)
                    let pre_capture = preclones(&children_ts, &scope);
                    let pre_call = preclones(&children_ts, &scope);
                    let name_id = format_ident!("{name}");
                    ts.extend(quote! {
                        let #name_id = {
                            #pre_capture
                            move |__doc: &::sv_ui::Doc, __parent: ::sv_ui::ViewId #(, #pnames: #ptys)*| {
                                let __doc: ::sv_ui::Doc = __doc.clone();
                                #pre_call
                                #children_ts
                            }
                        };
                    });
                    scope.plain.insert(name.clone());
                    scope.snippets.insert(name.clone());
                }
                _ => {
                    let node_ts = self.emit_node(n, &scope)?;
                    // 节点级预克隆:兄弟节点各自克隆引用到的普通变量
                    let pre = preclones(&node_ts, &scope);
                    if pre.is_empty() {
                        ts.extend(node_ts);
                    } else {
                        ts.extend(quote! { { #pre #node_ts } });
                    }
                }
            }
        }
        Ok(ts)
    }

    fn emit_node(&mut self, node: &Node, scope: &Scope) -> Result<TokenStream, CompileError> {
        match node {
            Node::Element {
                tag,
                attrs,
                children,
                offset,
            } => self.emit_element(tag, attrs, children, *offset, scope),
            Node::Text { segments } => {
                let el = self.fresh("t");
                let create = self.leaf_create(&el, &Tag::Text, segments, scope)?;
                let append = emit::append(&parent_ident(), &el);
                Ok(quote! { #create #append })
            }
            Node::If {
                arms,
                else_children,
                ..
            } => self.emit_if(arms, else_children, scope),
            Node::Each {
                list,
                pat,
                pat_offset,
                index,
                key,
                children,
                else_children,
                offset,
            } => self.emit_each(
                EachParts {
                    list,
                    pat,
                    pat_offset: *pat_offset,
                    index: index.as_deref(),
                    key: key.as_ref(),
                    children,
                    else_children,
                    offset: *offset,
                },
                scope,
            ),
            Node::Key { key, children, .. } => {
                let key_expr = self.value_closure_expr(key, scope)?;
                let children_ts = self.emit_nodes(children, scope)?;
                let build = self.rebuild_closure(children_ts, scope);
                Ok(quote! {
                    ::sv_ui::key_block(&__doc, __parent, move || #key_expr, #build);
                })
            }
            // {@render name(args)}:调用 snippet 闭包。
            // Svelte 的 snippet 参数是响应式的——这里以参数元组为 key 包一层
            // key_block:参数依赖变化 → 重建 snippet 块(粒度比 Svelte 粗一档,
            // 语义一致;纯静态参数的 key 无依赖,永不重算,零开销)。
            // 参数需 Clone + PartialEq(v0 文档化约束)。
            Node::Render { call, .. } => {
                let expr = self.expr(call, scope, false)?;
                let syn::Expr::Call(call_expr) = expr else {
                    return Err(CompileError::at_offset(
                        self.source,
                        call.offset,
                        "{@render} 需要调用形式,如 {@render row(item)}",
                    ));
                };
                let func = &call_expr.func;
                // 引用普通变量的参数包 Clone::clone(&(..)),避免 Fn 闭包移出捕获
                let args: Vec<TokenStream> = call_expr
                    .args
                    .iter()
                    .map(|a| {
                        let ts = a.to_token_stream();
                        if idents_within(ts.clone(), &scope.plain).is_empty() {
                            ts
                        } else {
                            quote! { ::std::clone::Clone::clone(&(#a)) }
                        }
                    })
                    .collect();
                // key 闭包与 build 闭包各自带预克隆的块表达式,互不争抢捕获
                let key_body = if args.is_empty() {
                    quote! { () } // 无参 snippet:key 恒定,永不重建
                } else {
                    quote! { (#(#args),*,) }
                };
                let pre_key = preclones(&key_body, scope);
                let build_body = quote! { (#func)(&__doc, __parent #(, #args)*); };
                let pre_build = preclones(&build_body, scope);
                Ok(quote! {
                    ::sv_ui::key_block(
                        &__doc, __parent,
                        { #pre_key move || #key_body },
                        { #pre_build move |__doc, __parent| {
                            let __doc: ::sv_ui::Doc = __doc.clone();
                            #build_body
                        } },
                    );
                })
            }
            // {@debug a, b}:依赖变化即 Debug 打印
            Node::Debug { args, .. } => {
                let mut fmt = String::from("[debug]");
                let mut exprs = Vec::new();
                for a in args {
                    let label = a.src.replace('{', "{{").replace('}', "}}");
                    fmt.push_str(&format!(" {label} = {{:?}} ·"));
                    exprs.push(self.expr(a, scope, false)?);
                }
                let fmt = fmt.trim_end_matches(" ·").to_string();
                Ok(quote! {
                    ::sv_reactive::effect(move || {
                        ::std::println!(#fmt #(, #exprs)*);
                    });
                })
            }
            // {#await fut}{:then v}{:catch e}:pending → 完成/失败渲染
            Node::Await {
                fut,
                pending,
                then_pat,
                then_children,
                catch_pat,
                catch_children,
                ..
            } => {
                let fut_expr = self.expr(fut, scope, false)?;
                let pending_ts = self.emit_nodes(pending, scope)?;
                let pending_cl = self.rebuild_closure(pending_ts, scope);

                let bind_arm = |pat: &Option<String>, scope: &Scope| -> (Scope, TokenStream) {
                    let mut inner = scope.clone();
                    let binding = match pat {
                        Some(name) => {
                            inner.shadowed.insert(name.clone());
                            inner.plain.insert(name.clone());
                            let id = format_ident!("{name}");
                            quote! { let #id = ::std::clone::Clone::clone(__value); }
                        }
                        None => quote! { let _ = __value; },
                    };
                    (inner, binding)
                };
                let (then_scope, then_bind) = bind_arm(then_pat, scope);
                let then_ts = self.emit_nodes(then_children, &then_scope)?;
                let then_pre = preclones(&then_ts, scope);
                let then_cl = quote! {
                    move |__doc, __parent, __value| {
                        let __doc: ::sv_ui::Doc = __doc.clone();
                        #then_pre
                        #then_bind
                        #then_ts
                    }
                };
                if catch_children.is_empty() && catch_pat.is_none() {
                    Ok(quote! {
                        ::sv_ui::tasks::await_block(&__doc, __parent,
                            move || #fut_expr, #pending_cl, #then_cl);
                    })
                } else {
                    let (catch_scope, catch_bind) = bind_arm(catch_pat, scope);
                    let catch_ts = self.emit_nodes(catch_children, &catch_scope)?;
                    let catch_pre = preclones(&catch_ts, scope);
                    let catch_cl = quote! {
                        move |__doc, __parent, __value| {
                            let __doc: ::sv_ui::Doc = __doc.clone();
                            #catch_pre
                            #catch_bind
                            #catch_ts
                        }
                    };
                    Ok(quote! {
                        ::sv_ui::tasks::await_block_result(&__doc, __parent,
                            move || #fut_expr, #pending_cl, #then_cl, #catch_cl);
                    })
                }
            }
            Node::Const { .. } | Node::Snippet { .. } => {
                unreachable!("Const/Snippet 在 emit_nodes 中处理")
            }
        }
    }

    fn emit_element(
        &mut self,
        tag: &Tag,
        attrs: &[Attr],
        children: &[Node],
        offset: usize,
        scope: &Scope,
    ) -> Result<TokenStream, CompileError> {
        if let Tag::Component(name) = tag {
            return self.emit_component(name, attrs, children, offset, scope);
        }
        if *tag == Tag::Overlay {
            return self.emit_overlay(attrs, children, offset, scope);
        }
        let el = self.fresh("el");
        let mut ts = match tag {
            Tag::View => emit::create(&el, ElemKind::View, ""),
            Tag::Checkbox => emit::create(&el, ElemKind::Checkbox, ""),
            Tag::Input | Tag::TextArea => emit::create(&el, ElemKind::TextInput, ""),
            Tag::Animation => emit::create(&el, ElemKind::Animation, ""),
            Tag::Overlay => unreachable!("overlay 在 emit_element 顶部拦截"),
            Tag::Text | Tag::Button => {
                let segments: &[Segment] = match children.first() {
                    Some(Node::Text { segments }) => segments,
                    _ => &[],
                };
                self.leaf_create(&el, tag, segments, scope)?
            }
            Tag::Component(_) => unreachable!(),
        };
        ts.extend(emit::append(&parent_ident(), &el));

        // ---- 静态样式源(优先级:class 类 < style=""/简写,后写覆盖) ----
        let mut style_setters = TokenStream::new();
        // 伪类变体收集(→ 内部状态 + 指针事件接线)
        let mut hover_static: Vec<TokenStream> = Vec::new();
        let mut hover_conds: Vec<(TokenStream, TokenStream)> = Vec::new();
        let mut active_static: Vec<TokenStream> = Vec::new();
        let mut focus_static: Vec<TokenStream> = Vec::new();
        // 元素类型规则打底(specificity 直觉:元素 < 类)
        let tag_name = match tag {
            Tag::View => "view",
            Tag::Text => "text",
            Tag::Button => "button",
            Tag::Checkbox => "checkbox",
            Tag::Input => "input",
            Tag::TextArea => "textarea",
            Tag::Animation => "animation",
            Tag::Overlay => unreachable!(),
            Tag::Component(_) => unreachable!(),
        };
        if let Some(entry) = self.sheet.elements.get(tag_name) {
            style_setters.extend(entry.base.clone());
            if let Some(h) = &entry.hover {
                hover_static.push(h.clone());
            }
            if let Some(a) = &entry.active {
                active_static.push(a.clone());
            }
            if let Some(f) = &entry.focus {
                focus_static.push(f.clone());
            }
        }
        for attr in attrs.iter().filter(|a| a.name == "class") {
            match &attr.value {
                AttrValue::Str { value, offset } => {
                    for cls in value.split_whitespace() {
                        let entry = self.sheet.classes.get(cls).ok_or_else(|| {
                            CompileError::at_offset(
                                self.source,
                                *offset,
                                format!("未知样式类 `.{cls}`(应在 <style> 块里定义)"),
                            )
                        })?;
                        style_setters.extend(entry.base.clone());
                        if let Some(h) = &entry.hover {
                            hover_static.push(h.clone());
                        }
                        if let Some(a) = &entry.active {
                            active_static.push(a.clone());
                        }
                        if let Some(f) = &entry.focus {
                            focus_static.push(f.clone());
                        }
                    }
                }
                AttrValue::Expr(_) => {
                    return Err(CompileError::at_offset(
                        self.source,
                        attr.offset,
                        "class 属性 v0 只支持静态字符串(条件类用 class:名字={cond})",
                    ));
                }
            }
        }
        for attr in attrs {
            match attr.name.as_str() {
                "class" | "checked" | "@attach" | "autofocus" | "placeholder" | "aria-label"
                | "rows" => {}
                // <animation> 专属:src(源素材路径,构建期 importer 用)/ loop / autoplay
                // / label(a11y)。模板层记录并建节点,素材注册是壳侧(register_*)的事。
                "src" | "loop" | "autoplay" | "label" if *tag == Tag::Animation => {}
                "value" if tag.is_text_input() => {}
                name if name == "onclick"
                    || name.starts_with("on")
                    || name.starts_with("style:")
                    || name.starts_with("class:")
                    || name.starts_with("transition:")
                    || name.starts_with("in:")
                    || name.starts_with("out:")
                    || name.starts_with("bind:") => {}
                "style" => match &attr.value {
                    AttrValue::Str { value, offset } => {
                        style_setters.extend(style::parse_style(self.source, value, *offset)?);
                    }
                    AttrValue::Expr(_) => {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "v0 的 style 属性只支持字符串;动态样式请用 style:字段={表达式} 指令",
                        ));
                    }
                },
                name => match &attr.value {
                    AttrValue::Str { value, offset } => {
                        let decl = format!("{name}:{value}");
                        let setters =
                            style::parse_style(self.source, &decl, *offset).map_err(|mut e| {
                                e.message = format!("属性 `{name}`:{}", e.message);
                                e
                            })?;
                        style_setters.extend(setters);
                    }
                    AttrValue::Expr(_) => {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            format!("未知属性 `{name}`"),
                        ));
                    }
                },
            }
        }

        // ---- class: 条件类 与 style: 指令 ----
        let mut class_conds: Vec<(TokenStream, TokenStream)> = Vec::new();
        let mut style_directives: Vec<TokenStream> = Vec::new();
        for attr in attrs {
            if let Some(cls) = attr.name.strip_prefix("class:") {
                let entry = self.sheet.classes.get(cls).cloned().ok_or_else(|| {
                    CompileError::at_offset(
                        self.source,
                        attr.offset,
                        format!("未知样式类 `.{cls}`(应在 <style> 块里定义)"),
                    )
                })?;
                let setters = entry.base.clone();
                let cond = match &attr.value {
                    AttrValue::Expr(e) => {
                        let x = self.expr(e, scope, false)?;
                        quote! { #x }
                    }
                    // 简写 class:muted → 条件即同名变量
                    AttrValue::Str { value, .. } if value.is_empty() => {
                        let e = ExprSrc {
                            src: cls.to_string(),
                            offset: attr.offset,
                        };
                        let x = self.expr(&e, scope, false)?;
                        quote! { #x }
                    }
                    AttrValue::Str { .. } => {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "class: 的值应为 {布尔表达式} 或省略(简写)",
                        ));
                    }
                };
                if let Some(h) = &entry.hover {
                    hover_conds.push((cond.clone(), h.clone()));
                }
                class_conds.push((cond, setters));
            } else if let Some(key) = attr.name.strip_prefix("style:") {
                let AttrValue::Expr(e) = &attr.value else {
                    return Err(CompileError::at_offset(
                        self.source,
                        attr.offset,
                        "style: 指令的值应为 {表达式}(静态值请用 style=\"...\")",
                    ));
                };
                let expr = self.expr(e, scope, false)?;
                let setter = style_directive_setter(key, &expr).ok_or_else(|| {
                    CompileError::at_offset(
                        self.source,
                        attr.offset,
                        format!("style: 不认识字段 `{key}`"),
                    )
                })?;
                style_directives.push(setter);
            }
        }

        let has_hover = !hover_static.is_empty() || !hover_conds.is_empty();
        let has_active = !active_static.is_empty();
        let has_focus = !focus_static.is_empty();
        let has_state = has_hover || has_active || has_focus;
        // 用户自己的指针回调(有 :hover 时与内部状态接线合成,避免互相覆盖)
        let user_enter = attrs.iter().find(|a| a.name == "onpointerenter");
        let user_leave = attrs.iter().find(|a| a.name == "onpointerleave");
        // 焦点回调同理:sv-ui 只有一个 focus_change 槽,`:focus` 与
        // onfocus/onblur 必须合成一次设入
        let user_focus_attr = attrs.iter().find(|a| a.name == "onfocus");
        let user_blur_attr = attrs.iter().find(|a| a.name == "onblur");
        let focus_expr = |attr: Option<&Attr>| -> Result<Option<TokenStream>, CompileError> {
            let Some(a) = attr else { return Ok(None) };
            let AttrValue::Expr(e) = &a.value else {
                return Err(CompileError::at_offset(
                    self.source,
                    a.offset,
                    "事件处理器应为 {闭包表达式}",
                ));
            };
            Ok(Some(self.expr(e, scope, true)?.to_token_stream()))
        };
        let (user_focus_expr, user_blur_expr) = if has_focus {
            (focus_expr(user_focus_attr)?, focus_expr(user_blur_attr)?)
        } else {
            (None, None)
        };
        if class_conds.is_empty() && !has_state {
            if !style_setters.is_empty() {
                // 静态样式:创建时设置一次,零 effect
                ts.extend(quote! { __doc.update_style(#el, |s| { #style_setters }); });
            }
            for setter in &style_directives {
                ts.extend(quote! {
                    ::sv_ui::bind_style_patch(&__doc, #el, move |s| { #setter });
                });
            }
        } else {
            // 条件类 / :hover:该元素样式整体交给一个响应式重算闭包
            // (优先级:类 < style="" < 条件类 < :hover < style: 指令)
            let arms = class_conds.iter().map(|(c, s)| quote! { if #c { #s } });
            let hover_arms = hover_conds
                .iter()
                .map(|(c, h)| quote! { if #c && __hv.get() { #h } });
            let hover_block = if has_hover {
                quote! { if __hv.get() { #(#hover_static)* } #(#hover_arms)* }
            } else {
                TokenStream::new()
            };
            // 声明序按 CSS 惯例 L-V-F-H-A::focus 垫底,悬停/按压可以盖它
            let focus_block = if has_focus {
                quote! { if __fc.get() { #(#focus_static)* } }
            } else {
                TokenStream::new()
            };
            // :active 排在 :hover 后(CSS 惯例:LVHA 声明序,按压态最终生效)
            let active_block = if has_active {
                quote! { if __ac.get() { #(#active_static)* } }
            } else {
                TokenStream::new()
            };
            let wiring = if has_hover {
                let enter_bind = match user_enter {
                    Some(a) => {
                        let AttrValue::Expr(e) = &a.value else {
                            return Err(CompileError::at_offset(
                                self.source,
                                a.offset,
                                "事件处理器应为 {闭包表达式}",
                            ));
                        };
                        let x = self.expr(e, scope, true)?;
                        quote! { let __ue = #x; }
                    }
                    None => quote! { let __ue = || {}; },
                };
                let leave_bind = match user_leave {
                    Some(a) => {
                        let AttrValue::Expr(e) = &a.value else {
                            return Err(CompileError::at_offset(
                                self.source,
                                a.offset,
                                "事件处理器应为 {闭包表达式}",
                            ));
                        };
                        let x = self.expr(e, scope, true)?;
                        quote! { let __ul = #x; }
                    }
                    None => quote! { let __ul = || {}; },
                };
                quote! {
                    #enter_bind
                    #leave_bind
                    __doc.set_on_pointer_enter(#el, move || { __hv.set(true); __ue(); });
                    __doc.set_on_pointer_leave(#el, move || { __hv.set(false); __ul(); });
                }
            } else {
                TokenStream::new()
            };
            let hv_decl = if has_hover {
                quote! { let __hv = ::sv_reactive::state(false); }
            } else {
                TokenStream::new()
            };
            let ac_decl = if has_active {
                quote! { let __ac = ::sv_reactive::state(false); }
            } else {
                TokenStream::new()
            };
            let fc_decl = if has_focus {
                quote! { let __fc = ::sv_reactive::state(false); }
            } else {
                TokenStream::new()
            };
            // `:focus` 要能触发,元素必须可获焦 —— 与 onkeydown 自动设位同理
            // (floem 教训:不自动设位,样式永远不生效,而且查不出原因)
            let fc_wiring = if has_focus {
                let f = user_focus_expr.clone();
                let b = user_blur_expr.clone();
                let change = emit::focus_change(&el, f, b, Some(quote! { __fc }));
                quote! {
                    __doc.set_focusable(#el, true);
                    #change
                }
            } else {
                TokenStream::new()
            };
            let ac_wiring = if has_active {
                quote! {
                    __doc.set_on_pointer_down(#el, move || __ac.set(true));
                    __doc.set_on_pointer_up(#el, move || __ac.set(false));
                }
            } else {
                TokenStream::new()
            };
            ts.extend(quote! {
                {
                    #hv_decl
                    #ac_decl
                    #fc_decl
                    ::sv_ui::bind_style(&__doc, #el, move |s| {
                        #style_setters
                        #(#arms)*
                        #focus_block
                        #hover_block
                        #active_block
                        #(#style_directives)*
                    });
                    #wiring
                    #ac_wiring
                    #fc_wiring
                }
            });
        }

        // ---- 焦点回调(onfocus/onblur 合成进单一 set_on_focus_change,
        //      与 :hover 的 __ue/__ul 合成同款,避免互相覆盖)----
        let user_focus = attrs.iter().find(|a| a.name == "onfocus");
        let user_blur = attrs.iter().find(|a| a.name == "onblur");
        if !has_focus && (user_focus.is_some() || user_blur.is_some()) {
            // 词汇表按"有没有用户闭包"收参(缺席的一侧它补空闭包);
            // `.svelte` 侧的表达式先过 runes 改写
            let handler = |attr: Option<&Attr>| -> Result<Option<TokenStream>, CompileError> {
                let Some(a) = attr else { return Ok(None) };
                let AttrValue::Expr(e) = &a.value else {
                    return Err(CompileError::at_offset(
                        self.source,
                        a.offset,
                        "事件处理器应为 {闭包表达式}",
                    ));
                };
                Ok(Some(self.expr(e, scope, true)?.to_token_stream()))
            };
            let f = handler(user_focus)?;
            let b = handler(user_blur)?;
            ts.extend(emit::focus_change(&el, f, b, None));
        }

        // ---- 事件 / 绑定 / 附着 / 过渡 ----
        let mut autofocus = false;
        let mut rows: u16 = 3;
        let mut bind_scrolly: Option<TokenStream> = None;
        for attr in attrs {
            match attr.name.as_str() {
                // Svelte 5 事件属性形态(遗留 on: 指令已移除,拒绝分支在下面统一指路)
                "onclick" => match &attr.value {
                    AttrValue::Expr(e) => {
                        let handler = self.expr(e, scope, true)?;
                        ts.extend(emit::on_click(&el, handler.to_token_stream()));
                    }
                    AttrValue::Str { .. } => {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "事件处理器应为 {闭包表达式}",
                        ));
                    }
                },
                // onkeydown:挂键盘回调并自动设 focusable(floem 教训:
                // 不自动设位,回调永远收不到事件是新手第一坑)
                // onkeydown/onkeyup 在下面合成一次设入(共用一个槽位)
                "onkeydown" | "onkeyup" => match &attr.value {
                    AttrValue::Expr(_) => {}
                    AttrValue::Str { .. } => {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "事件处理器应为 {闭包表达式}(签名 |e: &KeyEvent|)",
                        ));
                    }
                },
                // 已在上方合成进 set_on_focus_change
                "onfocus" | "onblur" => {}
                // autofocus 布尔属性:建树末尾聚焦(多个时文档序最后者胜)
                "autofocus" => autofocus = true,
                "onpointerenter" | "onpointerleave" if has_hover => {}
                "onpointerenter" | "onpointerleave" => match &attr.value {
                    AttrValue::Expr(e) => {
                        let handler = self.expr(e, scope, true)?;
                        let setter = if attr.name == "onpointerenter" {
                            quote! { set_on_pointer_enter }
                        } else {
                            quote! { set_on_pointer_leave }
                        };
                        ts.extend(quote! { __doc.#setter(#el, #handler); });
                    }
                    AttrValue::Str { .. } => {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "事件处理器应为 {闭包表达式}",
                        ));
                    }
                },
                // {@attach fn}:挂载时以 (doc, 节点) 调用,依赖变化时重跑
                "@attach" => {
                    let AttrValue::Expr(e) = &attr.value else {
                        unreachable!()
                    };
                    let expr = self.expr(e, scope, true)?;
                    ts.extend(quote! {
                        {
                            let __a_doc = __doc.clone();
                            let __a_el = #el;
                            ::sv_reactive::effect(move || { (#expr)(&__a_doc, __a_el); });
                        }
                    });
                }
                // aria-label:无障碍名称覆盖(调研 24 §4.1;任意元素可用)
                "aria-label" => match &attr.value {
                    AttrValue::Str { value, .. } => {
                        ts.extend(emit::aria_label(&el, quote! { #value }, false));
                    }
                    AttrValue::Expr(e) => {
                        let expr = self.expr(e, scope, false)?;
                        ts.extend(emit::aria_label(&el, expr.to_token_stream(), true));
                    }
                },
                // <textarea> 专属:可见行数(布局高 = rows × 行高)
                "rows" => {
                    if *tag != Tag::TextArea {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "rows 只能用在 <textarea> 上",
                        ));
                    }
                    let AttrValue::Str { value, .. } = &attr.value else {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "rows 的值应为静态数字,如 rows=\"5\"",
                        ));
                    };
                    let n: u16 = value.trim().parse().map_err(|_| {
                        CompileError::at_offset(
                            self.source,
                            attr.offset,
                            format!("rows 应为 1..=65535 的整数,收到 `{value}`"),
                        )
                    })?;
                    rows = n.max(1);
                }
                // <input> 专属:placeholder / value 单向 / bind:value 双向 /
                // oninput / onsubmit(调研 21 §2.7,复刻 bind:checked 模板)
                "placeholder" => {
                    if !tag.is_text_input() {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "placeholder 只能用在 <input>/<textarea> 上",
                        ));
                    }
                    let AttrValue::Str { value, .. } = &attr.value else {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "placeholder v0 只支持静态字符串",
                        ));
                    };
                    ts.extend(emit::placeholder(&el, quote! { #value }));
                }
                "value" if tag.is_text_input() => {
                    let AttrValue::Expr(e) = &attr.value else {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "value 的值应为 {表达式}(静态初值也用 {\"...\"})",
                        ));
                    };
                    let expr = self.expr(e, scope, false)?;
                    ts.extend(quote! {
                        {
                            let __v_doc = __doc.clone();
                            let __v_el = #el;
                            ::sv_reactive::effect(move || { __v_doc.set_input_value(__v_el, &(#expr)); });
                        }
                    });
                }
                "bind:value" => {
                    if !tag.is_text_input() {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "bind:value 只能用在 <input> 上",
                        ));
                    }
                    let AttrValue::Expr(e) = &attr.value else {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "bind:value 的值应为 {反应式变量}",
                        ));
                    };
                    let sig_ts = if let Ok(syn::Expr::Path(p)) =
                        crate::sourcemap::parse_str_at::<syn::Expr>(&e.src, e.offset)
                        && p.path.segments.len() == 1
                        && self
                            .script
                            .vars
                            .contains(&p.path.segments[0].ident.to_string())
                    {
                        let id = p.path.segments[0].ident.clone();
                        quote! { #id }
                    } else {
                        let x = self.expr(e, scope, false)?;
                        quote! { #x }
                    };
                    ts.extend(emit::bind_value(&el, sig_ts));
                }
                "oninput" | "onsubmit" => {
                    if !tag.is_text_input() {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            format!("{} 只能用在 <input> 上", attr.name),
                        ));
                    }
                    let AttrValue::Expr(e) = &attr.value else {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "事件处理器应为 {闭包表达式}(签名 |值: &str|)",
                        ));
                    };
                    let handler = self.expr(e, scope, true)?.to_token_stream();
                    ts.extend(if attr.name == "oninput" {
                        emit::on_input(&el, handler)
                    } else {
                        emit::on_submit(&el, handler)
                    });
                }
                // onscroll:滚动偏移变化回调(签名 Fn(f32, f32),新 (x, y))
                "onscroll" => {
                    let AttrValue::Expr(e) = &attr.value else {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "事件处理器应为 {闭包表达式}(签名 |x: f32, y: f32|)",
                        ));
                    };
                    let handler = self.expr(e, scope, true)?;
                    ts.extend(emit::on_scroll(&el, handler.to_token_stream()));
                }
                // bind:scrolly:Signal<f32> ↔ 纵向滚动偏移双向桥(调研 22)。
                // 延后到事件循环末尾发射:桥会链式保留既有 on_scroll,
                // 与 onscroll 共存时二者都生效
                "bind:scrolly" => {
                    let AttrValue::Expr(e) = &attr.value else {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "bind:scrolly 的值应为 {反应式变量}",
                        ));
                    };
                    let sig_ts = if let Ok(syn::Expr::Path(p)) =
                        crate::sourcemap::parse_str_at::<syn::Expr>(&e.src, e.offset)
                        && p.path.segments.len() == 1
                        && self
                            .script
                            .vars
                            .contains(&p.path.segments[0].ident.to_string())
                    {
                        let id = p.path.segments[0].ident.clone();
                        quote! { #id }
                    } else {
                        let x = self.expr(e, scope, false)?;
                        quote! { #x }
                    };
                    bind_scrolly = Some(sig_ts);
                }
                // bind:checked:<checkbox> 双向绑定
                "bind:checked" => {
                    if *tag != Tag::Checkbox {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "bind:checked 只能用在 <checkbox> 上",
                        ));
                    }
                    let AttrValue::Expr(e) = &attr.value else {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "bind:checked 的值应为 {反应式变量}",
                        ));
                    };
                    let sig_ts = if let Ok(syn::Expr::Path(p)) =
                        crate::sourcemap::parse_str_at::<syn::Expr>(&e.src, e.offset)
                        && p.path.segments.len() == 1
                        && self
                            .script
                            .vars
                            .contains(&p.path.segments[0].ident.to_string())
                    {
                        let id = p.path.segments[0].ident.clone();
                        quote! { #id }
                    } else {
                        let x = self.expr(e, scope, false)?;
                        quote! { #x }
                    };
                    ts.extend(quote! {
                        {
                            let __b_sig = #sig_ts;
                            let __b_doc = __doc.clone();
                            let __b_el = #el;
                            ::sv_reactive::effect(move || { __b_doc.set_checked(__b_el, __b_sig.get()); });
                            __doc.set_on_click(#el, move || __b_sig.update(|__v| *__v = !*__v));
                        }
                    });
                }
                // checked={bool}:单向
                "checked" => {
                    let AttrValue::Expr(e) = &attr.value else {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "checked 的值应为 {布尔表达式}",
                        ));
                    };
                    let expr = self.expr(e, scope, false)?;
                    ts.extend(quote! {
                        {
                            let __c_doc = __doc.clone();
                            let __c_el = #el;
                            ::sv_reactive::effect(move || { __c_doc.set_checked(__c_el, #expr); });
                        }
                    });
                }
                name if name.starts_with("bind:") => {
                    return Err(CompileError::at_offset(
                        self.source,
                        attr.offset,
                        format!(
                            "v0 的元素绑定支持 bind:checked/bind:value/bind:scrolly;`{name}` 需要对应控件/布局测量(见 SVELTE-SUPPORT)"
                        ),
                    ));
                }
                // 进场过渡(out: 需要 INERT 延迟销毁,推迟)
                name if name.starts_with("transition:") || name.starts_with("in:") => {
                    let anim = name.split(':').nth(1).unwrap_or("");
                    if anim != "fade" {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            format!("v0 过渡只支持 fade,收到 `{anim}`"),
                        ));
                    }
                    let dur = match &attr.value {
                        AttrValue::Expr(e) => {
                            let x = self.expr(e, scope, false)?;
                            quote! { #x }
                        }
                        AttrValue::Str { value, .. } if value.trim().is_empty() => {
                            quote! { 200u32 }
                        }
                        AttrValue::Str { value, offset } => {
                            let n: u32 = value.trim().parse().map_err(|_| {
                                CompileError::at_offset(
                                    self.source,
                                    *offset,
                                    format!("过渡时长 `{value}` 不是毫秒数"),
                                )
                            })?;
                            quote! { #n }
                        }
                    };
                    ts.extend(quote! { ::sv_ui::anim::transition_in_fade(&__doc, #el, #dur); });
                }
                name if name.starts_with("out:") => {
                    return Err(CompileError::at_offset(
                        self.source,
                        attr.offset,
                        "out: 出场过渡需要 INERT 延迟销毁机制,已推迟(见 SVELTE-SUPPORT)",
                    ));
                }
                name if name.starts_with("on")
                    && !name.starts_with("on:")
                    && !matches!(
                        name,
                        "onclick"
                            | "onpointerenter"
                            | "onpointerleave"
                            | "onkeydown"
                            | "onfocus"
                            | "onblur"
                            | "oninput"
                            | "onsubmit"
                            | "onscroll"
                    ) =>
                {
                    return Err(CompileError::at_offset(
                        self.source,
                        attr.offset,
                        format!(
                            "v0 事件支持 onclick/onpointerenter/onpointerleave/onkeydown/onfocus/onblur/oninput/onsubmit,收到 `{name}`"
                        ),
                    ));
                }
                name if name.starts_with("on:") => {
                    // SVELTE-SUPPORT 裁决:on: 指令已移除(对齐 Svelte 5),事件只有属性形态
                    let hint = match name {
                        "on:click" => "点击事件用属性形态 onclick={|| ...}",
                        "on:keydown" => "键盘事件用属性形态 onkeydown={|e| ...}",
                        "on:focus" => "焦点事件用属性形态 onfocus={...}",
                        "on:blur" => "焦点事件用属性形态 onblur={...}",
                        _ => "on: 指令已移除,事件用属性形态(如 onclick={...})",
                    };
                    return Err(CompileError::at_offset(
                        self.source,
                        attr.offset,
                        format!("不支持 `{name}`:{hint}"),
                    ));
                }
                _ => {}
            }
        }
        // onkeydown/onkeyup 合成一次设入(sv-ui 只有一个 on_key 槽位,
        // 分开设会互相顶掉;自动设 focusable 也在 key_handlers 里)
        let key_expr = |name: &str| -> Result<Option<TokenStream>, CompileError> {
            let Some(a) = attrs.iter().find(|a| a.name == name) else {
                return Ok(None);
            };
            let AttrValue::Expr(e) = &a.value else {
                return Ok(None); // 上面已对字符串形态报过错
            };
            Ok(Some(self.expr(e, scope, true)?.to_token_stream()))
        };
        ts.extend(emit::key_handlers(
            &el,
            key_expr("onkeydown")?,
            key_expr("onkeyup")?,
        ));

        // 多行开关放在属性之后:rows 已定,一次调用定型
        if *tag == Tag::TextArea {
            ts.extend(quote! { __doc.set_multiline(#el, true, #rows); });
        }
        if let Some(sig) = bind_scrolly {
            ts.extend(emit::bind_scroll_y(&el, sig));
        }
        if autofocus {
            ts.extend(quote! { __doc.focus(#el); });
        }

        if *tag == Tag::View && !children.is_empty() {
            let children_ts = self.emit_nodes(children, scope)?;
            ts.extend(quote! { { let __parent = #el; #children_ts } });
        }
        Ok(ts)
    }

    /// `<TodoItem label={x} bind:value={count}>children...</TodoItem>`
    /// → `todo_item(&__doc, __parent[, TodoItemProps { ... }])`
    fn emit_component(
        &mut self,
        tag_name: &str,
        attrs: &[Attr],
        children: &[Node],
        offset: usize,
        scope: &Scope,
    ) -> Result<TokenStream, CompileError> {
        let fn_name = crate::sanitize_fn_name(tag_name);
        let fn_ident = format_ident!("{fn_name}");
        let Some(sig) = self.registry.get(&fn_name) else {
            return Err(CompileError::at_offset(
                self.source,
                offset,
                format!("未知组件 `<{tag_name}>`(没有找到对应的 .svelte 文件)"),
            ));
        };
        // 未声明 $props 的组件:不带 props 参数;声明了(哪怕空)就带——
        // caller/callee 的函数签名契约由"是否声明"唯一决定
        let Some(fields) = &sig.fields else {
            if let Some(attr) = attrs.first() {
                return Err(CompileError::at_offset(
                    self.source,
                    attr.offset,
                    format!(
                        "组件 `<{tag_name}>` 没有声明 $props,不接受 prop `{}`",
                        attr.name
                    ),
                ));
            }
            if !children.is_empty() {
                return Err(CompileError::at_offset(
                    self.source,
                    offset,
                    format!("组件 `<{tag_name}>` 没有声明 children,不能传子内容"),
                ));
            }
            return Ok(quote! { #fn_ident(&__doc, __parent); });
        };

        // `bind:xxx` 归一化成 prop 名 xxx(仅 $bindable 字段可用)
        let norm_name = |attr: &Attr| -> String {
            attr.name
                .strip_prefix("bind:")
                .unwrap_or(&attr.name)
                .to_string()
        };
        for attr in attrs {
            let name = norm_name(attr);
            let Some(field) = fields.iter().find(|f| f.name == name) else {
                let known: Vec<_> = fields.iter().map(|f| f.name.as_str()).collect();
                return Err(CompileError::at_offset(
                    self.source,
                    attr.offset,
                    format!(
                        "组件 `<{tag_name}>` 没有 prop `{name}`(声明了:{})",
                        known.join(", ")
                    ),
                ));
            };
            if attr.name.starts_with("bind:") && !field.bindable {
                return Err(CompileError::at_offset(
                    self.source,
                    attr.offset,
                    format!("prop `{name}` 不是 $bindable,不能用 bind: 语法"),
                ));
            }
        }
        if !children.is_empty() && !fields.iter().any(|f| f.name == "children") {
            return Err(CompileError::at_offset(
                self.source,
                offset,
                format!(
                    "组件 `<{tag_name}>` 传了子内容,但其 $props 没有声明 `children: sv_ui::Snippet`"
                ),
            ));
        }

        let props_ty = format_ident!("{}Props", pascal(&fn_name));
        let mut inits = Vec::new();
        for field in fields {
            let fid = format_ident!("{}", field.name);
            let attr = attrs.iter().find(|a| norm_name(a) == field.name);
            // children:模板子内容编译成隐式 snippet
            if field.name == "children" && !children.is_empty() {
                if attr.is_some() {
                    return Err(CompileError::at_offset(
                        self.source,
                        offset,
                        "children 不能同时用属性和子内容两种方式传递",
                    ));
                }
                let children_ts = self.emit_nodes(children, scope)?;
                let pre_capture = preclones(&children_ts, scope);
                let pre_call = preclones(&children_ts, scope);
                inits.push(quote! {
                    #fid: {
                        #pre_capture
                        ::std::rc::Rc::new(move |__doc: &::sv_ui::Doc, __parent: ::sv_ui::ViewId| {
                            let __doc: ::sv_ui::Doc = __doc.clone();
                            #pre_call
                            #children_ts
                        }) as ::sv_ui::Snippet
                    }
                });
                continue;
            }
            let value = match attr {
                Some(attr) => match &attr.value {
                    AttrValue::Expr(e) => {
                        // 零参 snippet 名作为 prop:自动包成 sv_ui::Snippet
                        if let Ok(syn::Expr::Path(p)) =
                            crate::sourcemap::parse_str_at::<syn::Expr>(&e.src, e.offset)
                            && p.path.segments.len() == 1
                            && scope
                                .snippets
                                .contains(&p.path.segments[0].ident.to_string())
                        {
                            let id = p.path.segments[0].ident.clone();
                            inits.push(quote! {
                                #fid: ::std::rc::Rc::new(::std::clone::Clone::clone(&#id))
                                    as ::sv_ui::Snippet
                            });
                            continue;
                        }
                        // $bindable + 裸反应式变量名:直接传句柄(双向绑定零胶水)
                        if field.bindable
                            && let Ok(syn::Expr::Path(p)) =
                                crate::sourcemap::parse_str_at::<syn::Expr>(&e.src, e.offset)
                            && p.qself.is_none()
                            && p.path.segments.len() == 1
                            && self
                                .script
                                .vars
                                .contains(&p.path.segments[0].ident.to_string())
                        {
                            let ident = &p.path.segments[0].ident;
                            quote! { #ident }
                        } else {
                            let expr = self.expr(e, scope, true)?;
                            quote! { #expr }
                        }
                    }
                    AttrValue::Str { value, .. } => {
                        quote! { ::std::convert::Into::into(#value) }
                    }
                },
                None => {
                    if field.has_default {
                        let default_fn = format_ident!("default_{}", field.name);
                        quote! { #props_ty::#default_fn() }
                    } else {
                        return Err(CompileError::at_offset(
                            self.source,
                            offset,
                            format!("组件 `<{tag_name}>` 缺少必填 prop `{}`", field.name),
                        ));
                    }
                }
            };
            inits.push(quote! { #fid: #value });
        }
        Ok(quote! { #fn_ident(&__doc, __parent, #props_ty { #(#inits),* }); })
    }

    /// 叶子创建:全静态 → 直接带文本创建(零绑定);含插值 → 空文本 + bind_text
    fn leaf_create(
        &mut self,
        el: &syn::Ident,
        tag: &Tag,
        segments: &[Segment],
        scope: &Scope,
    ) -> Result<TokenStream, CompileError> {
        let kind = match tag {
            Tag::Button => ElemKind::Button,
            _ => ElemKind::Text,
        };
        // 模板段 → 共享词汇表的段(表达式先过 runes 改写,这是 .svelte 独有的一步)
        let mut parts = Vec::with_capacity(segments.len());
        for seg in segments {
            parts.push(match seg {
                Segment::Static(t) => TextPart::Lit(t.clone()),
                Segment::Expr(e) => TextPart::Expr(self.expr(e, scope, false)?.to_token_stream()),
            });
        }
        if let Some(label) = emit::static_text(&parts) {
            return Ok(emit::create(el, kind, &label));
        }
        let mut ts = emit::create(el, kind, "");
        ts.extend(emit::bind_text(el, &parts));
        Ok(ts)
    }

    /// `<overlay open={..} anchor="below" gap="4" modal close="outside"
    /// ondismiss={..} style="..">children</overlay>`(调研 25 O6)。
    /// 锚定到**父容器元素**(触发钮与 overlay 包在同一 view 即得下拉形态);
    /// children 编译成 overlay_block 的 build 闭包
    fn emit_overlay(
        &mut self,
        attrs: &[Attr],
        children: &[Node],
        offset: usize,
        scope: &Scope,
    ) -> Result<TokenStream, CompileError> {
        let mut open_ts = None;
        let mut side = "below".to_string();
        let mut gap = 4.0f32;
        let mut modal = false;
        let mut close: Option<String> = None;
        let mut dismiss_ts = quote! { None };
        let mut style_setters = TokenStream::new();
        // 弹层根的无障碍名称(对话框/菜单靠它被读屏播报;真机 C 复核发现缺它)。
        // 目标是弹层根 `__parent`(build 闭包里),故在 body 内发射。
        let mut aria_ts = TokenStream::new();
        for attr in attrs {
            match attr.name.as_str() {
                "open" => {
                    let AttrValue::Expr(e) = &attr.value else {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "open 的值应为 {布尔表达式}",
                        ));
                    };
                    open_ts = Some(self.expr(e, scope, false)?);
                }
                "anchor" => {
                    let AttrValue::Str { value, offset } = &attr.value else {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "anchor 应为静态字符串",
                        ));
                    };
                    match value.as_str() {
                        "below" | "above" | "left" | "right" | "center" => side = value.clone(),
                        other => {
                            return Err(CompileError::at_offset(
                                self.source,
                                *offset,
                                format!("anchor 支持 below/above/left/right/center,收到 `{other}`"),
                            ));
                        }
                    }
                }
                "gap" => {
                    let AttrValue::Str { value, offset } = &attr.value else {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "gap 应为数字",
                        ));
                    };
                    gap = value.trim().parse().map_err(|_| {
                        CompileError::at_offset(
                            self.source,
                            *offset,
                            format!("gap `{value}` 不是数字"),
                        )
                    })?;
                }
                "modal" => modal = true,
                "close" => {
                    let AttrValue::Str { value, offset } = &attr.value else {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "close 应为静态字符串",
                        ));
                    };
                    match value.as_str() {
                        "outside" | "any" | "none" => close = Some(value.clone()),
                        other => {
                            return Err(CompileError::at_offset(
                                self.source,
                                *offset,
                                format!("close 支持 outside/any/none,收到 `{other}`"),
                            ));
                        }
                    }
                }
                "ondismiss" => {
                    let AttrValue::Expr(e) = &attr.value else {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "ondismiss 应为 {闭包表达式}",
                        ));
                    };
                    let h = self.expr(e, scope, true)?;
                    dismiss_ts = quote! { Some(::std::rc::Rc::new(#h)) };
                }
                "style" => {
                    let AttrValue::Str { value, offset } = &attr.value else {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "overlay 的 style 只支持静态字符串",
                        ));
                    };
                    style_setters.extend(style::parse_style(self.source, value, *offset)?);
                }
                // 无障碍名称:给对话框/菜单一个读屏能播报的标题(静态串或 {表达式})。
                // **非反应式 + 借用**:标题在弹层生命周期内基本不变,且弹层根 `__parent`
                // 与正文里的 `{title}` 会争用同一个普通变量——用 `&(…).to_string()` 只
                // momentary 借用(不 move),正文闭包随后照常取用。反应式 `move` 闭包会
                // 永久捕获它,和正文冲突(真机 C 复核时踩到)。
                "aria-label" => {
                    let val = match &attr.value {
                        AttrValue::Str { value, .. } => quote! { #value },
                        AttrValue::Expr(e) => self.expr(e, scope, false)?.to_token_stream(),
                    };
                    aria_ts = quote! {
                        __doc.set_accessible_label(__parent, &(#val).to_string());
                    };
                }
                other => {
                    return Err(CompileError::at_offset(
                        self.source,
                        attr.offset,
                        format!(
                            "overlay 支持 open/anchor/gap/modal/close/ondismiss/style/aria-label,收到 `{other}`"
                        ),
                    ));
                }
            }
        }
        let Some(open) = open_ts else {
            return Err(CompileError::at_offset(
                self.source,
                offset,
                "overlay 缺少必填属性 open={布尔表达式}",
            ));
        };
        // 缺省关闭策略:普通弹层点外关;modal 只能程序关
        let close_str = close.unwrap_or_else(|| {
            if modal {
                "none".to_string()
            } else {
                "outside".to_string()
            }
        });
        let close_ts = match close_str.as_str() {
            "outside" => quote! { ::sv_ui::CloseBehavior::OnClickOutside },
            "any" => quote! { ::sv_ui::CloseBehavior::OnAnyClick },
            _ => quote! { ::sv_ui::CloseBehavior::None },
        };
        let anchor_ts = match side.as_str() {
            "center" => quote! { ::sv_ui::Anchor::WindowCenter },
            s => {
                let side_ident = match s {
                    "below" => quote! { Below },
                    "above" => quote! { Above },
                    "left" => quote! { Left },
                    _ => quote! { Right },
                };
                quote! {
                    ::sv_ui::Anchor::Node {
                        id: __anchor_el,
                        side: ::sv_ui::Side::#side_ident,
                        gap: #gap,
                    }
                }
            }
        };
        let children_ts = self.emit_nodes(children, scope)?;
        let body = quote! {
            __doc.update_style(__parent, |s| { #style_setters });
            #aria_ts
            #children_ts
        };
        let build = self.rebuild_closure(body, scope);
        Ok(quote! {
            {
                let __anchor_el = __parent;
                ::sv_ui::overlay_block(
                    &__doc,
                    move || #open,
                    move || #anchor_ts,
                    ::sv_ui::OverlayOpts {
                        layer: ::sv_ui::OverlayLayer::Popup,
                        modal: #modal,
                        close: #close_ts,
                        on_dismiss: #dismiss_ts,
                    },
                    #build,
                );
            }
        })
    }

    fn emit_if(
        &mut self,
        arms: &[Arm],
        else_children: &[Node],
        scope: &Scope,
    ) -> Result<TokenStream, CompileError> {
        let arm = &arms[0];
        let cond = self.expr(&arm.cond, scope, false)?;
        let then_ts = self.emit_nodes(&arm.children, scope)?;
        let else_ts = if arms.len() > 1 {
            self.emit_if(&arms[1..], else_children, scope)?
        } else {
            self.emit_nodes(else_children, scope)?
        };
        let then_closure = self.rebuild_closure(then_ts, scope);
        let else_closure = self.rebuild_closure(else_ts, scope);
        Ok(emit::if_block(
            &parent_ident(),
            cond.to_token_stream(),
            then_closure,
            else_closure,
        ))
    }

    fn emit_each(
        &mut self,
        parts: EachParts<'_>,
        scope: &Scope,
    ) -> Result<TokenStream, CompileError> {
        let EachParts {
            list,
            pat: pat_src,
            pat_offset,
            index,
            key,
            children,
            else_children,
            offset,
        } = parts;
        let list_expr = self.value_closure_expr(list, scope)?;
        let pat = crate::sourcemap::parse_with_at(syn::Pat::parse_single, pat_src, pat_offset)
            .map_err(|e| {
                CompileError::at_offset(
                    self.source,
                    pat_offset,
                    format!("{{#each}} 模式解析失败: {e}"),
                )
            })?;
        let mut inner_scope = scope.clone();
        let mut pat_binds = HashSet::new();
        collect_pat_idents(&pat, &mut pat_binds);
        inner_scope.shadowed.extend(pat_binds.iter().cloned());
        inner_scope.plain.extend(pat_binds.iter().cloned());

        // keyed:按 key 复用行作用域
        if let Some(key_src) = key {
            if !else_children.is_empty() {
                return Err(CompileError::at_offset(
                    self.source,
                    offset,
                    "keyed {#each} 暂不支持 {:else}(可外包一层 {#if list.is_empty()})",
                ));
            }
            let mut key_expr = self.parse_expr(key_src)?;
            script::rewrite_template_expr(
                &self.script.vars,
                &inner_scope.locals,
                &inner_scope.shadowed,
                &mut key_expr,
                false,
            )
            .map_err(|msg| CompileError::at_offset(self.source, key_src.offset, msg))?;
            // keyed 行拿的是 `Signal<T>`(ADR-7):内容变化原地更新而不是重建。
            // 于是绑定名在**行内**是反应式的 —— 与 {@const} 同一套改写
            // (`item.field` → `item.get().field`),故必须是单个标识符
            let syn::Pat::Ident(pat_ident) = &pat else {
                return Err(CompileError::at_offset(
                    self.source,
                    pat_offset,
                    "keyed {#each} 的绑定必须是单个标识符(行内它是 Signal,                     解构请在行内用 {@const})",
                ));
            };
            let bind_id = pat_ident.ident.clone();
            let mut row_scope = inner_scope.clone();
            let name = bind_id.to_string();
            row_scope.shadowed.remove(&name);
            row_scope.plain.remove(&name);
            row_scope.locals.insert(name);
            let children_ts = self.emit_nodes(children, &row_scope)?;
            let outer_pre = preclones(&children_ts, scope);
            Ok(quote! {
                ::sv_ui::each_block_keyed(
                    &__doc, __parent,
                    move || #list_expr,
                    // key 闭包拿的仍是 &T(裸值),绑定名在这里不是反应式
                    |__item| { let #pat = ::std::clone::Clone::clone(__item); #key_expr },
                    move |__doc, __parent, __item| {
                        let __doc: ::sv_ui::Doc = __doc.clone();
                        #outer_pre
                        let #bind_id = __item;
                        #children_ts
                    },
                );
            })
        } else {
            let idx_binding = match index {
                Some(name) => {
                    let id = format_ident!("{name}");
                    inner_scope.shadowed.insert(name.to_string());
                    inner_scope.plain.insert(name.to_string());
                    quote! { let #id = __index; }
                }
                None => quote! { let _ = __index; },
            };
            let children_ts = self.emit_nodes(children, &inner_scope)?;
            // 行闭包被逐行反复调用:开头对外层普通变量做每次调用的预克隆
            let outer_pre = preclones(&children_ts, scope);
            let row = quote! {
                move |__doc, __parent, __item, __index| {
                    let __doc: ::sv_ui::Doc = __doc.clone();
                    #outer_pre
                    let #pat = ::std::clone::Clone::clone(__item);
                    #idx_binding
                    #children_ts
                }
            };
            if else_children.is_empty() {
                Ok(emit::each_block(
                    &parent_ident(),
                    list_expr.to_token_stream(),
                    row,
                ))
            } else {
                let empty_ts = self.emit_nodes(else_children, scope)?;
                let empty = self.rebuild_closure(empty_ts, scope);
                Ok(quote! {
                    ::sv_ui::each_block_else(&__doc, __parent, move || #list_expr, #row, #empty);
                })
            }
        }
    }

    /// if/key/each-空态的重建闭包:`Fn` 会被反复调用,体内先对普通变量做
    /// 每次调用的预克隆,内层 move 闭包拿克隆、环境保原值
    fn rebuild_closure(&self, body: TokenStream, scope: &Scope) -> TokenStream {
        // 闭包协议在共享词汇表里;`.svelte` 独有的普通变量预克隆作为 prelude 注入
        let pre = preclones(&body, scope);
        emit::rebuild_closure(body, pre)
    }
}

/// 生成代码里的父节点变量名(共享词汇表按 Ident 收参,这里固定一个)
fn parent_ident() -> syn::Ident {
    format_ident!("__parent")
}

/// emit_each 的参数包(字段太多,聚合传递)
struct EachParts<'a> {
    list: &'a ExprSrc,
    pat: &'a str,
    pat_offset: usize,
    index: Option<&'a str>,
    key: Option<&'a ExprSrc>,
    children: &'a [Node],
    else_children: &'a [Node],
    offset: usize,
}

/// `style:字段` → Style 字段赋值(值是任意 Rust 表达式,类型由字段决定)
fn style_directive_setter(key: &str, expr: &syn::Expr) -> Option<TokenStream> {
    Some(match key {
        "padding" => quote! { s.padding = ::sv_ui::Edges::all(#expr); },
        "margin" => quote! { s.margin = ::sv_ui::Edges::all(#expr); },
        "gap" => quote! { s.gap = #expr; },
        "font-size" | "font_size" => quote! { s.font_size = #expr; },
        "radius" | "corner-radius" => quote! { s.corner_radius = #expr; },
        "opacity" => quote! { s.opacity = #expr; },
        "width" => quote! { s.width = Some(#expr); },
        "height" => quote! { s.height = Some(#expr); },
        "direction" => quote! { s.direction = #expr; },
        "bg" => quote! { s.bg = Some(#expr); },
        "fg" | "color" => quote! { s.fg = Some(#expr); },
        _ => return None,
    })
}
