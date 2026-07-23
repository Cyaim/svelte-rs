# svelte-rs

Svelte 风格的 Rust 跨平台桌面 UI 库(Win/Linux/macOS/鸿蒙)——**伞 crate**,
一个入口 re-export 整个 `sv-*` 生态。

```toml
[dependencies]
svelte-rs = "0.0.1"
```

```rust
use svelte_rs::prelude::*;

let count = state(0i32);
let double = derived(move || count.get() * 2);
effect(move || println!("{}", double.get()));
count.set(1); // 精准触发,无 VDOM、无 diff
```

## 子模块 ↔ 子 crate

| `svelte_rs::` | 子 crate | 职责 |
|---|---|---|
| `reactive` | `sv-reactive` | runes 响应式内核(state/derived/effect) |
| `ui` | `sv-ui` | retained 场景树 + 绑定原语 + 焦点/输入/弹层 |
| `macros` | `sv-macro` | `view!` 宏、`#[derive(Store)]` |
| `shell` | `sv-shell` | winit 窗口 + CPU/vello 渲染 + 输入/无障碍/动画 |
| `compiler` | `sv-compiler` | `.svelte` 单文件组件编译器(build.rs 用) |

顶层还直达最常用的:`state`/`derived`/`effect`/`batch`/`untrack`/`view!`/`Doc`/`run_app`。

**想瘦依赖**(比如只用响应式内核)可以直接依赖对应子 crate,不必经本 crate。
子 crate 名保持 `sv-*`(短、crates.io 空闲);本 crate 只是聚合入口(tokio / tokio-* 同款分层)。

双许可:MIT OR Apache-2.0。
