//! Arco Design 风格组件库(调研 26 的落地;A1 波次进行中)。
//!
//! 组件以 `.svelte` 单文件组件编写(`components/`),build.rs 注入
//! [`sv_arco_tokens`] 的 `:root` 令牌块后经 sv-compiler 编译,在这里
//! `include!` 并作为普通 Rust 函数导出:
//!
//! ```ignore
//! sv_arco::button(&doc, parent, sv_arco::ButtonProps {
//!     label: "主要按钮".into(),
//!     variant: "primary".into(),
//!     status: "default".into(),
//!     size: "default".into(),
//!     disabled: false,
//!     on_click: std::rc::Rc::new(|| println!("点了")),
//! });
//! ```
//!
//! **消费形态说明**:sv-compiler 的组件注册表是单构建目录扫描,跨 crate
//! 的 `.svelte` 里**不能**写 `<Button>` 标签引用本库组件 —— 对外交付就是
//! 上面的 Rust 函数 API(`view!` 宏与手写 Rust 都能调)。
//!
//! 视觉规范派生自 ByteDance Arco Design(MIT,见 `LICENSE-ARCO`);
//! 本 crate 为非官方实现,与 ByteDance 无关联、未获其背书。

// 生成产物:components/Button.svelte → pub fn button + pub struct ButtonProps
include!(concat!(env!("OUT_DIR"), "/button.rs"));

/// 设计令牌层的再导出(色板算法 + 全局令牌),免得消费方多记一个 crate 名。
pub use sv_arco_tokens as tokens;
