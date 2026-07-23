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

## 脏矩形与 scroll-blit / Damage & scroll-blit

CPU 呈现路径默认启用**局部重画**:滚动帧把上一帧像素按位移搬一段、只重画
新露出的条与滚动条列(scroll-blit);打字/勾选/换色/焦点/光标闪烁只重画对应
矩形(脏日志 `DirtyItem::Paint { id }` 定位)。任何吃不准的形态(弹层开着、
矢量动画、结构变更、小数位移、视口内有外来绘制)自动降级为多画,方向永远
是"多画不错画";与全量渲染**逐字节相同**由 `blit_render_matches_full_render_*`
差分测试守着。实测(release)3000 控件滚动场景:离屏 12.9 → 2.2ms/帧(5.9×),
开窗 55 → ~100fps。

环境变量:

| 变量 | 含义 |
|---|---|
| `SV_DAMAGE=0` | 关闭脏矩形/scroll-blit,恒整帧重画(怀疑画错时的一键排除法) |
| `SV_SHOW_FPS=1` | 连续重绘 + 每 30 帧打印帧率 |
| `SV_RENDERER=cpu\|vello` | 呈现后端(vello 走自己的场景缓存,不经此路径) |

离屏(`render_frame` / `--png`)与 vello 后端不走损伤路径,行为不变。
后续项:softbuffer `present_with_damage`(目前 present 仍整窗转换拷贝)。

## 许可 / License

MIT OR Apache-2.0
