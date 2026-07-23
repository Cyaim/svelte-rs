//! # sv-macro
//!
//! `view!` 过程宏:把 Svelte 风味的声明式模板编译成对 sv-ui 细粒度绑定原语
//! (`bind_text` / `bind_style` / `if_block` / `each_block`)的命令式调用。
//! 零 diff——模板在编译期展开,运行时哪个值变了就精准更新哪个节点。
//!
//! 本 crate 只剩 **parser 分叉**(ADR-2 内核合并):
//! - `parse`:手写递归下降(`syn::parse::ParseStream` → 共享模板 IR),
//!   表面语法校验全在这里,错误带真 span;
//! - IR 与 codegen 在 `sv_compiler::template` / `sv_compiler::generate_template`
//!   —— 与 `.svelte` 前端共享同一份内核,表达式以带 span 的 token 直通。

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;

mod parse;
mod store;

/// 把 Svelte 风味模板编译成 sv-ui 场景树构建 + 绑定代码,返回 `()`。
///
/// ```text
/// view! { doc表达式, 父节点表达式 =>
///     <view style(move |s| { ... }) >
///         "静态" {插值}
///         if cond { ... } else if cond2 { ... } else { ... }
///         for item, i in vec_expr { ... }
///         <button on_click(move || ...)>"label"</button>
///     </view>
/// }
/// ```
///
/// - `doc表达式` 求值为 `Doc` / `&Doc`,`父节点表达式` 求值为 `ViewId`;
/// - 仅支持 `<view>` / `<text>` / `<button>` 三种标签,支持自闭合 `<view />`;
/// - 连续的字符串字面量与 `{插值}` 合并为一个文本节点:全字面量则为静态文本,
///   含插值则编译成 `bind_text` 响应式绑定(插值表达式需实现 `Display`);
/// - `if` / `for` 编译成 `if_block` / `each_block`,分支/行内创建的状态与
///   绑定随块销毁自动回收。
#[proc_macro]
pub fn view(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as parse::ViewInput);
    // 共享内核发射(表面语法已在 parse 期校验完,这里失败只可能是内部缺陷,
    // 兜底转成宏调用点上的编译错误而不是 panic)
    let body = match sv_compiler::generate_template(&input.nodes) {
        Ok(ts) => ts,
        Err(e) => {
            return syn::Error::new(Span::call_site(), format!("view! 内部错误:{e}"))
                .to_compile_error()
                .into();
        }
    };
    let doc = &input.doc;
    let parent = &input.parent;
    // 空模板时抑制 unused 警告(正常模板中两者必然被使用)
    let silence = if input.nodes.is_empty() {
        quote! { let _ = (&__doc, __parent); }
    } else {
        proc_macro2::TokenStream::new()
    };
    quote! {{
        let __doc: ::sv_ui::Doc = (#doc).clone();
        let __parent: ::sv_ui::ViewId = #parent;
        #silence
        #body
    }}
    .into()
}

/// `#[derive(Store)]`:给结构体生成**字段级信号** store `XxxStore`
/// (ADR-1 里 Proxy 深层响应的替代品)。
///
/// `Signal<整个结构体>` 粒度太粗——改一个字段会把只读别的字段的 effect 一起
/// 叫醒。这个 derive 让每个字段各持一个 `Signal`:
///
/// ```ignore
/// #[derive(Store, Clone, PartialEq)]
/// struct Settings { theme: String, volume: f32 }
///
/// let s = Settings { theme: "dark".into(), volume: 0.8 }.into_store();
/// s.volume.set(0.5);                 // 只叫醒读 volume 的 effect
/// let snap: Settings = s.snapshot(); // 读全部字段(会订阅全部)
/// s.apply(next);                     // 整体写回,只写值变了的字段
/// ```
///
/// 要求:具名字段结构体、无泛型、字段类型 `Clone + PartialEq + 'static`。
/// **不做嵌套 store**:内层想更细就给内层也 derive 一次。
#[proc_macro_derive(Store)]
pub fn derive_store(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    match store::derive(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}
