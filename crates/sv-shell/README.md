# sv-shell

> 桌面渲染壳:窗口 + 渲染器 — [svelte-rs](https://github.com/Cyaim/svelte-rs) 的一个 crate。

winit 窗口 + 可切换 `Painter` 后端(CPU:softbuffer/tiny-skia;GPU:vello,`backend-vello` feature)+ taffy 布局 + Parley 文本栈 + AccessKit 无障碍 + IME/剪贴板接线。也提供离屏渲染(`render_to_png`),CI 里不用开窗就能验证渲染。

单独使用意义不大:整套栈的入口、示例与中英双语指南都在
[仓库根目录](https://github.com/Cyaim/svelte-rs)(`docs/README.md` 是导航)。
架构分层与 ADR 决策记录见 `docs/DESIGN.md`。

---

**EN** — The desktop shell: winit windowing, a switchable `Painter` backend (CPU tiny-skia / GPU vello), taffy layout, the Parley text stack, AccessKit, IME and clipboard wiring, plus offscreen rendering for CI.
This crate is part of the [svelte-rs](https://github.com/Cyaim/svelte-rs) workspace;
start from the repository root for guides (bilingual) and runnable examples.

## 许可 / License

MIT OR Apache-2.0
