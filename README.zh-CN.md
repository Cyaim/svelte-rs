**中文** | [English](./README.md)

# svelte-rs

[![CI](https://github.com/Cyaim/svelte-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/Cyaim/svelte-rs/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#许可)

Svelte 风格的 Rust 跨平台桌面 UI 库 — **探索原型**。

核心思路:把 Svelte 5 的编译哲学搬到原生桌面。模板在编译期变成对 retained
场景树的**定点更新代码**,运行时没有虚拟 DOM、没有 diff、没有重建。
目标平台:Windows / Linux / macOS / 鸿蒙(HarmonyOS NEXT)。

```rust
let count = state(0);                          // $state
let double = derived(move || count.get() * 2); // $derived
effect(move || println!("{}", double.get()));  // $effect
count.set(1);                                  // 精准触发,无 diff
```

同一件事的 `.svelte` 单文件组件写法——原汁 Svelte 模板语法 + 真 Rust 表达式
(script 里的 `count += 1` 会被编译器自动改写成句柄操作):

```text
<script>
let count = $state(0i32);
let double = $derived(count * 2);
</script>

<text>Count: {count} · 双倍 = {double}</text>
<button style="bg:#ff3e00; fg:#fff" onclick={|| count += 1}>+1</button>
{#if count > 5}
  <text fg="#ff3e00">超过 5 了!</text>
{/if}
```

## 亮点

- **Rust 版 runes 内核** — `state` / `derived`(可写)/ `effect` / `batch` /
  `untrack` / context,push-pull 三态脏标记,effect 所有权树。
- **双编译前端、同一目标** — `view!` 宏与 `.svelte` 单文件组件编译器(build.rs 集成)
  产物都是对场景树绑定原语的调用。
- **真 CSS、编译期解决** — 真实 CSS 语法的封闭子集(`:hover`、`:root` 变量、
  嵌套、继承)在构建期全部求解,零运行时选择器引擎。
- **可切换渲染后端** — CPU(tiny-skia + swash)与 GPU(vello 0.9 / wgpu)共用一个
  `Painter` trait;按构建、按环境变量(`SV_RENDERER`)选择,探测失败自动回退。
- **百万控件规模** — 视口虚拟化(`virtual_list`)实测 100 万逻辑控件:
  p99 = 5.28ms、1% low = 174fps、工作集 28MB(CPU 后端,连续滚动最坏工况)。

## 快速开始

```sh
cargo test                    # 全部测试
cargo run -p showcase         # 特性橱窗(推荐先看)
cargo run -p counter          # 计数器(view! 宏路线)
cargo run -p counter-sfc      # 计数器(.svelte 编译器路线,UI 在 src/Counter.svelte)
cargo run -p showcase -- --png out.png  # 离屏渲染一帧(无需窗口)
```

> 检出目录在 OneDrive 等同步盘时,建议启用 `.cargo/config.toml` 里注释掉的
> `target-dir`,把构建产物移出同步目录。

## 文档

文档中心在 [`docs/`](docs/README.zh-CN.md),指南分
[中文](docs/zh-CN/getting-started.md)与[English](docs/en/getting-started.md)两语:

| 指南 | |
|---|---|
| [快速上手](docs/zh-CN/getting-started.md) | 安装、跑示例、仓库导览 |
| [架构](docs/zh-CN/architecture.md) | 分层、数据流、为什么没有 VDOM |
| [响应式](docs/zh-CN/reactivity.md) | runes 内核使用指南 |
| [.svelte 组件](docs/zh-CN/sv-components.md) | 模板语法、props、构建集成 |
| [样式](docs/zh-CN/styling.md) | 编译期 CSS 子集 |
| [渲染后端](docs/zh-CN/rendering-backends.md) | Painter trait、CPU/vello、开关 |
| [性能](docs/zh-CN/performance.md) | virtual_list、membench、实测数字 |

参考资料(中文):[架构设计与 ADR](docs/DESIGN.md) ·
[Svelte 支持矩阵(77 项)](docs/SVELTE-SUPPORT.md) ·
[现代 CSS 差距矩阵(91 项)](docs/CSS-SUPPORT.md) ·
[调研报告 ×27](docs/README.zh-CN.md#调研报告)

## 仓库结构

| 路径 | 说明 |
|---|---|
| `crates/sv-reactive` | runes 响应式内核 |
| `crates/sv-ui` | retained 场景树 + 细粒度绑定原语 |
| `crates/sv-macro` | `view!` 宏前端 |
| `crates/sv-compiler` | `.svelte` 单文件组件编译器前端(含 `sv check`) |
| `crates/sv-shell` | winit 窗口壳 + CPU/vello 渲染器 |
| `crates/sv-vap` · `sv-pag` · `sv-lottie` | 动画格式解析器(VAP / PAG / Lottie) |
| `crates/sv-lsp` | `.svelte` 语言服务器(LSP):实时编译诊断 |
| `crates/sv-arco-tokens` | Arco Design 设计令牌:色板算法移植 + `global.less` 转译(Rust 常量 + `:root` CSS) |
| `crates/sv-arco` | Arco 风格组件库(`.svelte` 组件;A1 静态件已落地:Button/Tag/Badge/Divider/Alert/Typography/Link) |
| `examples/` | showcase · counter(-sfc) · todo-sfc · settings-sfc · input-demo · overlay-demo · membench · vap-gift · arco-gallery |

## 现状

M0 探索已完成:signal 到像素的完整闭环(中文渲染、HiDPI、命中测试)、`.svelte`
编译器覆盖 Svelte 5 主要语法面(77 项矩阵 ✅43)、双渲染后端、百万控件虚拟化
实测达标。这是原型——API 会变,布局/文本整形/帧调度等子系统是占位实现,
替换计划见 [docs/DESIGN.md](docs/DESIGN.md) 的路线图与 ADR。

## 版本与发布

尚未发布到 crates.io。命名已定(见 [docs/DESIGN.md](docs/DESIGN.md)
的 ADR-10:伞 crate `svelte-rs`,子 crate 保持 `sv-*` 前缀)。首发后工作区
所有 crate 同版本号、按依赖序推送。

**0.x 政策:minor 号(`0.X.0`)= 破坏性变更,patch 号(`0.0.X`)= 向后兼容。**
每次破坏性变更都在 [CHANGELOG.md](CHANGELOG.md) 写明迁移方式。谈 1.0 之前
排期的三项破坏性变更已全部落地:双前端内核合并(ADR-2 M1)、`on:` 事件指令
移除(统一为 Svelte 5 的 `onclick={..}` 属性形态)、帧调度语义(ADR-6,写入
攒到帧边界)。距 1.0 还差 crates.io 首发与稳定期。

MSRV 为 **1.88**——由 let-chains(`if let ... && ...`)决定,而不是 edition 2024 的 1.85;CI 有专门一条构建道钉死。

## 许可

双许可:MIT OR Apache-2.0。
