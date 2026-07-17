//! IR + 变换后的 script → Rust 源码(prettyplease 格式化,人类可读)
//!
//! 所有权纪律(对抗审查后的定型设计):模板里引用的**普通变量**(props 解构、
//! script 普通 let、each 模式绑定)会被生成的 `move` 闭包捕获。为避免
//! use-after-move(E0382)与 `Fn` 闭包移出捕获(E0507),采取两层预克隆:
//! 1. **节点级**:每个节点的生成代码若引用普通变量,包一层
//!    `{ let x = Clone::clone(&x); ... }`——兄弟节点各拿各的克隆,原变量保活;
//! 2. **重建闭包体级**:if/each/key 的重建闭包每次调用开头重新克隆——闭包环境
//!    保有原值,反复调用各拿新克隆。
//! 代价:模板引用的普通变量需要 `Clone`(文档化约束)。

use std::collections::{HashMap, HashSet};

use proc_macro2::{TokenStream, TokenTree};
use quote::{ToTokens, format_ident, quote};
use syn::parse::Parser as _;

use crate::script::{self, ScriptOutput, collect_pat_idents};
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
}

pub fn generate(
    source: &str,
    fn_name: &str,
    script: &ScriptOutput,
    nodes: &[Node],
    registry: &PropsRegistry,
    classes: &HashMap<String, TokenStream>,
) -> Result<String, CompileError> {
    let mut root_scope = Scope::default();
    root_scope.plain.extend(script.plain_vars.iter().cloned());

    // $props → 组件签名:pub struct XxxProps(+ 默认值关联函数)+ 第三个参数 + 解构
    let (props_struct, props_param, props_destructure) = match &script.props {
        Some(decl) => {
            let props_ty = format_ident!("{}Props", pascal(fn_name));
            let fnames: Vec<_> = decl.fields.iter().map(|f| format_ident!("{}", f.name)).collect();
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

    let mut cg = Cg { source, script, registry, classes, n: 0 };
    let body = cg.emit_nodes(nodes, &root_scope)?;
    let script_stmts = &script.stmts;
    let fn_ident = format_ident!("{fn_name}");

    let file_ts = quote! {
        #props_struct
        #[allow(unused_variables, unused_mut, clippy::all)]
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
    Ok(format!(
        "// 由 sv-compiler 生成,请勿手改。\n{}",
        prettyplease::unparse(&file)
    ))
}

fn pascal(snake: &str) -> String {
    snake
        .split('_')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut c = s.chars();
            let head: String = c.next().into_iter().flat_map(|c| c.to_uppercase()).collect();
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
    /// `<style>` 块编译出的类 → 赋值语句流
    classes: &'a HashMap<String, TokenStream>,
    n: usize,
}

impl Cg<'_> {
    fn fresh(&mut self, prefix: &str) -> syn::Ident {
        self.n += 1;
        format_ident!("__{prefix}{}", self.n)
    }

    fn parse_expr(&self, e: &ExprSrc) -> Result<syn::Expr, CompileError> {
        syn::parse_str(&e.src).map_err(|err| {
            CompileError::at_offset(self.source, e.offset, format!("表达式解析失败: {err}"))
        })
    }

    /// 解析模板表达式并做 runes 读写改写
    fn expr(&self, e: &ExprSrc, scope: &Scope, force_move: bool) -> Result<syn::Expr, CompileError> {
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
    fn value_closure_expr(
        &self,
        e: &ExprSrc,
        scope: &Scope,
    ) -> Result<TokenStream, CompileError> {
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
                Node::Snippet { name, params, children, offset } => {
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
            Node::Element { tag, attrs, children, offset } => {
                self.emit_element(tag, attrs, children, *offset, scope)
            }
            Node::Text { segments } => {
                let el = self.fresh("t");
                let create = self.leaf_create(&el, &Tag::Text, segments, scope)?;
                Ok(quote! { #create __doc.append(__parent, #el); })
            }
            Node::If { arms, else_children, .. } => self.emit_if(arms, else_children, scope),
            Node::Each { list, pat, pat_offset, index, key, children, else_children, offset } => {
                self.emit_each(EachParts {
                    list,
                    pat: pat,
                    pat_offset: *pat_offset,
                    index: index.as_deref(),
                    key: key.as_ref(),
                    children,
                    else_children,
                    offset: *offset,
                }, scope)
            }
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
        let el = self.fresh("el");
        let mut ts = match tag {
            Tag::View => quote! { let #el = __doc.create_view(); },
            Tag::Text | Tag::Button => {
                let segments: &[Segment] = match children.first() {
                    Some(Node::Text { segments }) => segments,
                    _ => &[],
                };
                self.leaf_create(&el, tag, segments, scope)?
            }
            Tag::Component(_) => unreachable!(),
        };
        ts.extend(quote! { __doc.append(__parent, #el); });

        // 第一遍:静态样式。优先级(后写覆盖):class 类 < style=""/简写属性;
        // style: 指令(响应式 patch)在第二遍,永远最后生效——顺序确定
        let mut style_setters = TokenStream::new();
        for attr in attrs.iter().filter(|a| a.name == "class") {
            match &attr.value {
                AttrValue::Str { value, offset } => {
                    for cls in value.split_whitespace() {
                        let setters = self.classes.get(cls).ok_or_else(|| {
                            CompileError::at_offset(
                                self.source,
                                *offset,
                                format!("未知样式类 `.{cls}`(应在 <style> 块里定义)"),
                            )
                        })?;
                        style_setters.extend(setters.clone());
                    }
                }
                AttrValue::Expr(_) => {
                    return Err(CompileError::at_offset(
                        self.source,
                        attr.offset,
                        "class 属性 v0 只支持静态字符串(动态样式走 style: 指令)",
                    ));
                }
            }
        }
        for attr in attrs {
            match attr.name.as_str() {
                "class" => {}
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
                name if name == "onclick"
                    || name.starts_with("on")
                    || name.starts_with("style:") => {}
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
        if !style_setters.is_empty() {
            ts.extend(quote! { __doc.update_style(#el, |s| { #style_setters }); });
        }

        // 第二遍:事件与 style: 指令
        for attr in attrs {
            match attr.name.as_str() {
                // Svelte 5 事件属性 onclick={...} 与遗留 on:click={...} 都认
                "onclick" | "on:click" => match &attr.value {
                    AttrValue::Expr(e) => {
                        let handler = self.expr(e, scope, true)?;
                        ts.extend(quote! { __doc.set_on_click(#el, #handler); });
                    }
                    AttrValue::Str { .. } => {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "事件处理器应为 {闭包表达式}",
                        ));
                    }
                },
                name if name.starts_with("on") && !name.starts_with("on:") && name != "onclick" => {
                    return Err(CompileError::at_offset(
                        self.source,
                        attr.offset,
                        format!("v0 只支持 onclick,收到 `{name}`"),
                    ));
                }
                name if name.starts_with("on:") => {
                    if name != "on:click" {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            format!("v0 只支持 on:click,收到 `{name}`"),
                        ));
                    }
                }
                name if name.starts_with("style:") => {
                    let AttrValue::Expr(e) = &attr.value else {
                        return Err(CompileError::at_offset(
                            self.source,
                            attr.offset,
                            "style: 指令的值应为 {表达式}(静态值请用 style=\"...\")",
                        ));
                    };
                    let expr = self.expr(e, scope, false)?;
                    let setter = style_directive_setter(&name["style:".len()..], &expr)
                        .ok_or_else(|| {
                            CompileError::at_offset(
                                self.source,
                                attr.offset,
                                format!("style: 不认识字段 `{}`", &name["style:".len()..]),
                            )
                        })?;
                    ts.extend(quote! {
                        ::sv_ui::bind_style_patch(&__doc, #el, move |s| { #setter });
                    });
                }
                _ => {}
            }
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
                format!("未知组件 `<{tag_name}>`(没有找到对应的 .sv 文件)"),
            ));
        };
        // 未声明 $props 的组件:不带 props 参数;声明了(哪怕空)就带——
        // caller/callee 的函数签名契约由"是否声明"唯一决定
        let Some(fields) = &sig.fields else {
            if let Some(attr) = attrs.first() {
                return Err(CompileError::at_offset(
                    self.source,
                    attr.offset,
                    format!("组件 `<{tag_name}>` 没有声明 $props,不接受 prop `{}`", attr.name),
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
            attr.name.strip_prefix("bind:").unwrap_or(&attr.name).to_string()
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
                        // $bindable + 裸反应式变量名:直接传句柄(双向绑定零胶水)
                        if field.bindable
                            && let Ok(syn::Expr::Path(p)) = syn::parse_str::<syn::Expr>(&e.src)
                            && p.qself.is_none()
                            && p.path.segments.len() == 1
                            && self.script.vars.contains(&p.path.segments[0].ident.to_string())
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
        let create = |label: &str| match tag {
            Tag::Button => quote! { let #el = __doc.create_button(#label); },
            _ => quote! { let #el = __doc.create_text(#label); },
        };
        let all_static = segments.iter().all(|s| matches!(s, Segment::Static(_)));
        if all_static {
            let label: String = segments
                .iter()
                .map(|s| match s {
                    Segment::Static(t) => t.as_str(),
                    _ => unreachable!(),
                })
                .collect();
            return Ok(create(&label));
        }
        let mut pushes = TokenStream::new();
        for seg in segments {
            match seg {
                Segment::Static(t) if t.is_empty() => {}
                Segment::Static(t) => pushes.extend(quote! { __s.push_str(#t); }),
                Segment::Expr(e) => {
                    let expr = self.expr(e, scope, false)?;
                    pushes.extend(quote! { __s.push_str(&(#expr).to_string()); });
                }
            }
        }
        let mut ts = create("");
        ts.extend(quote! {
            ::sv_ui::bind_text(&__doc, #el, move || {
                let mut __s = ::std::string::String::new();
                #pushes
                __s
            });
        });
        Ok(ts)
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
        Ok(quote! {
            ::sv_ui::if_block(&__doc, __parent, move || #cond, #then_closure, #else_closure);
        })
    }

    fn emit_each(&mut self, parts: EachParts<'_>, scope: &Scope) -> Result<TokenStream, CompileError> {
        let EachParts { list, pat: pat_src, pat_offset, index, key, children, else_children, offset } =
            parts;
        let list_expr = self.value_closure_expr(list, scope)?;
        let pat = syn::Pat::parse_single.parse_str(pat_src).map_err(|e| {
            CompileError::at_offset(self.source, pat_offset, format!("{{#each}} 模式解析失败: {e}"))
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
            let children_ts = self.emit_nodes(children, &inner_scope)?;
            let outer_pre = preclones(&children_ts, scope);
            Ok(quote! {
                ::sv_ui::each_block_keyed(
                    &__doc, __parent,
                    move || #list_expr,
                    |__item| { let #pat = ::std::clone::Clone::clone(__item); #key_expr },
                    move |__doc, __parent, __item| {
                        let __doc: ::sv_ui::Doc = __doc.clone();
                        #outer_pre
                        let #pat = ::std::clone::Clone::clone(__item);
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
                Ok(quote! { ::sv_ui::each_block(&__doc, __parent, move || #list_expr, #row); })
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
        if body.is_empty() {
            return quote! { |_, _| {} };
        }
        let pre = preclones(&body, scope);
        quote! {
            move |__doc, __parent| {
                let __doc: ::sv_ui::Doc = __doc.clone();
                #pre
                #body
            }
        }
    }
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
        "padding" => quote! { s.padding = #expr; },
        "gap" => quote! { s.gap = #expr; },
        "font-size" | "font_size" => quote! { s.font_size = #expr; },
        "radius" | "corner-radius" => quote! { s.corner_radius = #expr; },
        "width" => quote! { s.width = Some(#expr); },
        "height" => quote! { s.height = Some(#expr); },
        "direction" => quote! { s.direction = #expr; },
        "bg" => quote! { s.bg = Some(#expr); },
        "fg" | "color" => quote! { s.fg = Some(#expr); },
        _ => return None,
    })
}
