# sv-ui

> retained 场景树 + 细粒度绑定原语 — [svelte-rs](https://github.com/Cyaim/svelte-rs) 的一个 crate。

桌面版的 "DOM":节点树 + `bind_text` / `bind_style` / `if_block` / `each_block` / `key_block` 等绑定原语,外加焦点链、文本编辑内核、滚动、弹层、动画与后台任务桥。**没有 VDOM,不做 diff**——模板编译器(`sv-macro` / `sv-compiler`)的编译目标就是这里。

单独使用意义不大:整套栈的入口、示例与中英双语指南都在
[仓库根目录](https://github.com/Cyaim/svelte-rs)(`docs/README.md` 是导航)。
架构分层与 ADR 决策记录见 `docs/DESIGN.md`。

---

**EN** — The retained scene tree (a desktop "DOM") plus the fine-grained binding primitives that both template front-ends compile into. No VDOM, no diffing.
This crate is part of the [svelte-rs](https://github.com/Cyaim/svelte-rs) workspace;
start from the repository root for guides (bilingual) and runnable examples.

## 许可 / License

MIT OR Apache-2.0
