# 07 · .sv 文件的 IDE 体验:Volar 式虚拟文档转发 rust-analyzer 的可行性

> 调研日期:2026-07-17。关键事实已联网核实(Volar/Vue 3.x 架构现状、svelte-language-tools 现状、TypeScript 7/tsgo 事件、rust-analyzer 的 rust-project.json / discoverConfig / ra_ap 库化 / salsa 移植状态、slint-lsp、qmlls/cxx-qt、cargo JSON 诊断、tree-sitter injection)。个别推导性结论与训练数据来源已在文末单独标注。
>
> 前置背景:本项目正在评估"独立编译器路线"——`.sv` 单文件组件(script 块是 Rust + runes,template 块是 Svelte 语法),由自研编译器生成对 sv-ui retained 场景树的定点更新代码。本文回答:这条路线的 IDE 体验能不能靠"生成虚拟 .rs + 位置映射 + 转发 rust-analyzer"低成本获得,而不是像 Slint 那样自建全套 LSP。

---

## 0. 结论先行(TL;DR)

1. **可行,但不是 Volar 的 1:1 平移,而是"otter.nvim 模式 + svelte2tsx 的 codegen 纪律"**。核心不对称:Volar/svelte-language-tools 能工作,是因为 **TypeScript 是一个可 in-process 调用的库**(`ts.LanguageService`);而 **rust-analyzer 是一个进程/服务器,没有插件 API,库形态(ra_ap_*)不承诺稳定**。因此我们的接入点不是"把 r-a 当库嵌进 sv-language-server"(那是远期选项),而是 **LSP-to-LSP 转发**:sv 侧维护虚拟(实际上建议**落盘**的).rs 文档 + 双向位置映射,把请求转发给一个 rust-analyzer 实例。这个模式有活的先例:Neovim 的 otter.nvim、Emacs lsp-proxy 的 org-babel 支持、VS Code 官方 embedded-languages 指南的 request forwarding 方案。
2. **最重要的架构决策:虚拟 .rs 不虚拟——直接落盘,而且就是 build.rs 的构建产物本身**(同一个编译器库生成,`foo.sv` → `foo_sv.rs` 平行文件 + mapping sidecar)。这样用户已有的 rust-analyzer 天然索引它,`.rs` 侧代码引用组件 API 有完整体验;`.sv` 侧的 hover/补全/跳转只是"位置换算 + 转发"的薄层;cargo check 的诊断天然产生、只需重映射。**"生成文件是真文件"把 IDE 问题的下限从'全靠自建'拉到'只欠一层映射'**,这是本报告最核心的判断。
3. **MVP 顺序应该反直觉地把 LSP 放最后**:① 让 codegen 本身 IDE 友好(表达式逐字、错误落点锚定、pretty-print、mapping sidecar);② TextMate/tree-sitter 高亮(模板内 Rust 表达式 injection);③ `sv check` CLI(cargo check JSON → .sv 重映射,svelte-check 等价物,同时服务 CI 与编辑器 problem matcher);④ VS Code 扩展用 `vscode.executeXxxProvider` 客户端转发(复用用户已在跑的 r-a,**零双实例**,Vue language-tools 3.0 就是把通信责任挪回 client 侧的);⑤ 编辑器无关的 sv-language-server(自持 r-a 子进程做 LSP proxy)。①–④ 合计约 **9–16 人周** 可到"日常可用";⑤ 再加 8–16 人周。
4. **最大技术风险不是转发本身,而是"写操作"的映射质量**:completion 的 auto-import(additionalTextEdits 落在生成文件的 use 区,要插回 .sv 的 script 块)、rename 跨 .sv/.rs、assist/quick-fix。Vue/Svelte 团队各打磨了 5 年以上,而且 2026 年还在为 TypeScript 7(tsgo,Go 重写)没有稳定 programmatic API 而停摆——**耦合宿主分析器是长期税,不是一次性成本**。对策:第一年只做只读特性(hover/def/诊断)+ 无 auto-import 的补全,写操作明确降级。
5. **cxx-qt 的现状是反面教材**:官方明说 rust-analyzer 插件与 QML 插件"不共享类型信息,Rust 类型在 QML 文件里不被识别,是已知限制"。两个独立工具链的类型世界靠事后桥接极难;我们避开它的方式恰恰是**不发明第二个类型世界**——.sv 里的表达式就是真 Rust,类型检查全部由 rustc/r-a 完成,sv 编译器只做语法变换,永远不需要"理解类型"。
6. **Slint 路线(自建全套 LSP)维持 04 号报告的判断:排除**。slint-lsp 是"编译器 + LSP + live-preview + 格式化"整体投入的一部分,前提是语言表达式也是自己的——我们表达式是 Rust,自建全套等于重写 rust-analyzer,不成立。
7. "没有 LSP 的第一年"完全可以体面:高亮(TextMate injection 让 script 块与模板表达式直接按 Rust 高亮)+ `sv check` 精确诊断 + **"Go to generated code" 命令**(生成文件可读、带回链注释,用户可跳进去享受完整 r-a,改完回来)+ bacon/cargo-watch 工作流。Dart 生态的 build_runner(`.g.dart`)证明"真实生成文件 + 好的可读性"本身就是一种及格的 DX。

---

## 1. 先例架构拆解(2026-07 现状)

### 1.1 Volar / Vue:虚拟代码 + 映射 + TS 当库用——以及三次架构大改的教训

