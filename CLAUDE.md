# svelte-rs 开发指南

Svelte 风格 Rust 跨平台桌面 UI 库(Win/Linux/macOS/鸿蒙)的探索原型。

## 必读

- `docs/DESIGN.md` — 架构分层、ADR 决策记录、路线图、风险清单。改架构前先读它。
- `docs/research/` — 26 份联网核实的调研报告(2026-07),DESIGN.md 的依据。
  商用路线图见 DESIGN.md §5(R1–R5 分期,落地方案 = 调研 20–25;
  调研 26 = sv-arco 组件库生态探索)。
- `docs/en/` + `docs/zh-CN/` — 中英双语指南(互为镜像,改一边必须同步另一边);
  导航在 `docs/README.md` / `docs/README.zh-CN.md`。

## 常用命令

```sh
cargo test                              # 全部测试
cargo run -p counter                    # 开窗跑计数器
cargo run -p counter -- --png out.png   # 离屏渲染一帧(验证渲染,无需窗口)
```

构建产物在 `C:/cargo-target/svelte-rs`(见 `.cargo/config.toml`,仓库在 OneDrive
内,target 不能放同步目录)。

## 架构速记

数据流:`state/derived`(sv-reactive)→ effect 精准改场景树(sv-ui)→ 版本号 bump
→ `on_mutate` → 重绘(sv-shell)。**没有 VDOM/diff**。模板有两个前端(ADR-2 修订版:
双前端共存,M1 合并内核):`view!` 宏(sv-macro)与 `.sv` 单文件组件(sv-compiler,
runes 源变换 + build.rs 集成,示例 examples/counter-sfc)。

约束:
- 响应式是单线程模型(thread-local runtime,句柄 `Copy + !Send`)。
- derived 计算中禁止写 state(会 panic,对应 Svelte state_unsafe_mutation)。
- sv-ui 是宏的编译目标:改绑定原语签名要同步改 sv-macro codegen 与其测试。
- 布局已迁 taffy 0.12(封在 sv-shell layout_tree 内,`Vec<Placed>` 契约);
  折行为 swash + unicode-linebreak 过渡(计划内报废);文本栈 Parley 在 R3。
  渲染 CPU/vello 双后端(Painter 抽象)。
