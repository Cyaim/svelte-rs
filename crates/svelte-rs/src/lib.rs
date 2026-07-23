//! # svelte-rs
//!
//! Svelte 风格的 Rust 跨平台桌面 UI 库(Win/Linux/macOS/鸿蒙)——**伞 crate**。
//!
//! 生态按职责拆成若干 `sv-*` 子 crate(响应式内核 / 场景树 / 宏前端 / 渲染壳 /
//! `.svelte` 编译器)。本 crate 是**单一入口**:`cargo add svelte-rs` 一条依赖就
//! 拿到全套,子模块名与子 crate 一一对应(tokio / tokio-* 同款分层)。
//!
//! ```ignore
//! use svelte_rs::prelude::*;
//!
//! let count = state(0i32);
//! let double = derived(move || count.get() * 2);
//! effect(move || println!("{}", double.get()));
//! count.set(1); // 精准触发,无 diff
//! ```
//!
//! 想瘦依赖(比如只用响应式内核)可以直接依赖对应子 crate,不必经本 crate。

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