Volar.js 现在是一个通用的 embedded-language tooling 框架(`@volar/language-core` / `language-service` / `language-server`),Vue、Astro、MDX 都基于它。机制:每个 `.vue` 文件由 `createVirtualCode/updateVirtualCode` 生成 `VirtualCode` 对象——template/script/style 各自变换为虚拟 TS/CSS 文档,`mappings` 数组记录 source↔generated 的区间对,并且**每段映射携带能力标记**(哪些区间参与 completion、navigation、semantic、verification 等),LSP 请求按光标所在映射转发到对应虚拟文档,结果反向映射回 .vue。

对我们更有信息量的是它的**架构演化史**(每一步都是血泪):

- **v1 "Take Over 模式"**:让 Vue LS 接管所有 .ts/.vue 文件,避免和 VS Code 内置 tsserver 双实例。结果:与内置 TS 体验冲突、维护负担大,**2.0 废弃**。
- **v2 "Hybrid 模式"**:Vue LS 只管 CSS/HTML,TS 能力由 **tsserver + @vue/typescript-plugin** 提供(把 Vue 的虚拟文档机制作为 TS 插件塞进 tsserver 进程),两者之间靠命名管道通信。结果:兼容性问题不断。
- **v3(2025-07)**:砍掉命名管道层,**把"两个 server 的协调责任"转移到 LSP client 侧**;`typescript.tsdk` 配置移除。
- **2026-07 的新剧情:TypeScript 7(tsgo,Go 重写)于 2026-07-08 GA**,但**没有稳定的 programmatic API**——Vue/Svelte/Astro/MDX/Angular 的模板类型检查全部依赖 TS 编译器 API,官方明确说稳定 API 要等 7.1(预计 2026-10)。整个"虚拟文档寄生宿主分析器"的生态被上游一个重写卡住。

**教训三条**:(a) 与宿主分析器的进程/实例关系是这个架构里最反复折腾的部分,能复用宿主已有实例就复用,协调逻辑放 client 侧最薄;(b) 映射要带能力标记,不是所有区间所有特性都转发;(c) 宿主分析器的形态变化是长期风险,耦合面越窄越好——**我们对 r-a 只用标准 LSP 面,不碰内部 API,是最抗变化的姿势**。

### 1.2 svelte-language-tools:svelte2tsx 路线,且明确没有迁去 Volar

Svelte 官方工具链:`svelte-language-server` + `svelte2tsx` + `svelte-check` + VS Code 扩展 + typescript-plugin。核心是 **svelte2tsx**:把 `.svelte` 组件(script + template)变换成一个 TSX 虚拟文件——script 内容基本逐字保留,模板变换成能让 TS 推断的表达式代码,输出 `code + map`(magic-string 生成的 v3 SourceMap)。语言服务器自持一个 TS LanguageService 实例分析虚拟文件,诊断/hover/补全经 sourcemap 回映。**svelte-check** 是同一管线的 CLI 形态:svelte2tsx → TS 程序级检查 + svelte.compile 的警告,合并输出,`--output machine` 供 CI 消费。

Volar 曾做过 Svelte 集成预览(volarjs/svelte-language-tools),但 **Svelte 团队最终没有迁移**,继续演进自己的 LS(2026-01 大规模性能优化、2026-06 支持 TypeScript 6.0;TS 7 RC 曾直接 crash svelte2tsx)。社区还长出了 svelte-check 的加速替代品(svelte-check-native、svelte-fast-check、rsvelte——用 Rust 重写检查管线)。

**对我们的启示**:svelte2tsx 的"**script 逐字搬运 + 模板变换成类型透明的表达式**"就是我们 codegen 的形态模板;"语言服务器和 CLI checker 共用同一变换器"就是 sv-language-server 与 `sv check` 共用 sv-compiler 的架构;而"没迁 Volar"说明**框架化的虚拟文档层不是必须的,自己维护一个专用变换器 + 映射完全撑得起一等公民的 DX**。

### 1.3 Slint:自建全套的成本结构(继续排除)

slint-lsp 实现了完整 LSP(诊断/补全/goto/format)+ live-preview,原生与 WASM 双形态,复用 `i_slint_compiler` 的 parser 与 lookup。它成立的前提是 **Slint 语言的表达式也是 Slint 的**——编译器本来就要做完整语义分析,LSP 是编译器的自然延伸。我们的 .sv 表达式是真 Rust,语义分析属于 rustc/r-a;自建全套 = 重写 rust-analyzer,荒谬。**Slint 唯一值得抄的是"编译器做成库、被 build.rs / LSP / preview 三个前端共用"的组织方式**——这与 04 号报告的结论一致,且是本报告方案的地基。

### 1.4 qmlls / cxx-qt:两个类型世界桥不起来的实证

Qt 6 的 qmlls 是又一个"自建全套"(补全/lint/格式化/goto),Qt 6.11 才做到"Go to C++ definition"、多项目支持——注意这是 Qt 体量的公司做了 5 个大版本的进度。而 **cxx-qt(Rust ⇄ Qt 桥)的官方文档明说:推荐 VS Code + rust-analyzer + Qt QML 插件,但"两个插件不共享类型信息,Rust 类型在 QML 文件中显示为不可识别,这是已知限制"**。也就是说:当 UI 语言与逻辑语言分属两个类型系统、靠 FFI 桥接时,IDE 层面的类型贯通至今是行业未解题。我们的架构性规避:**.sv 不引入第二个类型系统**——模板表达式、props 类型、事件闭包全是 Rust 类型,生成进 .rs 后由 r-a 一站式理解。这是"独立编译器 + 虚拟文档转发"路线可行性的根本原因。

### 1.5 轻量转发的活先例:otter.nvim / lsp-proxy / VS Code 官方指南

