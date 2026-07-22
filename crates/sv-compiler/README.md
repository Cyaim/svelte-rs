# sv-compiler

> .sv 单文件组件编译器 — [svelte-rs](https://github.com/Cyaim/svelte-rs) 的一个 crate。

script 块 runes 源变换(裸 `count` → `.get()`、`count += 1` → `.update`)+ 100% Svelte 模板语法 → Rust 代码生成,经 build.rs / OUT_DIR 集成。

单独使用意义不大:整套栈的入口、示例与中英双语指南都在
[仓库根目录](https://github.com/Cyaim/svelte-rs)(`docs/README.md` 是导航)。
架构分层与 ADR 决策记录见 `docs/DESIGN.md`。

---

**EN** — The `.sv` single-file-component compiler: runes source transform over the script block plus Svelte template syntax, emitting Rust through a build.rs / OUT_DIR integration.
This crate is part of the [svelte-rs](https://github.com/Cyaim/svelte-rs) workspace;
start from the repository root for guides (bilingual) and runnable examples.

## 许可 / License

MIT OR Apache-2.0
