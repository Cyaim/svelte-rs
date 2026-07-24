//! # svelte-rs
//!
//! Svelte 风格的 Rust 跨平台桌面 UI 库(Win/Linux/macOS/鸿蒙)——**伞 crate**。
//!
//! 生态按职责拆成若干 `sv-*` 子 crate(响应式内核 / 场景树 / 宏前端 / 渲染壳 /
//! `.svelte` 编译器)。本 crate 把它们按名 re-export,作为**统一命名入口**
//! (子模块名与子 crate 一一对应,tokio / tokio-* 同款分层)。
//!
//! ## 依赖声明(重要:不是"一条依赖拿全套")
//!
//! `view!` 宏与 `.svelte` 编译器生成的代码用**绝对路径** `::sv_ui::` / `::sv_reactive::`
//! 发射对场景树/响应式的调用 —— 这按 extern prelude 解析,伞 crate 的 re-export
//! **救不了**它(只依赖 `svelte-rs` 写 `view!`/`.svelte` 会 `E0433: could not find
//! sv_ui`)。所以用 UI 前端时,**必须把 `sv-ui` 与 `sv-reactive` 也列为直接依赖**;
//! `.svelte` 单文件组件另需 `sv-compiler` 作 **build-dependency**(build.rs 里
//! `sv_compiler::build("src")`)。典型 `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! sv-reactive = "0.1"
//! sv-ui = "0.1"
//! sv-shell = "0.1"      # 开窗渲染;瘦身/纯响应式可不要
//! # sv-macro = "0.1"    # 用 view! 宏时
//!
//! [build-dependencies]
//! sv-compiler = "0.1"   # 用 .svelte 单文件组件时
//! ```
//!
//! 伞 crate `svelte-rs` 便于一处拿到统一路径(`svelte_rs::ui`、`svelte_rs::reactive`
//! …)与 prelude,但**不替代**上面的直接依赖。示例见 `examples/`(均直依子 crate)。

// 子 crate 按名 re-export —— 与子 crate 一一对应,永不漂移。
pub use sv_compiler as compiler;
pub use sv_macro as macros;
pub use sv_reactive as reactive;
pub use sv_shell as shell;
pub use sv_ui as ui;

// 顶层直达最常用的项(免去逐个 `svelte_rs::reactive::state`)。
pub use sv_macro::{Store, view};
pub use sv_reactive::{Derived, Signal, batch, derived, effect, state, untrack};
pub use sv_shell::run_app;
pub use sv_ui::Doc;

/// 常用项一次导入:`use svelte_rs::prelude::*;`。
///
/// 收敛面刻意小——只放"写一个组件几乎一定会用到"的:runes 三件套 + `view!` +
/// `Doc`。更专门的项(布局样式、事件、弹层……)从 [`ui`] / [`shell`] 子模块取,
/// 免得 prelude 变成"全都导进来"的垃圾桶。
pub mod prelude {
    pub use sv_macro::{Store, view};
    pub use sv_reactive::{batch, derived, effect, state, untrack};
    pub use sv_ui::Doc;
}

#[cfg(test)]
mod tests {
    /// 伞 crate 的价值就是"一个入口能拿到全套"——这条编译过就证明了
    /// 各子 crate 的 re-export 路径都通(名字漂移会当场编译失败)。
    #[test]
    fn reexports_resolve() {
        // 路径解析即证明;不实际建运行时对象(那要窗口/GPU)。
        let _ = std::any::type_name::<crate::Signal<i32>>();
        let _ = std::any::type_name::<crate::Doc>();
        // 子模块路径存在
        fn _assert_paths() {
            #[allow(unused_imports)]
            use crate::{compiler, macros, reactive, shell, ui};
        }
    }
}
