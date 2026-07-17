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
