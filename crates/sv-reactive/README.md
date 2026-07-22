# sv-reactive

> Svelte 5 runes 风格的细粒度响应式内核 — [svelte-rs](https://github.com/Cyaim/svelte-rs) 的一个 crate。

`state` / `derived` / `effect` 三件套:thread-local arena + `Copy` 句柄,push-pull 三态脏标记(菱形依赖天然 glitch-free),effect 构成所有权树。单线程模型(句柄 `!Send`),后台线程走消息回 UI 线程写 signal。

单独使用意义不大:整套栈的入口、示例与中英双语指南都在
[仓库根目录](https://github.com/Cyaim/svelte-rs)(`docs/README.md` 是导航)。
架构分层与 ADR 决策记录见 `docs/DESIGN.md`。

---

**EN** — Fine-grained reactivity kernel in the style of Svelte 5 runes: thread-local arena + `Copy` handles, push-pull tri-state dirty marking, effects owned in a tree. Single-threaded by design.
This crate is part of the [svelte-rs](https://github.com/Cyaim/svelte-rs) workspace;
start from the repository root for guides (bilingual) and runnable examples.

## 许可 / License

MIT OR Apache-2.0
