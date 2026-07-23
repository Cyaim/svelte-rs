**中文** | [English](./README.md)

# svelte-rs 文档中心

指南为中英双语;参考资料(设计记录与调研报告)目前只有中文,按原样链接。

## 指南

| 中文 | English | 内容 |
|---|---|---|
| [快速上手](zh-CN/getting-started.md) | [Getting started](en/getting-started.md) | 安装、跑示例、仓库导览 |
| [架构](zh-CN/architecture.md) | [Architecture](en/architecture.md) | 分层、数据流、无 VDOM 设计 |
| [响应式](zh-CN/reactivity.md) | [Reactivity](en/reactivity.md) | `state`/`derived`/`effect`、context、异步桥 |
| [.sv 组件](zh-CN/sv-components.md) | [.sv components](en/sv-components.md) | 模板语法、props、build.rs 集成 |
| [样式](zh-CN/styling.md) | [Styling](en/styling.md) | 编译期 CSS 子集、`:hover`、变量 |
| [渲染后端](zh-CN/rendering-backends.md) | [Render backends](en/rendering-backends.md) | Painter trait、CPU/vello、环境开关 |
| [性能](zh-CN/performance.md) | [Performance](en/performance.md) | `virtual_list`、membench、实测数字 |

## 参考资料

- [DESIGN.md](DESIGN.md) — 架构设计与决策记录(ADR-1..10)。**改架构前先读它**。
- [SVELTE-SUPPORT.md](SVELTE-SUPPORT.md) — Svelte 5 语法/特性支持矩阵,77 项。
- [CSS-SUPPORT.md](CSS-SUPPORT.md) — 现代 CSS 差距矩阵,91 项。
- [plans/](plans/) — 工作计划,尤其是 [plans/open-issues.md](plans/open-issues.md):已知缺口的唯一登记处(CLAUDE.md 必读)。

## 调研报告

26 份联网核实的调研报告(2026-07),ADR 的依据:

| # | 报告 |
|---|---|
| 01 | [Svelte 5 编译模型与 Rust 映射](research/01-svelte-model.md) |
| 02 | [Rust GUI 生态全景与差异化](research/02-rust-gui-landscape.md) |
| 03 | [鸿蒙 Rust 自绘可行性](research/03-harmonyos.md) |
| 04 | [编译器策略(proc-macro vs 外部文件)](research/04-compiler-strategy.md) |
| 05 | [四平台渲染/文本/布局/无障碍选型](research/05-rendering-stack.md) |
| 06 | [.sv 构建集成机制(build.rs/OUT_DIR)](research/06-sv-build-integration.md) |
| 07 | [.sv 的 IDE/LSP 策略(Volar 式转发)](research/07-sv-ide-lsp.md) |
| 08 | [runes 源变换语义与健全性](research/08-sv-runes-transform.md) |
| 09 | [.sv 格式设计 + 热重载架构](research/09-sv-sfc-format-hotreload.md) |
| 10 | [双路线动手实证对比](research/10-route-comparison-hands-on.md) |
| 11 | [业界 CSS 策略光谱 + Rust 基建](research/11-css-industry-strategies.md) |
| 12 | [CSS 语义逐项映射设计](research/12-css-semantics-mapping.md) |
| 13 | [七类渲染后端逐一对比](research/13-render-backends.md) |
| 14 | [可切换 Painter 抽象](research/14-switchable-painter.md) |
| 15 | [三类场景现状分析](research/15-scenario-analysis.md) |
| 16 | [分场景内存基准测试](research/16-memory-benchmarks.md) |
| 17 | [分后端×分场景内存构成与帧率](research/17-backend-memory-fps.md) |
| 18 | [百万控件@144fps:swash 迁移 + 视口虚拟化](research/18-million-controls-144fps.md) |
| 19 | [距离可商用还有多远:四路审计与分档判决](research/19-commercialization-gap.md) |
| 20 | [键盘事件通道+焦点链+快捷键](research/20-keyboard-focus.md) |
| 21 | [文本输入+IME+剪贴板](research/21-text-input-ime-clipboard.md) |
| 22 | [滚动体系(裁剪/滚轮/滚动条/virtual_list 桥)](research/22-scroll-system.md) |
| 23 | [taffy 布局接入+文本换行](research/23-taffy-text-wrap.md) |
| 24 | [Parley 迁移+AccessKit](research/24-parley-accesskit.md) |
| 25 | [弹层体系+发布工程](research/25-overlay-release-engineering.md) |
| 26 | [arco.design 视觉标准组件库(sv-arco)可行性](research/26-arco-design-ui-kit.md) |

## 约定

- 指南互为镜像:`zh-CN/<page>.md` ↔ `en/<page>.md`,章节与事实一致。
- 每篇指南第一行是语言切换链接。
- 修改指南时应在同一次变更中同步两个语言版本。
