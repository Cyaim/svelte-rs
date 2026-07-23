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

## 日志 / Logging

壳层的诊断(GPU 回退、丢帧、vello 失败、draw_image 丢图……)统一走
[`log`](https://docs.rs/log) **门面**——`log::warn!` / `log::error!`。

**库不绑后端,后端由你的应用自选**,这就是"轻松更换日志库":换后端不用碰 sv-shell。
不装后端时 `log` 是零成本(宏体不求值)。常见接法:

```rust
fn main() {
    env_logger::init();                 // 或任何 log 后端
    // 想要 tracing 的高性能结构化:tracing-subscriber + tracing-log 桥即可,
    // 同样不碰 sv-shell —— 只在你的 main 里换一行。
    svelte_rs::run_app(/* … */);
}
```

`RUST_LOG=warn` 起手即可看到壳层诊断。

## 许可 / License

MIT OR Apache-2.0