- **otter.nvim**(Quarto/markdown 生态):对每个嵌入语言创建隐藏 buffer,**其余行用空行占位保持行号一致**,让对应 LS attach 到隐藏 buffer,请求按光标所在代码块转发、结果映射回主 buffer。证明"编辑器侧虚拟文档 + 转发"对包括 rust-analyzer 在内的任意 LS 都能跑通基础特性。
- **Emacs lsp-proxy**:通用 request forwarding + org-babel 虚拟文档 + 位置换算 + 块信息缓存,同一模式的 Emacs 实现。
- **VS Code 官方 embedded-languages 指南**:标准答案就两种——language services(自持宿主分析器实例)与 **request forwarding**(把请求转回 client,让已注册的 provider 处理),并明确指出自持实例的缺点是"你要持续跟进你依赖的语言服务的更新"。

这三者共同指向:**转发层本身是成熟模式,工程量可控;难点全在映射质量与生命周期同步**。

---

## 2. rust-analyzer 的可编程接入点盘点(2026-07 核实)

结论式列举,按对本项目的可用性排序:

### 2.1 标准 LSP 面(我们当 client)——首选接入点

r-a 就是一个 LSP server。sv-language-server 可以 spawn 它当子进程,自己作为 LSP client 与之对话:`didOpen/didChange` 维护文档、发 hover/completion/definition 请求、收 `publishDiagnostics`。r-a 对**打开文档的内存 overlay**支持是一等的:`didOpenTextDocument` 把文档写入 `mem_docs` 并 `set_file_contents` 进 VFS,编辑器里未保存的内容就是分析真相(VFS 的职责定义即"合并编辑器状态与磁盘状态")。这意味着**转发管线可以做到 keystroke 级新鲜度,不必等落盘**。

