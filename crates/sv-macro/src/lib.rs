//! # sv-macro
//!
//! `view!` 过程宏:把 Svelte 风味的声明式模板编译成对 sv-ui 细粒度绑定原语
//! (`bind_text` / `bind_style` / `if_block` / `each_block`)的命令式调用。
//! 零 diff——模板在编译期展开,运行时哪个值变了就精准更新哪个节点。
//!
//! 编译器主体是独立逻辑,本文件只是薄壳:
//! - `parse`:手写递归下降解析(`syn::parse::ParseStream` → IR)
//! - `ir`:模板中间表示
//! - `codegen`:IR → `TokenStream`

use proc_macro::TokenStream;

mod codegen;
mod ir;
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
    let input = syn::parse_macro_input!(input as ir::ViewInput);
    codegen::generate(&input).into()
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
