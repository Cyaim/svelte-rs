# svelte-rs 开发指南

Svelte 风格 Rust 跨平台桌面 UI 库(Win/Linux/macOS/鸿蒙)的探索原型。

## 必读

- `docs/DESIGN.md` — 架构分层、ADR 决策记录、路线图、风险清单。改架构前先读它。
- `docs/plans/open-issues.md` — **未了结问题登记**(已知缺口、未查明的回归、
  以及反复踩到的几类方法论错误)。动手前扫一眼,免得重踩。
- `docs/research/` — 29 份调研报告(2026-07,1–27 联网核实、28–29 仓库实证),DESIGN.md 的依据。
  商用路线图见 DESIGN.md §5(R1–R5 分期,落地方案 = 调研 20–25;
  调研 26 = sv-arco 生态探索;27 = 商用距离复盘;28 = 无组件库可用性实证)。
- `docs/en/` + `docs/zh-CN/` — 中英双语指南(互为镜像,改一边必须同步另一边);
  导航在 `docs/README.md` / `docs/README.zh-CN.md`。

## 常用命令

```sh
cargo test                              # 全部测试
cargo run -p counter                    # 开窗跑计数器
cargo run -p counter -- --png out.png   # 离屏渲染一帧(验证渲染,无需窗口)
```

构建产物默认在仓库内 `./target`。若检出目录在 OneDrive 等同步盘内,建议按
`.cargo/config.toml` 里的注释取消 `target-dir` 那行、把产物移出同步目录
(同步器锁文件会导致 Windows 链接失败/增量构建损坏)——**该行默认是注释掉的**。

## 架构速记

数据流:`state/derived`(sv-reactive)→ effect 精准改场景树(sv-ui)→ 版本号 bump
→ `on_mutate` → 重绘(sv-shell)。**没有 VDOM/diff**。模板有两个前端、**一个内核**
(ADR-2 M1 已完成):`view!` 宏(sv-macro)与 `.svelte` 单文件组件(sv-compiler,
runes 源变换 + build.rs 集成,示例 examples/counter-sfc)——两者只剩各自 parser,
汇入公共模板 IR(`sv_compiler::template`)与同一份 codegen。

约束:
- 响应式是单线程模型(thread-local runtime,句柄 `Copy + !Send`)。
- derived 计算中禁止写 state(会 panic,对应 Svelte state_unsafe_mutation)。
- 编译内核只有一份:IR 表达式载荷是**双态**的(`.svelte` 文本+偏移 / 宏 token
  带真 span 直通)。改绑定原语签名只改 `sv_compiler::emit` 一处 + 形状测试;
  改 codegen 在 `sv-compiler/src/codegen.rs` 一处(两前端同时生效,`.svelte`
  golden 逐字节 + 宏行为/span 测试都会盯着)。属性名表仍各 parser 自有。
- **宏路径的表达式不过任何改写**(runes/force-move/预克隆都是 `.svelte` 语义)
  ——`ExprSrc::Tokens` 分支必须保持原样直通,span 精度有契约测试守护。
- 布局已迁 taffy 0.12(封在 sv-shell layout_tree 内,`Vec<Placed>` 契约);
  文本栈已迁 Parley 0.11 + fontique(封在 sv-shell text.rs 门面,全仓唯一
  parley import;fallback 混排/折行/对齐/光标与选区几何——**线性路径与
  font.rs 已退役**)。渲染 CPU/vello 双后端(Painter 抽象)。
- 开窗路径是**帧对齐**的(ADR-6):写 signal 只入队 + 催一帧,effect 在
  帧前统一冲刷;要立刻看到结果调 `sv_reactive::tick()`。离屏/测试路径不受影响。
