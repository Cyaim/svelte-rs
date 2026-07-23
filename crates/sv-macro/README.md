# sv-macro

> `view!` 过程宏前端 — [svelte-rs](https://github.com/Cyaim/svelte-rs) 的一个 crate。

把 Svelte 风味模板编译成对 `sv-ui` 的命令式建树 + 绑定调用,零运行时比对。与 `.svelte` 单文件组件(`sv-compiler`)共享同一编译目标。

单独使用意义不大:整套栈的入口、示例与中英双语指南都在
[仓库根目录](https://github.com/Cyaim/svelte-rs)(`docs/README.md` 是导航)。
架构分层与 ADR 决策记录见 `docs/DESIGN.md`。

---

**EN** — The `view!` proc-macro front-end: compiles a Svelte-flavoured template into imperative scene-tree construction plus binding calls against `sv-ui`.
This crate is part of the [svelte-rs](https://github.com/Cyaim/svelte-rs) workspace;
start from the repository root for guides (bilingual) and runnable examples.

## 许可 / License

MIT OR Apache-2.0