两个需要原型验证的边界(文末 open question):
- **overlay-only 文件**(磁盘不存在、仅 didOpen 的路径)能否被模块树接收:机制上 VFS 有该文件、父模块 `mod foo_sv;` 指向它即可解析,但 r-a 的 source-root 划分与 watcher 假设"文件集合可枚举"(Better VFS issue #3715 记录了 eager/lazy 的张力),未见明确承诺。**规避方案(推荐):生成文件落盘,overlay 只作为编辑期的增量刷新**——落盘保证 crate graph 稳定,overlay 保证新鲜度,两者叠加没有冲突。
- unlinked file(不在任何 crate 里)只有降级服务并报 "not included in any crates" 诊断;cargo script(单文件包)支持仍是实验态。**所以虚拟 .rs 必须真正接进 crate graph,不能当孤儿文件用**。

### 2.2 crate graph 接线:cargo workspace / rust-project.json / discoverConfig

三种把生成文件"变成 r-a 眼中合法模块"的方式:

1. **cargo 原生(推荐)**:生成的 `foo_sv.rs` 位于用户 crate 源码树内、由 `mod` 声明引用(或 build.rs 写 OUT_DIR + `include!`——r-a 默认运行 build script 并索引 OUT_DIR 产物,但 OUT_DIR 路径含 hash、flycheck 曾有 "file not found in VFS" 类问题 #13520,体验二等)。零额外配置,用户已有 r-a 直接工作。
2. **rust-project.json**:非 cargo 构建系统的 crate graph 序列化格式(crates/deps/edition/cfg/sysroot/proc-macro dylib/runnables)。我们用不上主路径,但它证明 r-a 从设计上接受"外部工具喂 crate graph"。
3. **discoverConfig(JSONL 协议)**:Buck2(`rust-project develop-json`)/Bazel 用的动态项目发现——r-a 调用你的命令、你流式返回 project JSON 与进度。**这是"用自己的构建系统驱动 r-a"的官方蓝本**;若未来 .sv 走完全自定义构建(不经 cargo),这条路已铺好。flycheck 也可经 `runnables`/`check.overrideCommand` 定制(PR #18043)。

### 2.3 ra_ap_*:r-a 作为库的现状——可用但税重,列为远期选项

- rust-analyzer 内部就是一组库,以 `ra_ap_*` 前缀**每周自动发布**到 crates.io(核实:`ra_ap_ide` 0.0.342,2026-07-13 发布)。`AnalysisHost`(状态)+ `apply_change`(FileChange)+ `Analysis` 快照(completion/hover/goto/diagnostics 全有),VFS 支持内存内容——**API 面完全够我们做一个进程内嵌 r-a 的 sv-language-server**。
- 但**明确不承诺稳定**:0.0.x、不遵循 semver、任何一周都可能 breaking、十几个 crate 必须整体锁版本升级。实际消费者 cargo-modules(用 ra_ap 做模块/依赖图分析)就是"精确 pin + 定期整体升级"的活样本。
- **salsa 移植已完成**(2025-03,PR #18964,changelog #277),官方动机就是解锁并行求值与持久化缓存;移植初期出过内存翻倍(#19402)、性能回退(#19404)等问题,后续在修。**"r-a 库化 + 并行 + 持久缓存"的中期趋势对我们有利**,但 2026-07 的今天,进程内嵌方案的维护税(每周 breaking 的追赶)明显高于 LSP 转发,**只应在转发方案撞到硬墙(比如需要深度定制补全排序/跨文档语义)时启用**。
- 顺带:wgsl-analyzer 等项目选择的是"抄 r-a 架构自建"而非复用 ra_ap——那是给"自有语言"准备的路,不适用于我们。

### 2.4 scip / lsif:CI 侧代码智能(锦上添花)

`rust-analyzer scip` / `lsif` CLI 持续维护(2025 年还在修 SCIP 的 bug),Mozilla searchfox 等已从 save-analysis 迁到 SCIP。对我们:CI 里可以对含生成代码的 workspace 出 SCIP 索引,再写一个后处理器把生成文件的 occurrence 经 mapping 重写回 .sv,即可让 Sourcegraph/代码浏览类工具"看懂" .sv。优先级低,但路是通的。

### 2.5 明确不存在的东西(设计时当作公理)

- **r-a 没有插件 API**——不存在 tsserver-plugin 的等价物,不能把我们的虚拟文档机制"注入" r-a 进程。
- **没有官方途径复用编辑器中已运行的 r-a 实例**(LSP 是点对点的)。仅 VS Code 例外:client 侧 `vscode.executeXxxProvider` 命令可以借用任何已注册 provider(含 r-a),这正是 §3.2 方案 A 的支点。
- **Rust 没有 `#line` 指令**——不能像 C 预处理器那样让 rustc 直接把诊断标到 .sv,所有回映必须由工具层完成。

---

## 3. 方案设计:sv 工具链蓝图

### 3.0 总原则:一个编译器库,四个消费者

```
                 ┌────────────────┐
     .sv 源码 ──▶│  sv-compiler   │──▶  foo_sv.rs(可读、落盘、逐字表达式)
                 │ (parser/IR/    │──▶  foo_sv.rs.map(区间映射 sidecar)
                 │  codegen/map)  │──▶  模板语法诊断(直接 .sv 坐标)
                 └────────────────┘
   消费者:build.rs(构建) · sv check(CI 诊断) · sv-ls / VS Code 扩展(IDE) · sv fmt(格式化)
```

**同构保证**:IDE 看到的虚拟文档与 build 产物是同一个函数的输出,永不漂移——这是 Slint(compiler lib 三前端共用)与 svelte2tsx(LS 与 svelte-check 共用)共同验证过的组织方式,也是 04 号报告"编译器做成独立库"决策在 IDE 维度的直接回报。

### 3.1 决策 A:虚拟 .rs 落盘,作为一等构建产物

推荐布局(`foo.sv` 与产物同目录或平行 `gen/` 树,二选一,倾向同目录后缀式):

```
src/components/todo.sv          # 用户写的
src/components/todo_sv.rs       # 生成(.gitignore 可选;建议默认忽略)
src/components/todo_sv.rs.map   # 映射 sidecar(JSON)
src/components/mod.rs           # 含 `mod todo_sv;`(由 CLI 维护或 include! 聚合)
```

生成文件结构(svelte2tsx 纪律的 Rust 版):

```rust
// ⚠ Generated from todo.sv — edit the .sv file, not this one.
// sv-compiler <version>, map: todo_sv.rs.map

// ── script 块:逐字搬运(连续 1:1 映射区间)──
use crate::api::Todo;                          //@sv 3:1
let todos = store(Vec::<Todo>::new());          //@sv 5:1

// ── 模板:每个动态绑定一个语句,用户表达式逐字出现 ──
// <Checkbox bind:checked=…>  (todo.sv:24:9)
let __sv_b3: RwSignal<bool> = todo.done();      // 用户表达式 `todo.done()` 逐字,类型锚定
__doc.bind_checked(__n7, __sv_b3);
```

理由与取舍:
- **落盘 → 用户已有 r-a 自动索引**,.rs 侧引用组件零配置可用;`Go to generated` 兜底体验免费;cargo check/clippy 天然覆盖生成代码。
- **代价**:源码树被写入(用 `_sv.rs` 后缀 + 文件头警告 + .gitignore 缓解;Dart build_runner 的 `.g.dart` 是大规模同型先例)、编辑期与构建期双写者需约定(编译器输出确定性 + 原子写)。
- 拒绝纯 overlay(不落盘)作为主方案:模块树接线依赖未承诺的 VFS 行为(§2.1),且用户 r-a 与我们的 r-a 看到的世界会分叉。overlay 仅作 keystroke 级增量通道。
- 拒绝 OUT_DIR + `include!` 作为主方案:路径不稳定、诊断指向 target/ 深处、flycheck VFS 匹配有历史坑;可作为"不想污染源码树"用户的可选模式。

### 3.2 决策 B:转发目标分两阶段——先 VS Code client 转发,后通用 proxy

**方案 A(MVP,VS Code only):扩展内 client-side 转发,复用用户已在运行的 r-a,零双实例。**

- .sv 打开时,扩展用 `workspace.openTextDocument(genUri)` 把生成文件在后台打开;keystroke(防抖 ~200–300ms)时:调用 sv-compiler(WASM 或本地进程)重新生成 → 用 `WorkspaceEdit` 更新生成文档 buffer(不保存)→ VS Code 自动把 didChange 同步给 r-a(overlay 更新,不落盘)。
- hover/def/completion/references:光标位置经 .map 换算到 genUri → `vscode.executeHoverProvider / executeDefinitionProvider / executeCompletionItemProvider …` → 结果 range/uri 反向换算回 .sv。
- 诊断:监听 `languages.onDidChangeDiagnostics`,取 genUri 上的诊断(r-a 语义诊断 + flycheck)→ 重映射后经 `DiagnosticCollection` 发布到 .sv;能映射进用户表达式区间的原样发,落在胶水代码的收敛为"组件级内部错误"(那是我们 codegen 的 bug,单独上报通道)。
- 保存 .sv 时同步保存生成文档(触发 cargo check 用新内容)。
- 这就是 Vue v3"协调责任移到 client 侧"的极简版,也是 otter.nvim 的 VS Code 化。**没有第二个 r-a、没有自建 LSP server,是全部方案里工程量最小、且与 r-a 升级解耦最好的**。

**方案 B(通用):sv-language-server(Rust,基于 r-a 团队的 lsp-server crate 或 tower-lsp)自持 r-a 子进程。**

- 面向 Neovim/Helix/Zed/其他编辑器;sv-ls 注册 .sv,内部 spawn `rust-analyzer`,以 LSP client 身份维护生成文档 overlay,转发全部请求。
- **诚实代价:双 r-a 实例**(用户编辑器为 .rs 跑一个,我们为 .sv 跑一个;大 workspace 每实例 GB 级内存)。Vue/Svelte 用户忍受双 TS 实例多年,可接受但要写进文档;"接管模式"(让 sv-ls 也服务 .rs)是 Vue 已废弃的 Take Over 路线,不要走。
- position encoding:LSP 默认 utf-16,r-a 支持 utf-8 协商;.map 用字节偏移,换算统一走 line-index,一次写对。

### 3.3 映射 sidecar 格式(Volar mappings 的最小化版)

```json
{ "version": 1,
  "source": "todo.sv", "generated": "todo_sv.rs",
  "segments": [
    { "sv": [120, 380], "rs": [96, 356], "kind": "script" },
    { "sv": [512, 523], "rs": [1180, 1191], "kind": "expr",
      "anchor": [498, 540] },
    { "rs": [900, 1100], "kind": "glue", "anchor": [470, 620] } ]
}
```

- `script`:连续逐字区间(offset 差恒定,映射 O(1));`expr`:模板内表达式散点(逐字);`glue`:框架胶水,不参与正向特性,反向仅用 `anchor`(所属模板节点在 .sv 的区间)收敛诊断。
- 借鉴 Volar 的 per-mapping 能力标记思想但先不做全套:MVP 只区分"逐字区(全特性)/胶水区(仅诊断收敛)"两档,够用;后续需要再加 completion/navigation 细分。
- 由 codegen 在 emit 时顺手记录(magic-string 式 append/move),不做事后 diff 推断。

### 3.4 请求级映射的难度分层(决定第一年做什么、不做什么)

| 特性 | 难度 | 第一年 | 备注 |
|---|---|---|---|
| 诊断(语义 + cargo check) | 低 | ✅ | 逐字区精确映射,胶水区锚点收敛 |
| hover / goto definition / references(.sv→.rs 方向) | 低 | ✅ | 纯位置换算;hover 里出现 `__sv_*` 名字需做展示层清洗 |
| completion(主 edit) | 中 | ✅ | TextEdit range 回映;filterText/range 按 .sv 上下文校正 |
| completion 的 auto-import(additionalTextEdits) | 高 | ❌ 降级 | edit 落在生成文件 use 区 → 需插回 .sv script 块;svelte2tsx 同款老大难 |
| signature help / inlay hints | 中 | 二年 | inlay 反向散点多,量大 |
| rename(跨 .sv/.rs) | 高 | ❌ | WorkspaceEdit 多文件回映 + 生成文件不可编辑的语义 |
| assist / quick-fix | 高 | ❌ | 同上,且很多 assist 语义在 .sv 中不成立 |
| semantic tokens | 中 | 二年 | 区间批量回映,机械但量大;先靠 TextMate/tree-sitter |
| .rs→.sv 方向(从 Rust 代码跳进组件定义) | 中 | 二年 | 需要 r-a 结果后处理:目标在生成文件时改指 .sv |

### 3.5 生命周期与增量

- keystroke → 防抖 → sv-compiler 增量重生成(编译器输出**位置稳定**:改一个表达式只漂移该行附近,靠"每绑定一语句"的 codegen 纪律保证)→ 对生成文档发**区间 didChange**(而非全文替换),给 r-a 的 salsa 失效面最小化。
- 新增/删除 .sv 文件 → 重写 mod 聚合文件 → r-a 经 watcher(落盘方案免费)感知 crate 结构变化。
- 模板语法错误(未闭合标签等)时:**codegen 必须仍产出可编译骨架**(04 号报告的错误恢复原则在此同样是生命线),否则生成文档满屏错误、转发特性全灭。

---

## 4. 诊断映射:编辑器内与 CI 两条路径

**分类前提**:.sv 的错误分三层——① 模板语法/领域错误(标签不闭合、bind 到只读 signal 等):sv-compiler 自己诊断,**直接持有 .sv 坐标,不经 rustc**(Slint 同型);② 用户 Rust 表达式的类型/借用错误:rustc/r-a 产生,落在生成文件的逐字区,**精确回映**;③ 胶水代码错误:理论上只在 sv-compiler 有 bug 时出现,收敛为组件级错误 + 引导上报。

**编辑器内路径**:§3.2 已述——r-a 的语义诊断与它代跑的 flycheck(cargo check)都发布到生成文件 URI,由扩展/proxy 拦截、按 .map 改写 uri+range 后发布到 .sv。注意去重(r-a 原生诊断与 flycheck 会部分重叠,r-a 自己已有去重逻辑,改写层不要再放大)。不建议反向做法(让 r-a 的 `check.overrideCommand` 直接输出 .sv �标的假 rustc JSON):r-a 会因 .sv 不在其 VFS 而丢弃或告警(#13520 同类问题),映射责任应留在我们这层。

**CI 路径(`sv check`,svelte-check 等价物)**:

```
cargo check --workspace --message-format=json
  → 逐条解析(cargo_metadata crate;span: file_name/byte offsets/line/col/expansion)
  → file_name 命中 *_sv.rs 者:读 .map 重映射(含 span 内多标注、suggestion 的 replacement 区间)
  → 命中逐字区:改写为 .sv 坐标输出;命中胶水区:锚点收敛
  → 输出:human(彩色,引用 .sv 原文行)/ json / github-annotations 三种格式
```

- rustc JSON 的 `rendered` 字段(预渲染的人类可读文本)包含生成文件路径与代码摘录,human 输出要用 .sv 原文**重新渲染**,不能直接透传 rendered——这是工作量主体(约一半),但 svelte-check 的 machine/verbose 输出设计可直接抄。
- 同一命令兼任编辑器 problem matcher 数据源(VS Code task + bacon/cargo-watch 联动),是"没有 LSP 的第一年"的主力诊断通道。
- suggestion(rustc 的机器可应用修复)落在逐字区时可透传为 .sv 的 quick-fix 数据,属免费增值;落在胶水区丢弃。

**错误落点设计(codegen 侧的配套纪律,决定回映质量上限)**:
- 每个用户表达式先 `let` 绑定到**期望类型显式标注**的局部变量(`let __sv_b3: RwSignal<bool> = {user_expr};`),让类型不匹配的主 span 落在 user_expr 逐字区内,而不是深埋在 builder 泛型链里被放大到不可读——这同时压制了 Leptos 式"一屏泛型错误"的问题。
- 事件闭包整体逐字透传,期望签名同样用 let 锚定。
- 生成代码**零宏**(不 `macro_rules!`、不嵌 proc-macro):宏展开会制造二级 span(expansion 链),破坏"生成文件坐标 = 最终坐标"的假设,rustc JSON 的 expansion 处理复杂度会翻倍。

---

## 5. 语法高亮与格式化

**TextMate grammar(VS Code / 兼容编辑器,1–2 人周)**:`source.sv` 主 grammar:template 标签/属性/控制流自绘;`<script>` 块 `include: source.rust` 整块注入;模板 `{...}`、属性值表达式、`if/for/match` 头部同样注入 `source.rust`。Svelte/Vue 的 TextMate grammar 是现成结构模板。配套 language-configuration(注释/括号/缩进)半天。**这一步性价比全项目最高:script 块与所有表达式立刻获得正常 Rust 观感。**

**tree-sitter grammar(Neovim/Helix/Zed,2–4 人周)**:`tree-sitter-sv` + `queries/injections.scm` 把 script 内容与模板表达式节点标记为 `@injection.language rust`(机制成熟:tree-sitter-html/embedded-template 同款;Neovim 支持 `injection.combined` 把散点表达式合并为一个嵌套文档解析)。tree-sitter-svelte 可作骨架参考。它同时是 Zed 扩展的前提、未来 sv-fmt/结构化编辑的潜在地基(但 sv-fmt 建议直接复用 sv-compiler 的 parser,见下)。注意 tree-sitter 对"表达式内含 `}`/字符串嵌套"的 injection 边界要在 grammar 里精确划定,这是主要工作量来源。

**semantic tokens(二年)**:经 §3 转发管线把 r-a 的 semantic tokens 区间回映——机械但收益中等,TextMate 先顶着。

**sv fmt(2–4 人周,二阶段)**:模板部分用 sv-compiler 的 AST 自写 printer(节点属性排版、控制流缩进);script 块与模板内表达式抽出 → 喂 rustfmt(stdin 片段模式,表达式包 `fn __f(){ (…) }` 壳再剥壳)→ 拼回。svelte-language-tools 用 prettier 插件的分工同型。生成文件本身用 prettyplease 保证可读(这属于 codegen,不属于 sv fmt)。

---

## 6. MVP 分步、工作量与风险

前提:sv-compiler(parser/IR/codegen)已按 04/本报告的纪律存在;估算单位为资深 Rust 工程师人周,含测试不含长尾打磨。

| 阶段 | 内容 | 人周 | 退出标准 |
|---|---|---|---|
| S0 | codegen IDE 整备:逐字区/类型锚定/零宏/位置稳定/prettyplease;.map sidecar emission | 2–3 | 手写组件的生成文件可读;错误落点抽查达标 |
| S1 | TextMate grammar + VS Code 语言基础;(并行)tree-sitter-sv | 1–2(+2–4) | script/表达式 Rust 高亮正确;Neovim 可用 |
| S2 | `sv check`:cargo JSON 重映射,human/json/annotations 输出 | 2–3 | CI 红绿正确;类型错误精确标到 .sv 表达式 |
| S3 | VS Code 扩展转发 MVP:WorkspaceEdit 同步生成文档 + hover/def/completion(主 edit)+ 诊断重映射 | 4–8 | .sv 内 hover 出 Rust 类型;def 跳到 .rs;补全在表达式内可用 |
| S4 | 通用 sv-language-server(自持 r-a 子进程 proxy) | 8–16 | Neovim/Helix 达到 S3 同等只读体验 |
| S5 | 二年题:auto-import 回插、rename、semantic tokens、inlay hints、.rs→.sv 反向 | 持续 | — |

**MVP(S0–S3)≈ 9–16 人周;含 S4 全景 ≈ 17–32 人周**。对照:Vue/Svelte 是多人多年——我们便宜两个数量级的原因必须诚实列出:(a) 表达式是真 Rust,不需要 svelte2tsx 花在"为模板语法建类型环境"上的大量代码(那部分我们的等价物就是编译器本体);(b) 第一年砍掉全部写操作特性;(c) 先只做 VS Code。任何一条不成立,估算失效。

**风险排序**:

1. **写操作映射(auto-import/rename/assist)**——已定性为二年题并降级,风险转化为"补全没有 auto-import 的体验损耗"(Rust 用户对手写 use 的容忍度高于 TS 用户,可接受)。
2. **r-a 对高频 didChange 大生成文件的增量性能**——salsa 失效级联下,3k 行生成文件每 300ms 一改的 CPU 占用未知;缓解:区间 didChange、位置稳定 codegen、防抖自适应。需要 S3 早期就做压测(open question)。
3. **VS Code executeXxxProvider 的语义缺口**(completion resolve 限制、triggerCharacter 传递、command 字段处理)——有社区先例但细节要实测。
4. **双实例内存(S4)**——文档化 + 远期看 r-a 持久化缓存/并行化的上游进展。
5. **上游形态变化**——r-a 是活跃项目,LSP 面稳定但行为细节会变;TS7 事件是前车之鉴,我们只耦合标准 LSP + cargo JSON 两个最稳接口,已是最小耦合面。

**明确不做**:进程内嵌 ra_ap(每周 breaking 的追赶税,仅当转发撞硬墙再评估);Take Over 式接管 .rs;自建表达式语义分析(任何形式)。

---

## 7. "没有 LSP 的第一年":止血包

优先级排序(全部落在 S0–S2,即 LSP 之前):

1. **生成代码可读性当产品特性做**:文件头警告与回链、每个模板节点一条 `// <Button on:click=…> (todo.sv:42:7)` 注释、prettyplease 排版、`_sv.rs` 命名可预测。用户"跳进生成文件改、改完搬回 .sv"是丑但完整的 r-a 体验(Dart `.g.dart` 生态的日常;Slint 用户同样如此)。配 VS Code 命令/CLI:`sv expand todo.sv`(打开/打印生成文件并定位到对应节点)。
2. **`sv check` + bacon/cargo-watch 工作流**:保存即出 .sv 坐标的精确诊断,problem matcher 进编辑器问题面板——**没有 LSP 时,诊断质量就是 DX 的全部下限**,这也是 svelte-check 先于良好 LSP 存在的历史顺序。
3. **错误落点纪律(S0)**:类型锚定 let 绑定让 rustc 错误天然指向用户表达式;模板域错误(bind 到只读、属性拼错 did-you-mean)全部在 sv-compiler 内以 .sv 坐标直报,永不让用户面对胶水代码的错误。
4. **高亮先行(S1)**:TextMate injection 成本一周级,观感立刻从"纯文本"到"正常语言"。
5. **组件风格引导**:文档与模板鼓励"逻辑放 .rs 模块、.sv 只放模板 + 薄 script"——.rs 部分全程满血 r-a,把 IDE 盲区面积压到模板表达式这个最小集合。这不是妥协话术:它同时是好架构(可测试性)与 Svelte 社区已有的最佳实践方向。

---

## 8. 结论与建议

- **路线判定:Volar 式虚拟文档转发 rust-analyzer 成立**,但形态是"**落盘生成文件 + sidecar 映射 + 两阶段转发(VS Code client 转发 → 通用 LSP proxy)**",而非把 r-a 当库嵌入。它把独立编译器路线最大的短板(模板内 IDE 体验)从"Slint 级全家桶成本"压到"一层位置映射的成本",且与 04 号报告"编译器做成独立库"的既有决策完全咬合。
- **与 proc-macro 路线的对比结论没有反转**:proc-macro 路线的 IDE 体验仍然更便宜(r-a 原生在场,零转发层)。独立编译器路线买到的是:真正的 Svelte 单文件心智、无宏展开约束的 codegen 自由度(生成文件可读、可 diff、可断点)、更干净的热重载/远程预览通道;付出的就是本报告估算的 9–16 人周起步的工具链投入与长期维护位。**这笔账现在算得清了,决策层可以据此拍板**。
- 若走此路线,**立即行动项**是把 S0(codegen IDE 整备 + .map emission)纳入编译器 MVP 的验收标准——映射不是事后贴的,是 codegen 的输出契约;事后补映射等于重写 codegen。

---

## 9. 来源

**Volar / Vue**
- Volar.js 仓库与文档(language-core/VirtualCode/embeddedCodes):https://github.com/volarjs/volar.js/ · https://volarjs.dev/reference/languages/ · https://volarjs.dev/guides/first-server/
- Vue 官方博客 "Volar: a New Beginning":https://blog.vuejs.org/posts/volar-a-new-beginning
- Hybrid 模式讨论(v2.0):https://github.com/vuejs/language-tools/discussions/3789 · 完整 LS 回归 PR:https://github.com/vuejs/language-tools/pull/4119
- v3 升级指南(通信责任移到 client 侧、tsdk 移除):https://github.com/vuejs/language-tools/discussions/5456
- TS Native/tsgo 兼容性追踪:https://github.com/vuejs/language-tools/issues/5381

**Svelte**
- sveltejs/language-tools(LS/svelte2tsx/svelte-check/typescript-plugin):https://github.com/sveltejs/language-tools · svelte2tsx 包:https://github.com/sveltejs/language-tools/tree/master/packages/svelte2tsx
- LS 方案总览 issue #11 · svelte2tsx 引入 PR #57 · Volar 集成预览 issue #2267:https://github.com/sveltejs/language-tools/issues/2267 · Volar 侧示例:https://github.com/volarjs/svelte-language-tools
- "Svelte <3 TypeScript"(架构由来):https://svelte.dev/blog/svelte-and-typescript
- 2026 年动态(1 月性能优化、6 月 TS 6.0 支持):https://svelte.dev/blog/whats-new-in-svelte-january-2026 · https://svelte.dev/blog/whats-new-in-svelte-june-2026
- TS7 RC crash svelte2tsx issue #3063:https://github.com/sveltejs/language-tools/issues/3063 · 加速替代品:https://github.com/harshmandan/svelte-check-native · https://github.com/baseballyama/rsvelte

**TypeScript 7 / tsgo(2026-07 事件)**
- 官方公告:https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/
- Vue/Svelte 等待 7.1 API 的报道:https://byteiota.com/typescript-7-go-native-compiler/ · https://www.techtimes.com/articles/320049/20260710/typescript-7-now-stable-10-faster-builds-not-vue-svelte-yet.htm

**Slint / Qt**
- slint-lsp README(功能面、native+WASM):https://github.com/slint-ui/slint/blob/master/tools/lsp/README.md
- qmlls(Qt 6.11:goto C++ definition、多项目):https://www.qt.io/blog/whats-new-in-qml-language-server-in-6.11 · https://doc.qt.io/qt-6/qtqml-tooling-qmlls.html
- cxx-qt(r-a 与 QML 插件不共享类型信息的 known limitation):https://github.com/KDAB/cxx-qt

**轻量转发先例**
- otter.nvim(隐藏 buffer + 行号对齐 + 转发):https://github.com/jmbuhr/otter.nvim
- Emacs lsp-proxy(org-babel 虚拟文档 + 位置换算):https://github.com/jadestrong/lsp-proxy
- VS Code 官方 embedded languages 指南(request forwarding vs language services):https://code.visualstudio.com/api/language-extensions/embedded-languages · API commands(executeXxxProvider):https://code.visualstudio.com/api/references/commands

**rust-analyzer 接入点**
- 非 Cargo 项目 / rust-project.json / discoverConfig:https://rust-analyzer.github.io/book/non_cargo_based_projects.html · 深度集成 issue #13446:https://github.com/rust-lang/rust-analyzer/issues/13446 · flycheck discoverConfig PR #18043:https://github.com/rust-lang/rust-analyzer/pull/18043 · Bazel rules_rust:https://bazelbuild.github.io/rules_rust/rust_analyzer.html
- ra_ap 库化:https://docs.rs/ra_ap_ide/latest/ra_ap_ide/(0.0.342,2026-07-13)· https://crates.io/crates/ra_ap_rust-analyzer · 消费者先例 cargo-modules(版本 pin 实践):https://github.com/regexident/cargo-modules
- salsa 移植:PR #18964:https://github.com/rust-lang/rust-analyzer/pull/18964 · changelog #277:https://rust-analyzer.github.io/thisweek/2025/03/17/changelog-277.html · 移植后回退 #19402/#19404 · 持久缓存诉求 #4712 · 性能计划 #17491
- VFS/overlay/unlinked:Better VFS #3715:https://github.com/rust-lang/rust-analyzer/issues/3715 · standalone files PR #8955:https://github.com/rust-lang/rust-analyzer/pull/8955 · cargo script #15318 · cargo 团队 HackMD:https://hackmd.io/@rust-cargo-team/HJZ7cw5uxl · OUT_DIR flycheck VFS 问题 #13520:https://github.com/rust-lang/rust-analyzer/issues/13520
- scip/lsif:https://rust-lang.github.io/rust-analyzer/src/rust_analyzer/cli/scip.rs.html · searchfox 迁移:https://bugzilla.mozilla.org/show_bug.cgi?id=1761287

**诊断与高亮**
- rustc JSON 诊断格式:https://doc.rust-lang.org/rustc/json.html · cargo external tools:https://doc.rust-lang.org/cargo/reference/external-tools.html · cargo_metadata:https://docs.rs/cargo_metadata/latest/cargo_metadata/
- tree-sitter 高亮与 injection:https://tree-sitter.github.io/tree-sitter/3-syntax-highlighting.html · tree-sitter-embedded-template:https://github.com/tree-sitter/tree-sitter-embedded-template · Pulsar injections 详解:https://blog.pulsar-edit.dev/posts/20231110-savetheclocktower-modern-tree-sitter-part-5/ · Neovim treesitter(injection.combined):https://neovim.io/doc/user/treesitter/

---

## 附:未能联网核实、仅基于训练数据或推导的结论

- **Volar `CodeInformation` 的具体字段名**(completion/navigation/semantic/verification 等 per-mapping 标记):官方文档页未展开细节,字段命名基于训练数据;"按映射携带能力标记"的机制本身已由文档确认。
- **VS Code 会把 workspace 内已打开文档的变更自动同步给对应语言的 LS(r-a)**:由 LSP/VS Code 文档同步机制推导,主流行为,但"用 WorkspaceEdit 驱动后台生成文档→r-a overlay 更新"的完整链路需 S3 首日冒烟验证。
- **overlay-only(不落盘)文件能否被 r-a 模块树正常解析**:架构推导,未见官方承诺,本方案已通过"落盘为主"规避,仅影响可选优化。
- **r-a 默认运行 build script 并索引 OUT_DIR 生成代码**(`cargo.buildScripts.enable` 默认开启):训练期稳定事实,低风险。
- **Dart build_runner `.g.dart` 生态类比、RustRover 使用自研引擎而非 r-a、JetBrains LSP API 仅对付费 IDE 开放**:训练数据;影响的仅是类比与"鸿蒙 DevEco/JetBrains 系编辑器"这一 open question 的背景描述。
- **lsp-server crate 为 r-a 团队维护的同步 LSP 脚手架**:训练期稳定事实,低风险。
