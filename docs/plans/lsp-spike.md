# `.svelte` IDE 体验(LSP)spike 方案

> 状态:方案 / 未开工。对应 DESIGN.md §6 风险清单第 1 条(`.svelte` 的 IDE 体验是编译器路线
> 转正的最大悬置)与 §5 "M4 遗留独立项 · LSP(及格线 9–16 人周,调研 07)"。
>
> 本文**不重写调研 07**([`docs/research/07-sv-ide-lsp.md`](../research/07-sv-ide-lsp.md)),
> 只做一件事:把它落到**本仓库 2026-07-22 的真实现状**上,并用实测替换掉其中三处
> 关键的推导性假设。所有数字带出处;凡未实测的一律标"未核实"。
>
> 实测环境:Windows 11 / rustc 1.88.0 (6b00bc388 2025-06-23) /
> rust-analyzer 0.3.2971-standalone (ffcdbbd906 2026-07-13,取自 VS Code 扩展自带二进制
> ——本机 stable 工具链未装 `rust-analyzer` component,`rustup` 直接报
> `Unknown binary 'rust-analyzer.exe'`,CI 要显式 `rustup component add`)。
> 复现命令见 §8。

---

## 0. 裁决先行

1. **rust-analyzer 对 `OUT_DIR` 生成文件的支持,比调研 07 的措辞好,但好在的地方与
   本方案初稿写的不是同一处**(见 §10 复核记录 R1/R2,已按实测改写):
   - **真·好消息(复核新增实测)**:对 `include!(concat!(env!("OUT_DIR"), …))` 进来的
     生成文件,r-a 的 **hover / goto definition 直接可用**——`didOpen` 生成文件后
     hover 返回 `let count: i32`,goto 正常跳转,**不需要落盘**。这一条 M10 没测到
     (M10 用的是落盘文件),是 a1 成本的关键下修项。
   - **必须修正的一条**:M8 观察到的"发布到生成文件 URI 的诊断",实测 `source` 字段
     是 **`"rustc"`**,且与 `rust-analyzer/flycheck/0` 的 progress end 同时到达;
     把 `check.enable=false` 后,生成文件 URI 上的诊断**一条都不剩**(而 `src/main.rs`
     上 r-a 自己的 `source="rust-analyzer"` 诊断照常在 1.0 s 内到达)。
     **即:那是 r-a 代跑的 `cargo check` 的输出,不是 r-a 自己的语义分析。**
     调研 07 §4 本来就写了"r-a 的语义诊断与它代跑的 flycheck 都发布到生成文件 URI",
     所以 M8 是**证实**而不是推翻;真正被推翻的是它的另一半("r-a 原生诊断也会来,
     注意去重")——实测生成文件上**没有** r-a 原生诊断。
   - 因此:**a0 的数据源 = `cargo check` 的 JSON = P1 已经在解析的同一份东西**,
     只是由 r-a 代跑、在保存时触发。这不改变推荐路线,但把 a0 的边际价值从
     "IDE 支持观感的 80%" 下调,见 §5 与 §6 的修订。
2. **调研 07 §3.1/§3.3 提出的是一套 codegen 纪律("script 逐字搬运 + 模板表达式逐字
   出现"),本仓库当前 codegen 没有遵守它**——这不是调研 07 的"乐观假设",是我们的
   实现选择与它的建议分岔了。runes 源变换会改写 script 与模板表达式
   (`count` → `count.get()`、`count += 1` → 两条语句)。所以**不存在"连续偏移差恒定
   的逐字区"**,映射只能做到 **token 级散点 + 节点级包络**。
   **但初稿在这里搞错了两件事,已按实测改正**:
   - `extract_props`(`script.rs:237`)**不制造偏移漂移**:它做的是**字节等长空白替换**
     (`script.rs:347-357`,注释原文"字节等长空白替换(保留换行;多字节字符按字节数填,
     行列不漂移)")。只有 `replace_runes` 漂,而且每处恒定 **+4 字节**(6 个 rune 全是
     `$xxx` → `__sv_xxx`)。漂移表因此是一张 +4 计数表,不是通用编辑表。
   - **真正的坏消息在别处、而且更致命**:`script.rs` 的 `Rewriter` 用
     `format_ident!("{name}")` 重造用户变量名(`:884` `.set()`、`:899` `.update()`、
     `:1038` `.get()`),重造出来的 Ident 是 **`Span::call_site()`**,即
     `line=1, byte_range=0..0` —— **恰好等于本方案自己定义的"胶水"判据**。
     也就是说:**每一次对反应式变量的读/写(`.svelte` 里最常见、也最常出类型错误的
     那个 token),provenance 会被无声丢弃**。全示例实测 98 处。详见 §3.2 第零步。
3. **`prettyplease 重排版之后行号还对不对得上` 这个真问题,答案是:不要在格式化之前建
   映射。** 做法:格式化后把输出文本**重新 parse 一遍**,拿到每个 token 在**输出文本**里的
   精确字节区间,再与格式化前的 token 流做**锚点并行走**(实测 M1/M2/M5:CJK 长行被
   prettyplease 折成 4 行,映射毫发无损)。
4. **推荐路线:§5 的 (a),并拆成 a0/a1 两档**(路线判断经复核保留,分档内容按实测改了)。
   a0 = 沿用现有 OUT_DIR,只做"诊断搬运"(零 r-a 实例、零虚拟文档、零 LSP server);
   **a1a = 不落盘、直接对 OUT_DIR URI 做正反向转发,换 hover/goto**
   (复核实测 M8c:r-a 在 OUT_DIR 生成文件上 hover/goto 本来就能用,落盘不是前提);
   落盘降级为可选的 **a1b**,买的是可读性不是能力。
   **(c) 完整 Volar 式虚拟文件明确不做**;(b) 模板域薄 LSP 是 a1 的附加项,
   不是替代品——它救不了"`.svelte` 里的 Rust 表达式没有 IDE"这个真痛点。
5. **第一步不是 LSP,是把编译器域与 rustc 域的错误都变成 `.svelte` 坐标**。理由不是保守,
   是杠杆:今天 `.svelte` 用户写错一个类型,错误落在
   `target/debug/build/<pkg>-<hash>/out/counter.rs:44:44`(复核实测的真实坐标);
   写错一个模板/CSS,错误埋在 build.rs 的 **panic dump** 里。两条都要搬回
   `Counter.svelte:12:38`,是整条路线上单位成本体感最高的一步,100% 在我们自己的代码里,
   可被 `cargo test` 验收。
   **复核修订:这一步不是 1–2 人周,是 P-1(0.2–0.4)+ P0(2–3.5)+ P1(1.5–2.5)。**
   把 `build()` 的 panic 换成 `cargo::error=`(P-1)是其中最便宜、当天就能交付的一块,
   应该第一个做。

---

## 1. 现状核实(读源码 + 实测,不是转述)

### 1.1 编译管线的形状

`compile_sv_with`(`lib.rs:120`)四步:`sfc::split` → `script::transform` →
`template::parse` → `codegen::generate`。产物由 `build()`(`lib.rs:178`)写到
`$OUT_DIR/<fn_name>.rs`(`lib.rs:213`),示例侧 `include!(concat!(env!("OUT_DIR"), "/counter.rs"))`
(`examples/counter-sfc/src/main.rs:9`)。

| 事实 | 位置 | 对映射的含义 |
|---|---|---|
| 模板表达式带 .svelte 字节偏移 | `template.rs:36` `ExprSrc { src, offset }` | **provenance 的原料已经在了** |
| 30 处模板表达式走 `self.expr(` 这一个入口 | `codegen.rs:200` `parse_expr` / `:207` `expr`(`grep -c "self.expr("` = 30) | 但**它不是唯一的用户文本 parse 入口**,见下一行 |
| **还有 6 处 `parse_str` 直接吃用户文本** | `codegen.rs:267`(`$props` 类型)、`:1021/:1081/:1112/:1388/:1403`(`bind:` 目标)、`:1688`(`{#each}` 模式 `Pat::parse_single`) | 「插桩只改一个函数」**不成立**;这些走的是无 pad 的裸 `parse_str`,其 token 的 `line` 恒为 1 → 会被本方案的判据**误判为胶水而静默丢弃** |
| `proc-macro2` 已开 `span-locations` | `crates/sv-compiler/Cargo.toml`(锁定 1.0.106) | 不用加依赖;`script.rs:454` `syn_err` 已经在用它反算行列(**既有先例**) |
| script 块**整块一次** parse | `script.rs:367` `format!("{{\n{pre}\n}}")` + `parse_str` | 注意 wrapped 前缀是 `{\n` = **2 字节**,pad 之外还要减这 2 |
| parse 前的两次纯文本改写里,**只有一次会漂** | `script.rs:404` `replace_runes`(6 个 rune 一律 `$xxx`→`__sv_xxx`,**恒 +4 字节/处**);`script.rs:237` `extract_props` **字节等长空白替换**(`:347-357`) | 漂移表 = 一张 "+4 计数表";`extract_props` 可当恒等处理(**初稿此处写错,已改**) |
| **`Rewriter` 用 `format_ident!` 重造用户变量名** | `script.rs:884` `.set()`、`:899` `.update()`、`:1038` `.get()` | **provenance 在这里被丢掉**(call_site);全示例 98 处。这是本方案最大的实现缺口 |
| 生成代码经 `syn::parse2` + `prettyplease::unparse` | `codegen.rs:126-134` | 格式化前的 token 流可拿到;格式化后位置要靠重解析 |
| **元素与文本节点**用递增唯一名 `__{prefix}{n}` | `codegen.rs:195` `fresh`,只有两个调用点:`:318` `fresh("t")`、`:503` `fresh("el")` | 哨兵**只覆盖元素/文本节点**;`{#if}`/`{#each}`/`{#key}`/组件调用/script 语句**没有哨兵**(初稿写"每个节点"是错的) |
| 生成文件唯一注释是文件头一行 | `codegen.rs:132` | 没有任何锚点注释,"跳进生成文件看" 目前靠猜 |
| 编译错误只在**编译器域**自报 .svelte 行列 | `lib.rs:44` `CompileError`、`:69` `line_col`(**列是字符数,不是字节**) | 模板/CSS/runes 错误已经是好体验;rustc 错误是零体验 |
| 编译器域错误经 **`panic!`** 冒出来 | `lib.rs:207` `panic!("\n\n.svelte 编译失败\n  --> {e}\n")` | 它**不进 cargo 的 JSON 流**,只在 cargo stderr 的 panic dump 里,`sv check` 必须单独处理(§4.1 已补) |

**结论:现在没有任何 span 映射(生成代码 → .svelte),这一条与任务描述一致,已核实。**

### 1.2 生成产物的规模(实测 M9)

对全部 11 个 `.svelte` 跑 `compile_sv`(release,50 次取均值):

| 文件 | .svelte 行 | 生成行 | 生成字节 | 编译耗时 |
|---|---|---|---|---|
| Card.svelte | 13 | 73 | 2 576 | 0.47 ms |
| Counter.svelte | 23 | 180 | 6 382 | 2.41 ms |
| TodoItem.svelte | 23 | 172 | 6 372 | 1.84 ms |
| InputDemo.svelte | 63 | 394 | 14 530 | 5.06 ms |
| Settings.svelte | 57 | 476 | 18 906 | 5.25 ms |

两个可直接用于决策的数:**生成放大 5.6–8.4×(行)**;**单文件编译 0.4–5.2 ms**
(复核独立复现:0.44 / 1.65 / 1.56 / 3.35 / 3.67 ms,同量级、系统性偏低约 30%;
token 与锚点数完全一致:Counter 生成 180 行 / 1017 token / 425 锚点)。

后者意味着**编译器不是 IDE 路径上的瓶颈**,"要不要做增量编译器"这个分心项
**第一年不需要**。但这句话的适用范围要收窄(复核修订):

- 真正决定"保存 → 看到诊断"的不是 sv-compiler,是 **build.rs 重跑之后 cargo 必须重编
  整个叶子 crate**。复核实测(TEMP 里 counter-sfc 的等价工程,依赖只有 sv-reactive +
  sv-ui):改一次 `.svelte` → `cargo check` 墙钟 **242–261 ms**;空跑 129 ms;
  只改 `main.rs` 204 ms。**真实 app 带上 sv-shell(vello/parley/taffy/winit)会更慢,
  未核实**。
- 而且**诊断这条路根本吃不到 keystroke 新鲜度**:r-a 在生成文件上只转发 flycheck
  (§1.3 修订),flycheck 是**保存触发**的。keystroke 级新鲜度只对 hover/goto 有意义
  (那两个走 r-a 自己的分析,didChange 即时生效)。

### 1.3 rustc 与 rust-analyzer 今天怎么看待生成文件(实测,最重要的一节)

在 `%TEMP%` 里搭了一个最小复现工程(build.rs 写 `$OUT_DIR/counter.rs`,`main.rs` 里
`include!`),故意埋两个错(`count.nope()` 与未定义的 `NotAThing`,后者放在 `println!` 参数里):

- **M6 · rustc JSON**:诊断的 `spans[].file_name` = **生成文件的真实路径**,带
  `line_start/column_start` **与** `byte_start/byte_end`,且 **`expansion: null`**
  ——**即使错误发生在 `println!` 的参数里**。含义有三:
  ① `include!` 不制造二级 span,"生成文件坐标 = 最终坐标"成立;
  ② 我们可以**只用字节偏移**做重映射,绕开 UTF-8/UTF-16 列的全部麻烦;
  ③ `rendered` 字段里嵌着生成文件路径与生成代码摘录,**必须重渲染,不能透传**。
- **M7 · OUT_DIR 可发现**:`cargo check --message-format=json` 会先吐一条
  `{"reason":"build-script-executed", …, "out_dir":"…\\build\\incl-bd87ebdbc5767025\\out"}`。
  于是 `sv check` 不需要猜 hash 目录,**顺流读一条消息就知道生成文件在哪**。
- **M8 · rust-analyzer(LSP 层,不是 CLI)**:自建 LSP client 探针(§8 有脚本)
  `initialize` → `didOpen(src/main.rs)`,r-a 主动推来:

  ```
  URI: file:///…/target/debug/build/incl-bd87ebdbc5767025/out/counter.rs
     E0425 {start:{line:4,character:19}, end:{line:4,character:28}} 'cannot find value `NotAThing`…'
     E0599 {start:{line:3,character:18}, end:{line:3,character:22}} 'no method named `nope`…'
  ```

  **诊断被发布到生成文件自己的 URI,行列(0-based)在生成文件里完全正确。**
  (同一工程用 `rust-analyzer diagnostics .` CLI 也能查出这两个错,但 CLI 打印的
  文件名是被扫描的根文件而非诊断所属文件,容易误读——**以 LSP 层的结果为准**。)

- **M8b · 这些诊断是谁产的?(复核补测,推翻初稿的解读)** 同一探针加打 `source` 字段
  并做对照实验:

  | 配置 | 生成文件 URI 上的诊断 | `src/main.rs` 上的诊断 | 生成文件上的 hover/goto |
  |---|---|---|---|
  | 默认 | 2 条,**`source="rustc"`**,与 `rust-analyzer/flycheck/0` 的 progress end 同时到达(冷 +10.1 s / 热 +3.9 s) | — | ✅ |
  | `check.enable=false` + `checkOnSave=false` | **0 条**(显式 `didOpen` 生成文件、等 55–70 s 仍为 `n=0`) | r-a 原生 `source="rust-analyzer"` E0308,**+1.0 s** | ✅ `let count: i32` |
  | `check.enable=false`,生成文件改为源码树内真 `mod`(落盘形态) | **仍是 0 条** | 同上 | ✅ |

  **结论(必须写进方案):生成文件 URI 上的诊断 100% 来自 r-a 代跑的 `cargo check`,
  不是 r-a 自己的语义分析;r-a 的原生诊断在生成文件上一条都不发**(include! 形态与
  落盘 mod 形态都不发,原因未查明,列入 §8 未核实清单)。含义有三:
  ① **a0 与 P1 消费的是同一份数据**(cargo check JSON),a0 省掉的只是"自己 spawn cargo";
  ② 诊断永远是**保存级**、秒级延迟,keystroke 级诊断在这条路上不存在;
  ③ 调研 07 §4 担心的"r-a 原生诊断与 flycheck 重叠要去重"**不需要处理**。

- **M8c · 真正的好消息在 hover/goto(复核新增)**:对 `include!(concat!(env!("OUT_DIR"),…))`
  进来的生成文件,`didOpen` 后 `textDocument/hover` 返回 ```` ```rust\nlet count: i32\n``` ````、
  `textDocument/definition` 正常返回 `LocationLink`。**这说明 r-a 确实把 OUT_DIR 生成文件
  当真文件索引了,而且不需要落盘**——这直接下修 a1 的成本(§5 修订)。

**关于调研 07 的两处引用要更正**(初稿引错了小节,结论也过强):

- 给 OUT_DIR 打"二等"标签的是调研 07 **§2.2**(理由:路径含 hash、flycheck 有 #13520
  类历史问题、诊断指向 target/ 深处),不是 §2.1;而且它同一句里就写了
  "**r-a 默认运行 build script 并索引 OUT_DIR 产物**"——它从没说 r-a 不索引。
  它列的"二等"理由中,"路径含 hash / 用户永远不会打开"这一条**本方案自己也承认**,
  所以"过度悲观"这个判词收回。
- 调研 07 **§2.1** 的 open question 是 **overlay-only 文件**(磁盘上不存在、只靠 `didOpen`
  推给 r-a)能否被模块树接收。M8 测的是**磁盘上真实存在**的 OUT_DIR 文件,
  **完全没有触及这个 open question**。它依然未核实,而且只在"编辑期不落盘、
  纯内存刷新"这条路上才需要回答(§7 卡死点 3 的止损方向正好依赖它)。

---

## 2. 问题一:最小可用切片(排序与理由)

判据不是"这个特性有多爽",而是 **"它需要几个方向的映射 + 需不需要改协议"**。
按此排序,顺序是天然的:

| # | 特性 | 需要的映射方向 | 需要新组件 | 体感 | 裁决 |
|---|---|---|---|---|---|
| 0 | **语法高亮/折叠**(TextMate injection) | 无(不走 LSP) | 无 | 从"纯文本"到"正常语言" | **并行做掉,不占 LSP 预算** |
| 1 | **诊断重映射** | 仅**反向** gen→sv | `sv check` CLI(+ 可选扩展) | 最大 | **P0** |
| 2 | **悬停类型 hover** | **正向** sv→gen(结果文本不用回映) | VS Code 扩展 | 大 | **P1** |
| 3 | **跳转定义 goto** | 正向 + 结果**反向**(目标落在生成文件时要改指 .svelte) | 同上 | 中 | P2 |
| 4 | **补全 completion** | 正向 + `TextEdit.range` 反向 + `filterText`/触发字符校正 + `resolve` | 同上 | 中 | P3 |
| 5 | auto-import / rename / assist | 写操作,多文件 `WorkspaceEdit` 回插 | — | — | **不做**(与调研 07 同) |

**与任务里给的候选顺序的两处分歧,以及理由:**

- **悬停排在跳转之前**(任务提示里是"跳转定义 > 补全 > 悬停类型")。理由:hover 是**唯一
  只需要单向映射**的特性——把光标位置换到生成文件、把 r-a 返回的 markdown 原样弹出来
  就完事,`Hover.range` 丢掉都不影响可用性。goto 则必须处理"目标在生成文件里"这个情况
  (跳到 `target/.../out/counter.rs` 是**负体验**,比不跳还糟),那需要反向映射先就位。
  实现顺序应当跟着"映射方向的依赖"走,而不是跟着直觉的重要性走。
- **语法高亮不参与排序**。它不是 LSP 特性,不共享任何管线,风险独立为零,应该在第一天
  和 P0 并行开工。把它混进 LSP 优先级表会制造"我们在做 IDE 了"的错觉,而真正的悬置项
  (Rust 表达式的类型信息)一步没动。

**为什么诊断压倒一切**:因为它是**唯一一个在"没有 LSP"时也能交付的 LSP 级体感**
(CLI 形态即可),而且它的服务对象不止编辑器——CI、`cargo test` 失败信息、新人第一次
写错 `.svelte` 时看到的东西,全是它。调研 07 §7 的判断"没有 LSP 时,诊断质量就是 DX 的
全部下限"在本仓库尤其成立,因为**模板域/CSS 域/runes 域的错误已经是精确的 .svelte 行列**
(`lib.rs:44`),唯独 rustc 域是断的——补上这一块,`.svelte` 的错误体验就**整体**及格了。

---

## 3. 问题二:span 映射怎么做(核心)

### 3.1 三个必须先承认的事实

1. **`quote!` 造出来的 token 没有位置**,`prettyplease::unparse` 从 `syn::File` 重新打印,
   **spans 在输出这一步全部丢弃**。所以"在格式化前算好行号"是死路。
2. **proc-macro2 的 fallback span 无法区分不同次 `parse_str`**(实测 M3):
   `parse_str("count * 2")` 里的 `count` 与 `parse_str("other + count")` 里的 `other`
   **都报 `byte_range = 0..5`、`start = (1,0)`**。所以"从 span 反查它属于哪个表达式"
   不成立,**provenance 必须由 codegen 自己带**。
3. **`prettyplease` 不是恒等打印**(实测 M1):`quote!{ g(a, b,); }` 的尾逗号被吃掉,
   于是**全 token 序列前后不等(73 vs 71)**,但 **Ident/Literal 序列完全相等(28 vs 28)**。
   → 并行走**只能走 Ident/Literal**,标点一律跳过。

   **复核修正(M1 的结论对,但取样不代表真实产物)**:73 vs 71 是手写 `quote!` 里
   人为放尾逗号造出来的。在**本仓库 11 个真实 `.svelte` 的生成产物**上重跑
   `to_token_stream → unparse → parse_file → to_token_stream`,**全 token 序列也完全相等**
   (Counter 1017 vs 1017、Settings 2323 vs 2323、wide 3314 vs 3314,11/11 全等),
   而且 `unparse` 对生成文本**幂等**。所以"只走 Ident/Literal"是**保守选择而非必需**
   ——保守是对的(§7 卡死点 1 的保险丝要留),但不要把 73 vs 71 当成"prettyplease 会
   吃我们的 token"的证据写进文档,它在我们的 codegen 形状上从未发生。

4. **(复核补充,方案初稿完全漏掉)`byte_range()` 的语义已从 proc-macro2 1.0.106
   源码核实为真·字节**:`fallback.rs:346-424` 的 `FileInfo` 内部用 **char index** 记位置
   (`lines_offsets` 按 `chars()` 计数),但 `byte_range()` 经 `char_index_to_byte_offset`
   BTreeMap 转成**字节**再返回,且是**文件内相对**偏移。实测 CJK 用例
   (`"一二三四五六七八九十".len() + count`,pad=1/4)全部命中原串。
   **但 `Span::start().column` 是字符列,不是字节列**(`offset_line_column`)——本方案
   只用 `.line`,别顺手用 `.column`。
5. **(复核补充)`SOURCE_MAP` 是永不回收的 thread_local**:每次 `parse_str`/`parse_file`
   都往里加一个持有完整 `source_text` 的 `FileInfo`,没有公开的清理点除了
   `proc_macro2::extra::invalidate_current_thread_spans()`(它会 `truncate(1)`,
   之后再碰旧 span 会走到 `SourceMap::find` 的 `unreachable!` **直接 panic**)。
   build.rs 一次性进程无所谓;**a0/a1 那个长驻进程里按 keystroke 反复编译就是单调泄漏**。
   → 长驻形态必须二选一:每次编译 fork 子进程,或在"确认无存活 span"的边界上调
   `invalidate_current_thread_spans()`。**这也是本仓库"去 panic 纪律"的一个新暴露面。**

### 3.2 机制:虚拟行号 provenance + 格式化后锚点并行走

分**四**步(初稿三步,复核在前面加了一步"止血",没有它后面三步等于白做),
其中第一~三步已在 `%TEMP%` 里单独跑通(§8 复现)。

**第零步 · 先把被 `Rewriter` 丢掉的 provenance 找回来(复核新增,P0 的必要前提)**

`script.rs` 的 `Rewriter` 在三处用 `format_ident!("{name}")` **重造**用户变量名:

| 位置 | 改写 | 重造出来的 span |
|---|---|---|
| `script.rs:883-884` | `x = v` → `x.set(v)` | `Span::call_site()` |
| `script.rs:897-899` | `x += v` → `{ let __sv_rhs = v; x.update(…) }` | `Span::call_site()` |
| `script.rs:1037-1038` | 裸读 `x` → `x.get()` | `Span::call_site()` |

`Span::call_site()` 在 fallback 下就是 `line=1, byte_range=0..0`,**恰好是本方案用来
判定"胶水"的那个值**。复核实测(复刻 `script.rs:1037` 的最小用例):

```
改写前: [ count  line=4 br=3..8 ]        ← 有 provenance
改写后: [ count  line=1 br=0..0 ]        ← 没了,被判为胶水
修法后: [ count  line=4 br=3..8 ]        ← syn::Ident::new(name, orig.span())
```

**规模**:8 个 `.svelte`(Counter/InputDemo/Dialog/Settings/Stepper/TaskRow/TodoItem/wide)
的生成产物里,`.get()`/`.set()`/`.update()` 形态的用户变量引用共 **98 处**,
**全部丢失 provenance**。Counter.svelte 一个文件就有 9 处(生成行 7/9/44/46/92/117/137/142/160),
而 Counter.svelte 里用户写的 `count`/`double` 引用**总共就这么多**。

**修法**:`path_single_ident` 顺带返回原 Ident 的 `Span`,三处 `format_ident!` 换成
`syn::Ident::new(&name, orig_span)`。`parse_quote!` 对插值 token 的 span 是保留的,
所以只需要改这一个 token 的构造方式。**成本 < 半天,收益是让整个 P0 有意义。**

> **为什么这个坑必须写在这里而不是留给实现者踩**:`map_roundtrip_all_examples`
> (§6.2 第 1 条)**抓不到它**。那条断言检查的是"已记录的段两侧文本相等"——
> 丢失 provenance 的 token 压根不会被记录,断言恒真。这是**soundness 测试冒充
> completeness 测试**,见 §6.2 的修订。

**第一步 · 给用户 token 打上可区分的来源**

**注意:不止 `codegen.rs:200` 一处**。`codegen.rs` 里还有 6 个直接吃用户文本的
`parse_str`(`:267` `$props` 类型、`:1021/:1081/:1112/:1388/:1403` `bind:` 目标、
`:1688` `{#each}` 的 `Pat::parse_single`),`script.rs` 还有 2 个(`:327` `$props` 字段、
`:369` script 块)。**每一个都要走同一套 pad 分配**,否则它们产出的 token `line` 恒为 1
(单行输入),会被判据当成胶水**静默丢弃**——又是一个 roundtrip 测试抓不到的洞。

不要直接 `syn::parse_str(&e.src)`,而是**前面垫 N 个换行**再 parse:

```rust
// 伪码,示意签名变化;实际实现进 Cg,用 Cell/RefCell 存表避免 &mut self 借用冲突
fn parse_expr(&self, e: &ExprSrc) -> Result<syn::Expr, CompileError> {
    let pad = self.map.alloc_lines(e.src.lines().count().max(1)); // 单调递增,从 1 开始
    let expr = syn::parse_str(&format!("{}{}", "\n".repeat(pad), e.src))?;
    self.map.record_site(pad, e.offset, e.src.len());            // 虚拟行段 → .svelte 偏移
    Ok(expr)
}
```

实测 M4 保证了这一步的正确性:`pad=4` + `"if a {\n  b\n} else { c }"` 时,
`if`/`a` 报 `line=5`、`else` 报 `line=7`,而 **`byte_range.start - pad` 恰好等于
token 在原表达式串里的偏移**(0 / 3 / 13)。于是:

- **表达式归属** = `span.start().line - 1` 落在哪个已登记的虚拟行段;
- **表达式内偏移** = `span.byte_range().start - pad`;
- **.svelte 偏移** = `ExprSrc.offset + 表达式内偏移`。

**合成 token 天然可分辨**:`quote!` 造的 token 一律 `line = 1`、`byte_range = 0..0`
(实测 M4)。把虚拟行号从 1 开始分配,`line == 1` 就是"胶水"的判据,零歧义。

**script 块同理但更简单**:它是**整块一次 parse**(`script.rs:367`),给整块分配一个虚拟
行段即可。要减的常量有两个:pad,**加上 `wrapped = format!("{{\n{pre}\n}}")` 的前缀 2 字节**。

**偏移漂移表只需要处理 `replace_runes` 一个来源**(复核更正):`extract_props`
(`script.rs:347-357`)做的是**字节等长空白替换**,源码注释原文"字节等长空白替换
(保留换行;多字节字符按字节数填,行列不漂移)",**字节偏移恒等,不需要建表**。
`replace_runes` 的 6 个 rune 全是 `$xxx` → `__sv_xxx`,**每处恒定 +4 字节**,
所以"漂移表"就是"这个位置之前有几个 rune 命中" × 4 —— 让 `replace_runes` 顺手返回
命中位置的 `Vec<usize>` 即可,连区间表都不用。
(注意 `extract_props` 的等长填充是**按字节**填的:一个 3 字节汉字变 3 个空格。
所以它保住的是**字节**偏移,**字符列会漂**——又一条"全程用字节"的理由。)

**第二步 · 格式化之后,重解析,锚点并行走**

```rust
let file: syn::File = syn::parse2(file_ts)?;      // codegen.rs:126 既有
let pre  = anchors(file.to_token_stream());        // Vec<(text, span)>,只收 Ident/Literal
let out  = prettyplease::unparse(&file);           // codegen.rs:133 既有
let post = anchors(syn::parse_file(&out)?.to_token_stream());
debug_assert_eq!(pre.len(), post.len());           // 失配 = 降级信号,见 §7
for (a, b) in pre.iter().zip(post.iter()) {
    if a.text != b.text { /* 降级 */ }
    // a.span → (哪个表达式, 表达式内偏移) → .svelte 偏移
    // b.span.byte_range() → 在**格式化后文本**里的精确字节区间
}
```

**为什么这解决了"prettyplease 重排版后行号对不对得上"**:因为我们**根本不预测**行号
——输出位置是从**输出文本自己**解析出来的(实测 M2:重解析 + span-locations 给出精确的
line/col/byte)。prettyplease 怎么折行、怎么把 `__doc.update_style(...)` 拆成 5 行、怎么把
一行 CJK 字符串独占一行,统统与我们无关。实测 M5 的端到端原型里,一行 28 个汉字的
`create_text(...)` 被折成 4 行,后面表达式的映射精确到字节:

```
gen-token    gen-pos     sv-offset   sv-text
count        L10 C28          200      count
2            L10 C42          208          2
count        L14 C20          300      count
1            L14 C34          308          1
```

**第三步 · 节点级包络(胶水区兜底)**

只有 token 级散点是不够的:rustc 的错误 span 经常落在胶水上,**也经常落在标点上**
(见 §3.4)。所以同时记录**包络区间**:每个模板节点、每条 script 语句在生成文件里的
字节区间 → .svelte 区间。

**初稿说"哨兵现成、零新增机制",复核实测:不成立,这一步是要写代码的。**

- `self.fresh(` 在 `codegen.rs` 里只有 **2 个调用点**:`:318` `fresh("t")`(裸文本节点)、
  `:503` `fresh("el")`(元素)。**`{#if}` / `{#each}` / `{#key}` / `{#await}` / 组件调用
  一个哨兵都没有**——`emit_component` 的产物就是 `#fn_ident(&__doc, __parent);`,
  没有任何 `let __xxN =`。
- "下一个 `__elN` 的首次出现即终点"这个启发式在真实产物上**会错**。实测
  Counter.svelte 的生成文件:`__el7`(归零按钮)首现于 L120,`__el8` 首现于 L145,
  而 **L142 的 `move || count.get() > 5` 是 `{#if}` 的条件**,按该启发式会被归给
  「归零按钮」。
- 同一启发式对**父元素**也是错的:`__el1` 的"终点"会被算成 `__el2` 首现处,
  但 `__el1` 是整棵子树的父节点,真实包络是整个函数体。**包络必须是真嵌套的,
  靠文本搜索凑不出嵌套。**

**正确做法(改小,但必须做)**:在第二步的锚点并行走里,codegen 侧同时压一个
**节点栈**——`emit_nodes`/`emit_element`/`emit_if`/`emit_each`/`emit_component` 进出时
记录"当前节点的 .svelte 区间",走到的每个 pre-token 都带上栈顶;并行走给出该 token 在
输出文本里的字节位置,于是每个节点的包络 = 它名下所有 token 输出位置的 min..max。
**这不需要哨兵,也天然嵌套。**

### 3.3 sidecar 格式与生成时机

**时机**:codegen 的最后一步,与 `.rs` **同一次写盘**(`lib.rs:213` 旁边多写一个文件),
文件名 `$OUT_DIR/<fn_name>.rs.map`。**理由**:调研 07 §8 说得对——"映射不是事后贴的,
是 codegen 的输出契约";但更实际的理由是**一致性**:map 与 .rs 必须同生同死,分两次生成
必然漂移。

**格式**(v1,刻意最小;不抄 Volar 的 per-mapping 能力标记,那是有了 4 个以上特性之后
才需要的东西):

```jsonc
{
  "v": 1,
  "sv": "src/Counter.svelte",
  "gen": "counter.rs",
  "sv_len": 1043, "gen_len": 6382,          // 廉价的一致性校验(比 hash 便宜,够用)
  "sv_hash": "fnv1a:3f2a…",                  // .svelte 内容 hash;不匹配 → 拒绝使用该 map
  // 精确段:token 级,按 gen 起点排序、互不重叠 → 二分查找
  // [gen_start, gen_end, sv_start, sv_end]
  "tokens": [[1180,1185,512,517], [1188,1189,520,521], …],
  // 包络段:节点/语句级,**可嵌套**,查找取最内层
  // [gen_start, gen_end, sv_start, sv_end, kind]  kind: 0=script 语句 1=元素 2=块 3=属性
  "spans":  [[900,1100,470,620,1], …]
}
```

- **`"sv"` 必须写绝对路径**(复核补充)。build.rs 的 cwd 是包根,`sv check` 的 cwd 是
  workspace 根,写相对路径两边对不上。build.rs 里 `path.canonicalize()` 一次即可;
  真要写相对路径,就得从 `compiler-message.package_id` / `manifest_path` 反查包目录
  (cargo JSON 里两个字段都有,已实测),没必要绕这一圈。
- **两侧全用字节偏移**,不用行列。理由是实测 M6:rustc JSON 直接给 `byte_start/byte_end`,
  查表零转换;行列只在最后输出给人看/给 LSP 时才算(LSP 还要 UTF-16,见 §7 坑 4)。
  **注意 rustc JSON 的 `column_start/column_end` 是 1-based 字符列**(复核实测:
  含 7 个 3 字节汉字的一行,同一位置 `byte=123` / `column=40` / LSP `character=39`),
  和 `lib.rs:69` `line_col` 的口径一致——**三种编码同时在场:字节(map)、字符(rustc/
  我们的 CompileError)、UTF-16(LSP)**。
- **粒度裁决:token 级(锚点级)为主,包络级兜底,不做"行级"**。行级看似便宜,实际上
  在本仓库是**错的**:prettyplease 会把一条语句摊成 5 行(见 §1.2 的 `update_style`),
  一行里也可能挤着两个不同来源的表达式。既然锚点并行走已经把 token 级做出来了,
  行级反而是多余的中间态。
- **体量**(实测):`Counter.svelte` 的生成文件(181 行)展平后 1017 个 token,其中
  **Ident+Literal 锚点 425 个**;真正来自用户源码的锚点是其中的少数(其余是
  `::sv_ui::` 路径、`__elN`、闭包骨架这些胶水)。→ map 是 KB 级,可以无脑随 `.rs`
  一起写盘、一起被 `rerun-if-changed` 逻辑管理。

### 3.4 这套机制**做不到**的事(必须写进文档,不能装看不见)

1. **runes 改写让"一个用户表达式"不再是一段连续生成代码**。实测(`Counter.svelte` 的真实产物):

   ```rust
   __doc.set_on_click(__el5, move || {
       let __sv_rhs = 1;                              // ← 用户写的 `1`
       count.update(|__v| *__v += __sv_rhs)           // ← `count` 与 `+=` 的残骸
   });
   ```

   用户写的是 `|| count += 1`。若 `1` 类型不对,rustc 会把错误标到 `__sv_rhs` 那一行。
   token 级映射能把 `1` 映回去,但**周边的 `count.update(...)` 是胶水**,一旦主 span 落
   在胶水上就只能退到包络。**这是 runes 隐式反应性的固有代价**(ADR-2 换来的东西),
   不是可以修的 bug,应当在文档里明说,并在诊断输出里诚实呈现为"表达式级"。
2. **样式没有 provenance**。样式值在编译期就折叠成字面量——实测 Counter.svelte 的产物里
   确实是 `s.bg = Some(::sv_ui::Color::rgba(255u8, 62u8, 0u8, 255u8));`,
   `s.font_size = 20f32;`,全是合成 token。
   (复核更正:这不止发生在 `<style>` 块,**内联 `style="..."` 属性同样折叠**,
   而本仓库示例用内联样式远多于 `<style>` 块——Counter.svelte 根本没有 `<style>` 块。)
   **这没关系**:样式域的错误本来就由编译器自报 .svelte 行列,rustc 永远看不到它们。
   (复核更正:初稿写的"`css_c1_*` 测试族已覆盖"夸大了——全仓 CSS 相关测试只有
   `css_c1_box_model_vars_nesting` 与 `css_compat_names_units_hover` **两个**,
   不是一个"测试族"。样式域诊断的覆盖面本身就是 §7 止损后值得加厚的方向之一。)
3. **列精度止步于 token**。rustc 有时把 span 划在子表达式的一部分(如 `.to_string()` 的
   方法名),映回去只能定位到最近的用户 token。可选的廉价补救:用 rustc JSON 的
   `spans[].text[].text`(那一行的原文)在 .svelte 的表达式源码里做一次唯一匹配来收窄列
   ——**是启发式,匹配不唯一就放弃**,不要为了几个列位置引入不可解释的行为。
4. **(复核新增,而且这条打到了本方案自己的招牌 demo)主 span 经常落在标点上,
   而标点不在 `tokens[]` 里**。实测:把 §6.1 的示例错误注入真实 `Counter.svelte`
   (`{count}` → `{count + "x"}`)后跑 `cargo check --message-format=json`,得到

   ```
   code=E0277  cannot add `&str` to `i32`
     primary  …/out/counter.rs  L44 C44-45  bytes 1445-1446
       > __s.push_str(&(count.get() + "x").to_string());
   ```

   **主 span 宽度 1,就是那个 `+`**。按 §3.2 "标点一律跳过" 的规则,它在 `tokens[]`
   里查不到 → 落到包络 → `kind=approx` → 按 §4.2 只能报到元素级,
   **报不出 §6.1 承诺的 `Counter.svelte:12:38`**。
   **补法(必须写进 P0)**:查表未命中 `tokens[]` 时,先做**相邻锚点插值**——
   取生成侧 gen_start 左右最近的两个 token 段,若二者的 .svelte 区间同属一个表达式且
   顺序一致,则按"到左锚点的字节距离"平移出 .svelte 位置。对 `count.get() + "x"` 这种
   `左锚点=count`(第零步修好之后才有)、`右锚点="x"` 的情形,`+` 能被精确插值回
   `.svelte` 的 `+`。**这条与第零步是绑定的:第零步不做,左锚点就是胶水,插值也救不回来。**

---

## 4. 问题三:`sv check` 的数据流与失败模式

### 4.1 数据流

```
sv check [--json|--github] [cargo 参数透传]
  │
  ├─ 1. spawn: cargo check --workspace --message-format=json  (原样透传用户的 features/target)
  │
  ├─ 2. 流式读 JSON-lines:
  │      reason=build-script-executed → 记下 out_dir           (实测 M7:字段确实存在)
  │      reason=compiler-message      → 进第 3 步
  │
  ├─ 3. 对每条诊断的每个 span:
  │      file_name 是否命中 <out_dir>/<name>.rs 且旁边有 <name>.rs.map?
  │        否 → 原样透传(普通 .rs 的错误不该被我们碰)
  │        是 → 载入 map(带 hash 校验)→ 用 byte_start/byte_end 查表:
  │               命中 tokens[]  → 改写为 .svelte 精确区间          (kind=exact)
  │               命中 spans[]   → 收敛到最内层包络的 .svelte 区间   (kind=approx)
  │               都没命中       → **降级**,见 4.2             (kind=unmapped)
  │
  ├─ 4. 递归处理 children[](note/help 子诊断)与 suggested_replacement
  │
  └─ 5. 输出:human(用 .svelte 原文**重新渲染**)/ json / github-annotations
         退出码:有 error → 1;仅 warning → 0(`--deny-warnings` 可改)
```

**(复核新增)第 0 条数据流分支:编译器域错误根本不进 JSON 流。**
`.svelte` 的模板/CSS/runes 错误走 `lib.rs:207` 的 `panic!`,实测(注入 `fg="#zzz"`)结果是:

```
JSON 流:  本包既没有 compiler-message,也没有 build-script-executed(只有依赖包的)
cargo stderr:
  error: failed to run custom build command for `svdemo v0.0.0 (…)`
  Caused by: process didn't exit successfully: …build-script-build (exit code: 101)
    --- stderr
    thread 'main' panicked at E:\WorkSpaces\svelte-rs\crates\sv-compiler\src\lib.rs:207:23:
    .svelte 编译失败
      --> src\Counter.svelte:19:15: 属性 `fg`:颜色 `#zzz` 不是合法十六进制
    note: run with `RUST_BACKTRACE=1` …
```

含义:① §4.1 第 2 步"记下 out_dir"在这条路径上**拿不到 out_dir**;
② 用户看到的是 panic dump 里夹着的一行,还带着我们自己的内部文件路径;
③ `sv check` 要么去 scrape panic 文本(脆),要么**先把 `build()` 的 panic 换掉**。

**推荐先做后者(成本 ~0.1 人周,收益立刻可感知,列为 P-1)**:build.rs 用
`println!("cargo::error=<sv路径>:<行>:<列>: <消息>")` + 非零退出。复核实测输出:

```
error: svdemo@0.0.0: src/Counter.svelte:19:15: 属性 `fg`:颜色 `#zzz` 不是合法十六进制
error: build script logged errors
```

干净、一行、`文件:行:列: 消息` 直接能被 VS Code problemMatcher 正则吃掉,
而且**去掉了一个 panic**(合本仓库去 panic 纪律)。
**但要诚实**:复核实测 `cargo::error` **不会**进 `--message-format=json` 的
`compiler-message`(JSON 流里一条都没有),它只上 stderr。所以 `sv check` 仍然要
同时读 stderr;只是从"解析 panic dump"降级成"匹配一条规整的 error 行"。

**路径匹配的两个 Windows 坑(实测 M6 里就能看到,复核已复现)**:rustc 吐出来的路径是
`C:\…\out/counter.rs` ——**反斜杠与正斜杠混用**(前半段来自 cargo,`/counter.rs`
来自 `include!` 的字面量拼接)。必须先规范化再比较;直接字符串相等**一定**匹配不上。
其次,同一个包在 `target/debug/build/` 下可能残留多个 `<pkg>-<hash>/out` 目录(旧的构建),
**只认本次 cargo 运行吐出来的 `out_dir`**,不要 glob。

### 4.2 失败模式(每条都必须"降级"而不是"丢")

**铁律:输入 N 条诊断,输出必须 N 条。**"我映射不了所以我不说了"是最坏的失败——用户会
以为编译通过了。对应验收测试 `check_never_drops_diagnostic`(条数守恒断言)。

| 失败模式 | 触发条件 | 降级行为 |
|---|---|---|
| **查不到映射段** | 主 span 落在纯胶水(如 `bind_text` 闭包骨架) | 输出**保留原诊断全文**,位置指向 **`<组件名>.svelte`(整文件)**,附一行:`该错误落在 sv-compiler 生成的胶水代码上(<gen路径>:<行>:<列>),通常是编译器 bug,请附 .svelte 源码上报` |
| **map 文件缺失/过期** | `.svelte` 改了但 build.rs 没重跑;或 hash 不匹配 | **完全不重映射**,原样透传 rustc 诊断(指向生成文件)+ 一条 warning:`未找到与 <gen> 匹配的 span map,诊断保持生成文件坐标` |
| **同一诊断多 span** | rustc 的 `expected due to this` 等多标注 | **逐 span 独立映射**;主 span 能映、次 span 不能 → 主 span 用 .svelte 坐标,次 span 降级为 note 文本(不硬凑位置) |
| **suggestion 落在胶水区** | `suggested_replacement` 的区间不在 tokens[] 内 | **丢弃该 suggestion**(不是丢诊断)。理由:把胶水代码的修复建议应用到 .svelte 会**毁用户源码**,这是唯一一处"丢"是正确的 |
| **诊断跨越多个映射段** | 错误 span 覆盖 `用户表达式 + 胶水` | 取**包络**(最内层 spans[]),标 approx;不要拼接不连续的 .svelte 区间 |
| **`rendered` 字段** | 永远 | **重渲染**。透传等于把 `target/.../out/counter.rs` 的路径与生成代码摘录甩给用户,比不重映射还糟 |
| **同一个 .svelte 被多个 crate 编译** | workspace 里两个包引用同一目录 | 按 `out_dir` 分别处理,输出去重(key = .svelte 路径 + 区间 + 消息) |
| **cargo 本身失败**(链接错误等) | 无 compiler-message | 原样透传 cargo 的 stderr,退出码跟随 |
| **build.rs 失败(= `.svelte` 编译器域错误)** | 见 §4.1 第 0 分支 | 本包无 `build-script-executed`;从 stderr 抓 `<sv>:<行>:<列>: <消息>` 输出为一等诊断。**这条最常见**——用户改 `.svelte` 时语法错的频率远高于类型错 |
| **缓存命中时诊断是否还在** | 第二次 `cargo check` 什么都没重编 | **不用处理**:复核实测 cargo 会**重放**缓存的 `compiler-message`(warning)与 `build-script-executed`,两次跑输出一致;error 因为单元从不 fresh,必然重发 |

### 4.3 与编辑器的关系

同一个 `sv check` 同时是:CI 的红绿闸、VS Code task + problemMatcher 的数据源、
bacon/cargo-watch 的被调用者。**这是"没有 LSP 的第一年"的主力诊断通道**(调研 07 §7),
也是 P3 那个 VS Code 扩展的**逻辑复用来源**——扩展做的事情是同一套重映射,只是输入从
`cargo check` 的 JSON 换成 r-a 已经发布的诊断(实测 M8)。

---

## 5. 问题四:三条路的评估与推荐

### (a) 生成文件是一等产物 + r-a 正常索引 + 我们做位置转发

**因为 M8,这条路要拆成两档,成本天差地别。**

**a0 · 诊断搬运(不落盘,沿用现有 OUT_DIR)**
- 做什么:VS Code 扩展监听 `languages.onDidChangeDiagnostics`,捞出 `out_dir` 下生成文件
  URI 上的诊断(r-a **已经在发**,实测 M8),按 `.map` 重写 uri+range,用自己的
  `DiagnosticCollection` 发布到 `.svelte`;`.svelte` 保存时触发 build.rs(或直接调编译器 CLI 重生成)。
- 工作量:**1–2 人周**(前提:§3 的 map 已就位)。零 r-a 实例、零虚拟文档、零 LSP server、
  零 crate graph 改动。
- 收益(**复核下修**):`.svelte` 文件里出现波浪线与 Problems 面板条目。但初稿写的
  "IDE 支持观感的 80% 来自这一条" **不成立**——M8b 已证明这些诊断就是 flycheck 的
  `cargo check` 输出,与 P1 的 `sv check` **同源、同鲜度(保存级)、同内容**。
  a0 相对 P1 的**真实增量**只有两项:① 不用用户自己开终端/配 task;
  ② 复用 r-a 已经在跑的那次 cargo check,不多花一份编译。
- **更便宜的替代(20% 力气拿 80% 收益,初稿漏了)**:`sv check` + `.vscode/tasks.json`
  的 `problemMatcher`。VS Code 内建的 problemMatcher 会把匹配到的
  `文件:行:列: 消息` 直接变成 Problems 面板条目 + 编辑器波浪线,**不需要写任何扩展**。
  成本 ≈ 0.2 人周(一段正则 + 一段文档),覆盖了 a0 收益的绝大部分。
  **建议:P1 交付时顺手带上 tasks.json 模板;P3(扩展)是否还值得做,拿 tasks.json
  用两周之后再决定。** 这条同时消解了 a0 的头号风险(下一条)。
- 风险:VS Code 的 `onDidChangeDiagnostics` 对**未打开文件**的可见性需首日冒烟
  (Problems 面板确实会列出未打开文件的诊断,但扩展 API 能否读到未打开 URI 的诊断
  **未核实**)。**如果这条不成立**,退路就是上面那条 tasks.json 路线。

**a1 · 正/反向位置转发(hover/goto/补全的前提)——落盘由"前提"降级为"可选"**
- 做什么:扩展用 `vscode.executeHoverProvider` 等命令借用用户已在跑的 r-a,
  位置正/反向换算。
- **落盘不是必需的(复核实测,M8c)**:r-a 对 `include!(concat!(env!("OUT_DIR"),…))`
  进来的生成文件,hover 返回 `let count: i32`、goto 正常返回 `LocationLink`,
  **不落盘就已经能用**。所以初稿"a1 = 落盘 + 转发"里的"落盘"应当拆出来单算:
  - **a1a(推荐先做)**:不落盘,直接对 OUT_DIR 的 URI 做正/反向转发。
    goto 目标落在 OUT_DIR 时,用 `.map` 反向改指 `.svelte` —— 用户压根看不到那个丑路径,
    **也就不需要为了"体面"而落盘**。
  - **a1b(可选,按需)**:落盘换的是"Go to generated code 有稳定路径"这一个产品特性,
    以及"生成代码可 diff / 可下断点"。**它换来的不是能力,是可读性**,
    应当和 §7 的止血包(节点锚点注释)一起权衡,不要绑在 hover/goto 的关键路径上。
- 工作量:**a1a 2–4 人周**(比初稿的 3–5 略低,因为砍掉落盘与 mod 聚合),
  置信度**低**(这一档仍是本方案未 spike 面最大的部分:`executeHoverProvider`
  在 OUT_DIR URI 上的行为细节、反向映射的覆盖率都没测过)。
- 代价(诚实列):a1b 若真做,源码树被写入(`_sv.rs` 后缀 + 文件头警告 + gitignore 缓解,
  Dart `.g.dart` 同型);build.rs 与编辑器**双写者**需要约定(见 §7 坑 3)。

### (b) 自建薄 LSP,只做模板域,Rust 域直接放弃

- 能做:标签/属性名补全(属性表在 `codegen.rs`/`template.rs` 里是现成的)、
  `class="…"` 补全(`<style>` 块里的类名已解析)、组件名与 `$props` 字段补全
  (`PropsRegistry`,`lib.rs:79` **已经存在**,build 的第一遍就在建)、块结构折叠、
  未闭合标签实时诊断。这些**全部零依赖 r-a**,而且质量上限比转发路线高(我们比 r-a 更懂 `.svelte`)。
- 工作量:**2–4 人周**(tower-lsp 或 r-a 团队的 `lsp-server` crate + 复用 sv-compiler 的 parser)。
- **裁决:不作为替代品,作为 a1 之后的附加项**。理由很硬:它一行也没有缓解"最大悬置"
  ——`.svelte` 里的 **Rust 表达式**没有类型信息。做完 (b) 之后,用户在 `{count.| }` 处按
  Ctrl+Space 依然什么都没有。把 4 人周花在这里而不花在诊断上,是拿好看的换有用的。
- 但有一条**现在就该顺手做**:`PropsRegistry` 已经存在,意味着 **"组件 props 拼错"
  这类错误已经能在编译器域精确报 .svelte 行列**(测试 `component_call_with_props_and_default`
  已验证)。这是 (b) 的价值在**不做 LSP** 的前提下就能兑现的部分——继续加厚编译器域诊断,
  比做 (b) 的 LSP 壳更划算。

### (c) 完整 Volar 式虚拟文件

- 需要:虚拟文档生命周期、`VirtualCode`/`mappings` 抽象、能力标记、自持或代理宿主分析器、
  编辑器无关的 LSP server、双 r-a 实例(内存 GB 级/实例)。
- 工作量:调研 07 估 S4 = 8–16 人周,**在 a1 之上**。
- **裁决:不做,而且不是"以后再说"式的不做**。理由(复核把依据从 M8 换成 M8c,
  因为 M8 只证明了 flycheck 会转发,证明不了"r-a 在分析生成文件"):**M8c 的 hover/goto
  实测**才是硬证据——r-a 确实把 OUT_DIR 生成文件当真文件在做语义分析。
  Volar 之所以要造虚拟文档,是因为 `.vue` 的 TS 代码在磁盘上**不存在**;而我们的生成
  文件**本来就是真文件**。为了一个已经解决的问题引入一整套框架,
  是把调研 07 §1.1 里 Volar 三次架构大改的学费重付一遍。

### 推荐:**(a)**,按 a0 → a1a 推进,a1b/(b) 作为附加,(c) 明确排除

一句话理由(**复核改写,原句把功劳记在了 M8 头上,而 M8 只是 flycheck 转发**):
**M8c 把 (a) 的地基白送了——r-a 在 OUT_DIR 生成文件上的 hover/goto 今天就能用,
不落盘也能用;于是 (a) 剩下的全部工作是"位置换算 + 搬运";
(b) 解决不了真痛点;(c) 在解决一个我们没有的问题。**

**但要同时记住这条路的天花板(复核补):诊断永远是保存级的**——生成文件 URI 上
根本没有 r-a 的原生诊断(M8b),那条路上没有 keystroke 级新鲜度可拿。
keystroke 级只属于 hover/goto。

---

## 6. 问题五:分步落地、人周与验收

**总原则:第一步必须在 1–2 人周内出可感知的东西**,否则这件事永远排不上
(这是本仓库 R1–R4 的既有节奏,不是空话)。

| 阶段 | 内容 | 人周(**复核修订**) | 置信度 | 退出标准(测试名) |
|---|---|---|---|---|
| **P-1** | `build()` 的 `panic!` → `cargo::error=`;`.vscode/tasks.json` + problemMatcher 模板 | **0.2–0.4** | 高(已实测输出形态) | `build_reports_compile_error_without_panic` + 手查 Problems 面板 |
| **P0** | span map:**第零步 provenance 止血(`script.rs` 三处 re-span)** + 虚拟行号 provenance(**9 个 parse 入口,不是 1 个**)+ 锚点并行走 + 真嵌套包络(节点栈)+ 标点插值 + `.rs.map` sidecar + rune 漂移表 | **2–3.5**(初稿 1–1.5) | 中高(核心机制已跑通,但工作面比初稿大 2–3 倍) | `map_roundtrip_all_examples`、`map_anchor_walk_is_total`、**`map_covers_all_reactive_reads`**、**`map_covers_each_pattern_and_bind`**、`map_survives_prettyplease_reflow`、`map_script_offsets_after_rune_replace`、`map_cjk_utf16_columns` |
| **P1** | `sv check`:cargo JSON 解析 + 重映射 + human 输出 + 全部降级路径 + **build.rs 失败分支** | **1.5–2.5** | 中高 | `check_remaps_type_error_to_sv`、`check_never_drops_diagnostic`、`check_degrades_on_glue`、`check_drops_suggestion_in_glue`、`check_rejects_stale_map`、`check_surfaces_build_script_error` |
| **P2** | TextMate grammar + language-configuration(**并行,不占 LSP 预算**) | 0.5–1 | 高 | 人查 + `grammar_snapshot`(用 `vscode-tmgrammar-test` 的 fixture) |
| **P3** | VS Code 扩展 · a0 诊断搬运 | 1–2 | 中 | `ext_diag_relocation`(单测重映射纯函数)+ 手测清单。**先用 P-1 的 tasks.json 跑两周再决定要不要做** |
| **P4** | a1a:hover/goto 正反向转发(**不落盘**) | 2–4 | **低** | `map_bidirectional_roundtrip`、`lsp_hover_smoke`(#[ignore]) |
| **P5** | 补全(无 auto-import) | 3–6 | 低 | — |
| — | a1b:生成文件落盘 + mod 聚合(可读性特性,非能力前提) | 1–2 | 中 | — |
| — | 通用 LSP server(Neovim/Helix/Zed) | 6–12 | 低 | **第一年不做** |

**"日常可用"= P-1+P0–P4 ≈ 7.4–13.4 人周**(初稿 7–11.5)。区间上移的原因**只有一个**:
P0 的工作面被复核实测放大了(provenance 止血、9 个 parse 入口、包络要真嵌套、
标点插值),不是新增了范围。这个数与调研 07 的 9–16 基本重合——**初稿"低于调研 07"的
那个结论撤回**;调研 07 的 S0(codegen IDE 整备)给 2–3 人周,现在看是对的。

**差额来源仍需诚实说清**:① 我们砍掉了虚拟文档层与通用 LSP server;
② 只做 VS Code、只做只读;③ a1 不落盘(M8c)。**任何一条不成立,估算作废**——
尤其第②条:一旦有人要 Neovim 支持,直接 +6–12 人周,那是另一个决策。

### 6.1 第一步的具体切片(必须出东西)

**复核修订:分成两个"出东西"的时刻,别把它们捆在一起。**

**第 0 天的那个(P-1,0.2–0.4 人周)**:`build()` 的 `panic!` 换成 `cargo::error=`。
交付物是把

```
error: failed to run custom build command for `counter-sfc v…`
Caused by: process didn't exit successfully: …build-script-build (exit code: 101)
  --- stderr
  thread 'main' panicked at …\crates\sv-compiler\src\lib.rs:207:23:
  .svelte 编译失败
    --> src\Counter.svelte:19:15: 属性 `fg`:颜色 `#zzz` 不是合法十六进制
  note: run with `RUST_BACKTRACE=1` …
```

变成

```
error: counter-sfc@0.1.0: src/Counter.svelte:19:15: 属性 `fg`:颜色 `#zzz` 不是合法十六进制
```

(两段都是复核实测输出。)配一份 `.vscode/tasks.json` problemMatcher,
`.svelte` 的**编译器域**错误当天就进 Problems 面板 + 编辑器波浪线,**一行映射代码都不用写**。

**第 2–6 周的那个(P0 表达式部分 + P1 human 输出)**,刻意砍掉:script 块映射
(第一版降级到"整个 script 块 → 一个包络段")、json/annotations 输出、suggestion 处理。
**但 §3.2 第零步(provenance 止血)与 §3.4 第 4 条(标点插值)不能砍**——砍了下面这张
对照图就出不来。

**可感知的交付物**:在 `examples/counter-sfc/src/Counter.svelte` 里把 `{count}` 改成
`{count + "x"}`,然后:

```
$ cargo run -p sv-check
error[E0277]: cannot add `&str` to `i32`
  --> examples/counter-sfc/src/Counter.svelte:12:38
   |
12 |   <text font-size="20">Count: {count + "x"} · 双倍 = {double}</text>
   |                                      ^ no implementation for `i32 + &str`
```

—— 与今天的对照(**复核已在 TEMP 里的 counter-sfc 等价工程上真跑过,不再是推测**):

```
error[E0277]: cannot add `&str` to `i32`
  --> <target>\debug\build\svdemo-<hash>\out/counter.rs:44:44
   |
44 |                 __s.push_str(&(count.get() + "x").to_string());
```

`.svelte` 侧 `12:38` 的坐标经核对是对的(`+` 在 Counter.svelte 第 12 行第 38 个字符)。
**但有两处必须改**(复核):① 错误码是 **E0277**,不是 E0308;
② 生成侧坐标是 **44:44**(初稿写的 47:31 是编的,已换成实测值)。
③ **主 span 就是那个 `+`(宽度 1 的标点)**——所以这张招牌图能不能出来,
取决于 §3.2 第零步(re-span)与 §3.4 第 4 条(标点插值)**两件事都做**。
不做的话,输出只能退到 `Counter.svelte:12`(元素级),说服力大打折扣。

**这个前后对照就是这件事能不能排上号的全部说服力,应该作为 PR 描述的第一张图。**

### 6.2 LSP 的验收怎么自动化(这是本节最容易糊弄的地方)

**关键手法:把每个 LSP 特性劈成"位置换算"(纯函数)与"协议转发"(IO)两半,
把 95% 的验收压在前一半。**

1. **映射的自校验 golden(不需要人工维护 golden 文件)**:
   对每个 `examples/**/*.svelte`,生成 `.rs` + `.map`,然后断言
   **`gen_text[seg.gen_start..seg.gen_end] == sv_text[seg.sv_start..seg.sv_end]`**
   对 `tokens[]` 里每一段成立(token 文本两侧必然逐字相等——这正是"逐字区"的定义)。
   任何一处映射**错位**当场红,而且**不需要任何人工标注**。测试名 `map_roundtrip_all_examples`。

   > **复核警告:初稿称它"最强的一条 / 任何一处映射错位当场红"是危险的高估。**
   > 它只是**soundness** 测试——只检查"已记录的段是否正确",对"**该记录却没记录**"
   > 完全无感。本方案实测到的两个最大缺口(`format_ident!` 丢 provenance 98 处、
   > 6 个旁路 `parse_str` 入口)**都会让这条测试保持全绿**。
   > 靠它验收 P0,等于给自己发一张假的通行证。

1b. **完整性(completeness)断言 —— 复核新增,这才是 P0 真正的验收闸**:
   - `map_covers_all_reactive_reads`:对每个 `.svelte`,统计 script.vars 里每个反应式变量
     在 `.svelte` 源码中的引用次数 N,断言 `tokens[]` 里映射到这些引用位置的段数 == N。
     (今天这个数是 **0/98**;第零步做完应当是 98/98。)
   - `map_covers_each_pattern_and_bind`:`{#each x as pat}` 的 `pat`、`bind:value={x}`
     的 `x`、`$props` 字段类型,各至少有一段映射。
   - **覆盖率地板**:对全部 `.svelte`,`.svelte` 侧"落在模板表达式与 script 块内的非空白字节"
     被 `tokens[]` 覆盖的比例 ≥ 阈值(先量出基线再定,别拍脑袋定 80%)。
     这条同时是 §7 卡死点 2 的**前置**指标——不必等 P1 的 fixture 库才知道要不要止损。
2. **锚点全覆盖断言**:`pre.len() == post.len()` 且逐个文本相等,否则 fail
   (测试名 `map_anchor_walk_is_total`)。这条同时是 prettyplease 升级的哨兵——
   它哪天改了打印策略,这个测试先红,而不是用户先遇到错位的诊断。
   (复核实测:11/11 个真实生成产物上,连**全 token 序列**都相等,不止锚点。
   所以这条断言今天有很大余量,**建议直接断言全 token 序列相等**,保险丝更灵敏。)
3. **重排版鲁棒性**:构造一个必然触发折行的用例(一行 40+ 汉字 + 深嵌套),断言映射不变
   (`map_survives_prettyplease_reflow`)。这是 §3.2 那个真问题的回归卫兵。
4. **诊断重映射的 fixture 双档**:
   - **快档(进 `cargo test`)**:把 rustc JSON **录制成 fixture**(`tests/fixtures/*.json`),
     只测重映射逻辑,毫秒级、无 cargo 依赖。
   - **慢档(CI 单独 job)**:`tests/fixtures/bad_*.svelte`,每个文件顶部一行
     `// expect: 12:38 E0308`,测试在临时 crate 里真跑 `cargo check` 比对。
5. **真 LSP 冒烟(`#[ignore]`,CI 独立 job)**:自建 LSP client(§8 的探针脚本已跑通,
   直接改造)spawn rust-analyzer,`initialize` → `didOpen` → 在 map 指定的位置发
   `textDocument/hover`,断言返回的 markdown 含 `i32`。**CI 需要 `rustup component add
   rust-analyzer`**(本机 stable 工具链默认没有,实测会报 `Unknown binary`)。
   这一档**只保证协议没接错**,不保证质量——质量靠 1–3。
6. **明确不做的验收**:不搞"截图对比 IDE 界面"、不搞 VS Code 集成测试跑真编辑器。
   投入产出比在这个团队规模下不成立,用手测清单(照 `input-demo/README` 的既有形式)代替。

---

## 7. 诚实的失败模式

> 这一节的用途是:**提前写好止损条件,免得半年后靠感觉判断"要不要继续"。**

### 卡死点 1:锚点并行走失配(最可能,但也最容易兜住)

**症状**:`map_anchor_walk_is_total` 红。**原因**:codegen 里某处绕过 `parse_expr` 直接把
用户文本塞进 `quote!`;或 prettyplease 升级改了打印;或某个 `syn` 版本对字面量做了归一化。
**止损**:降级到**包络级映射**(`fresh()` 哨兵,§3.2 第三步)——精度从 token 掉到"节点",
诊断仍然指到 `.svelte` 的正确元素上。**这个降级必须在 P0 就实现,而不是等失配了再写**,
否则失配当天整条管线不可用。

### 卡死点 2:胶水区命中率过高 —— **这是真正该止损的信号**

**症状**:P1 的 fixture 库跑起来,发现**真实类型错误里超过一半落在胶水区**
(尤其 runes 改写产物:`__sv_rhs`、`update(|__v| …)`、`bind_text` 闭包)。
**含义**:"精确回映"是自欺,用户看到的多数是"表达式级 + 一句抱歉"。
**止损动作**:**立刻停掉 LSP 方向的一切投入**,把预算转到 **codegen 的错误落点纪律**
——调研 07 §4 的"每个用户表达式先 `let` 绑定到期望类型显式标注的局部变量"。
理由:落点纪律**同时**改善 rustc 直接输出与我们的映射,而映射只改善后者。
**量化判据**:fixture 库(≥30 个真实错误场景)里 `kind=exact` 占比 **< 60%** 就触发。

> **复核:按今天的代码,这条止损会在第一天就触发,而且是被一个 bug 触发的。**
> 反应式变量的读/写在生成代码里全部是 `format_ident!` 重造的 call_site token
> (98 处,§3.2 第零步),它们是**类型错误最高发的位置**;招牌 demo 的主 span
> 又落在标点上(§3.4 第 4 条)。不做第零步 + 标点插值,`kind=exact` 大概率
> 直接跌到判据线以下,团队会误以为"路线错了"而砍掉一个其实可行的方向。
> **所以:第零步与标点插值必须在 P0 内完成,`map_covers_all_reactive_reads`
> 变绿之前不许开始统计 exact 命中率。**

### 卡死点 3:双写者(build.rs vs 编辑器)—— a1 才会遇到,但会很恶心

**症状**:编辑 `.svelte` 时 cargo 反复重编译、文件锁错误、r-a 索引抖动、诊断闪烁。
**根因**:落盘方案下,build.rs 与扩展都在写同一个 `_sv.rs`,而 r-a 的 watcher 在中间。
**止损**:编辑期**只用内存 overlay 不落盘**(保存 `.svelte` 时才落),或干脆退回 a0(OUT_DIR)。
**预防**:codegen 输出必须**确定性 + 内容不变时不写盘**(`if new == old { return }`)
——这一条应该在 P0 就加进 `lib.rs:214` 的写盘处,成本一行,收益是省掉整类玄学问题。

### 卡死点 4:位置编码(UTF-16)在 CJK 上翻车 —— 本仓库尤其致命

**症状**:`.svelte` 里含中文的行,诊断的波浪线偏移若干字符。
**根因**:LSP 默认 `utf-16` code units;rustc JSON 的 `column_*` 是**字符**列;
我们的 map 用**字节**;**本仓库所有示例都是中文界面**
(`create_text("sv 计数器(.svelte 编译器路线)")`),所以这个坑一定会踩。

**复核修正了两处量化描述**:

- 错位量**不是**"该行中文字数",而是 **2 × 该行 3 字节字符数**(每个 3 字节 CJK 字符
  占 1 个 UTF-16 code unit,byte−utf16 = 2)。实测:含 7 个汉字的一行,
  同一位置 `byte_in_line=53` vs `utf16=39`,差 14 = 2×7。少算一半会让"看起来对了一点"
  的错误修法蒙混过关。
- **真正阴险的不是 CJK,是非 BMP 字符(emoji、CJK 扩展 B)**。对 BMP 内的汉字,
  **字符列 == UTF-16 列**(实测 r-a 给 `character=39`、rustc 给 `column=40`(1-based),
  完全一致),所以"直接用 rustc 的 column"在中文上碰巧是对的;一旦出现 emoji
  (2 个 UTF-16 units),字符列与 UTF-16 列才分家。**用字节 + 统一换算函数**依然是
  正确姿势,但测试用例必须**同时**放中文行与 emoji 行,只放中文测不出这一类。

**止损**:不需要止损,需要**预防**:map 内部一律字节;所有"字节 → LSP Position"
与"字节 → rustc 风格字符列"的换算各收敛到**一个函数**;测试里放一条中文行 + 一条
含 emoji 的行做回归(测试名 `map_cjk_utf16_columns`)。

### 卡死点 5:r-a 对高频 didChange 的增量性能

**症状**:编辑 `.svelte` 时诊断延迟 > 2s,或 CPU 常驻高位。
**已知数据**:483 行落盘生成文件上,`textDocument/hover` **3 ms**(实测 M10,单次)。

**复核把这条的核心疑问回答掉了,同时把它降级**:

- `didChange → publishDiagnostics` 这个往返在**诊断这条路上根本不存在**。M8b 证明
  生成文件 URI 上只有 flycheck(`source="rustc"`)的诊断,而 flycheck 是**保存触发**的;
  r-a 的原生诊断在生成文件上一条都不发。所以"keystroke 级诊断"不是性能问题,
  是**架构上不可得**。初稿把它列为"未核实的往返延迟",方向错了。
- 于是本条的止损("退到保存时才重生成")**其实是唯一可选项,不是退路**。
  实测保存路径的墙钟:改一次 `.svelte` → `cargo check` **242–261 ms**(小工程,§1.2),
  外加 r-a 调度 flycheck 的延迟(冷 10.1 s / 热 3.9 s,含它自己排队与整 workspace check)。
  **保存级 = 秒级**,这依然远好于今天,所以这条不构成路线级风险——但**不要在 PR 里
  宣传"实时诊断"**。
- 真正还没测的性能面挪到了 a1a:**hover/goto 走的是 r-a 自己的分析,确实吃 didChange**。
  3k 行生成文件 + 200ms 防抖下的 salsa 失效级联仍然**未核实**,列为 P4 首日冒烟项。

### 卡死点 6:上游形态变化

r-a 无插件 API、无官方途径复用编辑器里已跑的实例(调研 07 §2.5),我们的耦合面只有
**标准 LSP + cargo JSON** 两个最稳的接口 —— 这已经是最小耦合姿势。
**但有两条本方案自己引入的新耦合必须记账**:

1. **`prettyplease` 的打印行为**(§3.2 第二步依赖"token 序列保持")。它是 dtolnay 的库、
   语义稳定,但**我们的映射正确性挂在它身上**。`map_anchor_walk_is_total` 就是保险丝。
2. **(复核新增)`proc-macro2` 的 fallback span 内部表示**。`byte_range()` 的
   "文件内相对字节"语义、`call_site() == line 1 / 0..0`、`SourceMap` 的 `find()` 里那句
   `unreachable!("Invalid span with no related FileInfo!")` —— 这些都是**实现细节,
   不是 API 契约**(1.0.106 源码,`fallback.rs:344-500`)。锁 `proc-macro2` 的次版本
   意义不大(它是全 workspace 统一的),但**必须有一条测试直接断言这些不变量**
   (测试名 `pm2_span_invariants`),proc-macro2 一升级就先红。
   顺带:`SOURCE_MAP` 的单调增长(§3.1 第 5 条)在长驻进程里是内存泄漏 +
   `u32` 偏移空间的消耗,**别把 sv-compiler 直接嵌进常驻 LSP 进程里循环调用**。

### 什么信号出现就该整体止损、退回"只读特性 + 好错误信息"

**任一条成立即止损**:

1. **P1 交付后,fixture 库里 ≥ 80% 的日常错误已经能落到正确的 `.svelte` 行**
   —— 这时 LSP 转发的边际收益只剩"补全"一项,而补全是 P5(3–6 人周、置信度低)。
   **这是"成功导致的止损",最可能发生,也最该被坦然接受**:把剩下的预算给编译器域诊断
   (did-you-mean、属性名拼错、`$props` 类型不匹配)与文档,收益/成本远高于继续做 LSP。
2. **卡死点 2 触发**(exact 命中率 < 60%)——方向错了,先修 codegen 落点纪律。
3. **P3 做完两周内没人用**(团队内部 `.svelte` 编写者仍然习惯"跳进生成文件改")
   —— 说明痛点判断错了,真实痛点在别处(可能是 `.svelte` 语法本身或组件模型)。
4. **单人维护面告警**:DESIGN.md §6 风险 5 已经列了"单人/小团队维护面过宽"。
   LSP 是**长期税**而非一次性成本(调研 07 §0.4)。如果 R5(鸿蒙)开工与 LSP 撞期,
   **LSP 让路**——因为 `.svelte` 还有 `view!` 宏这个双前端退路(ADR-2:"双前端策略把这笔税
   从赌注变成选项"),而鸿蒙没有退路。

**退回后的止血包**(全部落在 P0–P2 之内,即便止损也已到手):
`sv check` 的精确诊断 + TextMate 高亮 + 生成代码可读性(应当在 P0 顺手加上
**节点锚点注释** `// <button onclick> Counter.svelte:14:5` ——map 里已经有这些区间,
输出到注释是零边际成本,却让"跳进生成文件"从猜谜变成可导航)。

---

## 8. 附:实测记录与复现方法

全部实验在 `%TEMP%` 下的独立 crate 里做,**未改动本仓库任何代码**。

| 编号 | 结论 | 怎么复现 |
|---|---|---|
| M1 | prettyplease `unparse` 文本幂等;**全 token 序列前后不等**(尾逗号被吃,73 vs 71),**Ident/Literal 锚点序列相等**(28 vs 28) | 临时 crate:`quote!` 造含尾逗号的 fn → `parse2` → `to_token_stream()` 拍平 → `unparse` → `parse_file` → 再拍平,两次比较 |
| M2 | 重解析格式化输出 + `span-locations` 给出**输出文本**里的精确 line/col/byte | 同上,打印 `span.start()` 与 `span.byte_range()` |
| M3 | 两次独立 `parse_str` 的 span **不可区分**(`"count * 2"` 的 `count` 与 `"other + count"` 的 `other` 都是 `byte_range 0..5`) | `syn::parse_str::<Expr>` 两次,打印 `byte_range` |
| M4 | **虚拟行号法可行**:前缀 `pad` 个换行后,`start().line == pad + 相对行`,`byte_range.start - pad == 表达式内偏移`;`quote!` 合成 token 恒为 `line=1, byte_range=0..0` | `parse_str(&("\n".repeat(pad) + src))`,pad=1/2/4 各试,含多行表达式 |
| M5 | **端到端映射原型跑通**:一行 28 汉字被 prettyplease 折成 4 行,后续表达式仍精确映射到 .svelte 偏移 200/208/300/308 | §3.2 三步串起来的 60 行原型 |
| M6 | rustc JSON:`file_name` = 生成文件,带 `byte_start/byte_end`,**`expansion: null`**(即使错误在 `println!` 参数里) | 临时 crate:build.rs 写 `$OUT_DIR/counter.rs`(埋 E0599 + E0425),`main.rs` `include!`,跑 `cargo check --message-format=json` |
| M7 | cargo 吐 `{"reason":"build-script-executed", …, "out_dir":"…"}` → OUT_DIR 可确定性发现 | 同上,过滤该 reason |
| M8 | **rust-analyzer 把生成文件的诊断发布到生成文件自己的 `file://` URI,位置正确** | 自建 LSP client(Python,Content-Length 分帧):`initialize`(带 `cargo.buildScripts.enable`)→ `initialized` → `didOpen(src/main.rs)` → 收 `textDocument/publishDiagnostics` |
| M9 | 11 个 `.svelte` 的生成放大 6–8×(行);单文件编译 0.47–5.25 ms(release,50 次均值) | 临时 crate 调 `sv_compiler::compile_sv` 循环计时 |
| M10 | 483 行落盘生成文件上 `textDocument/hover` **3 ms** | 同 M8 的探针,改发 hover |

**复核补做的实测(§10 有完整清单与复现方法)**:M8b(诊断来源判定)、M8c(OUT_DIR 上的
hover/goto)、M11(cargo 缓存重放)、M12(build.rs 失败的输出形态 + `cargo::error`)、
M13(真实 `Counter.svelte` 注错的端到端坐标)、M14(`format_ident!` 的 provenance 丢失)、
M15(`.svelte` 保存 → `cargo check` 墙钟)、M16(proc-macro2 `byte_range` 语义源码核实)。

**未核实清单(不要当成事实引用)**:

- ~~`didChange → publishDiagnostics` 的往返延迟~~ —— **已作废**:诊断这条路上没有这个
  往返(M8b)。取而代之的未核实项是:**a1a 下 hover/goto 在 3k 行生成文件 + 高频
  didChange 时的 salsa 失效开销**。
- **r-a 为什么不给生成文件发原生诊断**(include! 与落盘 mod 两种形态都不发,M8b)。
  这条只影响"能不能拿到 keystroke 级语义诊断"的上限,不影响本方案任何一步,
  但如果有人想把 a0 做成"实时诊断",必须先搞清楚它。
- VS Code 扩展 API 能否读到**未打开文件** URI 上的诊断(a0 的关键前提,P3 首日冒烟)。
  **复核补充的退路**:`sv check` + tasks.json problemMatcher(§5 a0),不依赖这条。
- `vscode.executeHoverProvider` 等命令在生成文件 URI 上的行为细节(调研 07 §6 风险 3)。
  (M8c 只证明了**直连 r-a 的 LSP 请求**可用,没证明 VS Code 命令层的转发可用。)
- 调研 07 §2.1 的原始 open question(**overlay-only、磁盘上不存在的文件**能否被 r-a
  模块树接收)**仍然未核实**——M8/M8c 测的都是磁盘上真实存在的文件。
- 让 r-a 的 `check.overrideCommand` 直接输出 `.svelte` 坐标的假 rustc JSON 是否可行
  —— 沿用调研 07 §4 的判断(r-a 会因 `.svelte` 不在其 VFS 而丢弃或告警),**本方案未测**,
  且**不打算走**:映射责任应留在我们这层。
- 落盘方案下 build.rs 与编辑器双写者的真实表现(卡死点 3)。
- **真实 app 规模下的 `.svelte` 保存 → cargo check 墙钟**。复核实测的 242–261 ms 来自
  只依赖 sv-reactive + sv-ui 的最小工程;带 sv-shell(vello/parley/taffy/winit)的
  叶子 crate 会更慢,未测。
- 本机 stable 工具链未装 `rust-analyzer` component;所有 r-a 实测用的是 VS Code 扩展
  自带的 0.3.2971 二进制。CI 上的版本行为可能不同。

---

## 9. 与既有文档的关系

- **DESIGN.md §6 风险 1**:本方案是它的落地稿。若 P0+P1 落地,该风险的描述应改为
  "rustc 诊断已重映射回 `.svelte`;只读 LSP 特性(hover/goto)未做",杀伤力从第 1 位下调。
- **ADR-2(双前端共存)**:本方案不改变 ADR-2,反而给它加了一条实证——
  `view!` 宏路径的 span 精度是免费的,`.svelte` 路径的 span 精度要花 **7.4–13.4 人周**买
  (复核修订)。这正是 ADR-2 "双前端把这笔税从赌注变成选项" 的定量版本。
  **复核补一句更扎心的**:`view!` 宏路径的 span 精度也不是全免费的——`sv-macro` 的
  codegen 只要也用 `format_ident!` 重造用户名字,同样会丢 span,只是 rustc 直接在
  用户文件里报错,丢的是**列精度**而非整条映射。这条值得单独查一次(**未核实**)。
- **调研 07**:结构与结论沿用。**§0 的三处"修订"经复核只有一处半站得住**:
  ① OUT_DIR 好消息 —— 真正成立的是 hover/goto(M8c),诊断那半是 flycheck 转发(M8b),
     且调研 07 §4 本来就写了;打给它的"过度悲观"判词已收回。
  ② 逐字区假设不成立 —— 成立,但那是**我们的 codegen 没遵守它的建议**,不是它假设错了;
     而且 `extract_props` 那部分是初稿自己看错了源码。
  ③ 虚拟文档层多余 —— 成立(依据从 M8 换成 M8c)。
- **调研 06(build.rs/OUT_DIR 定案)**:a0 与 **a1a 都完全兼容现状**(M8c:不落盘也有
  hover/goto);只有可选的 a1b 会动这条决策,届时才需要在 DESIGN.md 里补 ADR。

---

## 10. 复核记录

> 对抗性复核,立场是"这份方案有问题",目标是尽力证伪。复核日期 2026-07-22,
> 与初稿同一环境(rustc 1.88.0 / r-a 0.3.2971-standalone / proc-macro2 1.0.106 /
> prettyplease 0.2.37 / syn 2.0.119,后三者取自 `Cargo.lock`)。
> 全部实验在 `%TEMP%` 与 scratchpad 下的独立工程里做,**本仓库零改动,只改了本文件**。

### 10.1 先说结论:初稿的可信度分布

初稿**不是**编出来的——凡是能核对的行号、token 数、体量数据,复核逐条对上了:

| 初稿的声明 | 复核结果 |
|---|---|
| `lib.rs:44/69/80/120/178/213`、`codegen.rs:126/133/195/200/207`、`script.rs:237/367/404/454` | **全部准确** |
| `grep -c "self.expr("` = 30 | **准确** |
| Counter 生成 180 行 / 展平 1017 token / Ident+Literal 锚点 425 | **一字不差复现** |
| M9 的 5 个文件的生成行数与字节数 | 复现,只有 InputDemo 字节数差 1(14 530 vs 14 531) |
| M6/M7:`file_name` 是生成文件真实路径、带 `byte_start/byte_end`、`expansion: null`(含 `println!` 参数内)、`build-script-executed` 带 `out_dir`、Windows 下 `…\out/counter.rs` 反斜杠正斜杠混用 | **全部复现** |
| M3/M4:两次独立 `parse_str` 的 span 不可区分;pad 换行法可用;`quote!` 合成 token 恒为 `line=1, byte_range=0..0` | **复现** |
| 环境声明(rustup 报 `Unknown binary 'rust-analyzer.exe'`、r-a 取自 VS Code 扩展) | **复现,报错文本一字不差** |

编译耗时复现为 0.44 / 1.65 / 1.56 / 3.35 / 3.67 ms(初稿 0.47 / 2.41 / 1.84 / 5.06 / 5.25),
同量级、系统性偏低约 30%,属机器负载差异,不算问题。

**所以下面列的都是"判断/解读/工作量"层面的问题,不是数据造假。**

### 10.2 推翻的三条(按杀伤力排序)

**R1 · M8 的解读错了:那不是 r-a 在分析生成文件,是 flycheck 在转发 `cargo check`。**
初稿把 M8 当成整篇方案的地基(§0.1「地基」、§5 杀 (c) 的理由、a0 的价值论证)。
复核加打 `source` 字段 + 做对照实验:默认配置下诊断 `source="rustc"`、与
`rust-analyzer/flycheck/0` 同时到达;`check.enable=false` 后生成文件 URI 上
**一条诊断都没有**(显式 `didOpen` 并等 70 s 仍 `n=0`),而 `src/main.rs` 上 r-a 原生
`source="rust-analyzer"` 的 E0308 照常 1.0 s 到达。落盘成真 `mod` 的形态也一样是 0 条。
→ 已改写 §0.1 与 §1.3(新增 M8b 表),并据此下修 §5 a0 的价值、修正 §7 卡死点 5。
另外:调研 07 §4 原文就写了"flycheck 都发布到生成文件 URI",所以 M8 是**证实**;
被打"过度悲观"标签的那句在 **§2.2** 而不是 §2.1,且它同句就承认 r-a 索引 OUT_DIR。
§2.1 真正的 open question 是 **overlay-only 文件**,M8 完全没碰到,已放回未核实清单。

**R2 · 「插桩只改 `parse_expr` 一处」不成立,而且最要命的丢失点在 `script.rs`,不在
`codegen.rs`。** `Rewriter` 用 `format_ident!("{name}")` 重造用户变量名
(`script.rs:884/899/1038`),重造出的 Ident 是 `Span::call_site()` = `line 1, 0..0`
= 方案自己定义的"胶水"。**即每一次反应式变量的读/写都会丢 provenance**,
全示例实测 **98 处**(Counter 9、InputDemo 24、Settings 14、wide 35 …)。
另有 6 个旁路 `parse_str` 入口(`codegen.rs:267/1021/1081/1112/1388/1403/1688`)
与 2 个 `script.rs` 入口不走 pad,产出的 token `line` 恒为 1、同样被静默丢弃。
→ 已在 §3.2 前面加"第零步 · provenance 止血",在 §1.1 表里加两行,
并在 §6 把 P0 从 1–1.5 人周上调到 2–3.5。

**R3 · 招牌 demo 按方案自己的机制做不出来。** 把 `{count}` → `{count + "x"}` 注入
**真实的 Counter.svelte** 跑 `cargo check --message-format=json`,得到的是
`E0277`(不是初稿写的 E0308)、生成侧 `44:44`(不是初稿编的 `47:31`)、
**主 span 宽度 1,就是那个 `+`**。而 §3.2 明写"标点一律跳过"、`tokens[]` 只放
Ident/Literal → 这条错误查不到精确段,只能退到元素级。
→ 已在 §3.4 加第 4 条(相邻锚点插值)、在 §6.1 改正错误码与坐标并写清依赖关系。

### 10.3 改掉的事实错误

| 位置 | 初稿 | 实际 |
|---|---|---|
| §0.2 / §1.1 | `extract_props` "把 `$props { … }` 整块删掉","偏移会漂" | **字节等长空白替换**,字节偏移恒等(`script.rs:347-357`,注释原文写着"行列不漂移") |
| §3.2 第三步 | `fresh()` "已经给**每个节点**发了唯一名","零新增机制" | `self.fresh(` 只有 2 个调用点(`:318` 文本、`:503` 元素);`{#if}`/`{#each}`/组件调用**无哨兵**。且"下一个 `__elN` 首现即终点"实测会把 `{#if}` 的条件(Counter 生成 L142)归给上一个按钮(`__el7`,L120),父元素的包络也算错 |
| §6.1 | `error[E0308]` / `counter.rs:47:31` | `E0277` / `44:44` |
| §3.4.2 | "`css_c1_*` 测试族已覆盖" | 全仓 CSS 测试只有 2 个:`css_c1_box_model_vars_nesting`、`css_compat_names_units_hover` |
| §3.4.2 | 只说 `<style>` 块 | 内联 `style="…"` 同样折叠成 `Color::rgba(255u8, 62u8, 0u8, 255u8)`,而 Counter.svelte 根本没有 `<style>` 块 |
| §7 坑 4 | "错位量恰好等于该行中文字数" | **2 ×** 3 字节字符数(实测 7 个汉字 → 差 14);且 BMP 汉字上"字符列 == UTF-16 列",真正分家的是 emoji |
| §1.2 | "生成放大 6–8×" | 5.6–8.4×(Card 5.6、Settings 8.4) |

### 10.4 补上的致命遗漏

1. **编译器域错误根本不进 cargo 的 JSON 流**(§4.1 新增第 0 分支)。`.svelte` 的模板/CSS/runes
   错误走 `lib.rs:207` 的 `panic!`,实测结果:本包**既没有 `compiler-message` 也没有
   `build-script-executed`**,错误埋在 cargo stderr 的 panic dump 里,还带着
   `E:\WorkSpaces\svelte-rs\crates\sv-compiler\src\lib.rs:207:23` 这样的内部路径。
   §4.1 第 2 步"记下 out_dir"在这条最常见的路径上直接失效。
   **这是整篇方案里最容易在第一天撞墙的地方,初稿一个字没提。**
2. **`proc-macro2` 的 `SOURCE_MAP` 是永不回收的 thread_local**(§3.1 新增第 5 条)。
   每次 `parse_str`/`parse_file` 都塞一个持有完整源文本的 `FileInfo`。
   build.rs 一次性进程无所谓,**a0/a1 那个"keystroke 级重新生成"的长驻进程就是
   单调泄漏**;唯一的清理入口 `invalidate_current_thread_spans()` 会让旧 span
   走到 `SourceMap::find` 的 `unreachable!` **直接 panic**——这与本仓库的去 panic 纪律
   正面冲突,必须在设计阶段就选定"fork 子进程"或"明确的失效边界"。
3. **`map_roundtrip_all_examples` 是 soundness 测试冒充 completeness 测试**(§6.2)。
   初稿称它"最强的一条 / 任何一处映射错位当场红"。实际上它只检查"已记录的段是否正确",
   而 R2 的两个缺口(98 处丢失 + 6 个旁路入口)**都会让它保持全绿**。
   已补 §6.2 第 1b 组完整性断言(`map_covers_all_reactive_reads` 等),
   并把 §7 卡死点 2 的 exact 命中率统计**前置条件化**——否则团队会被一个 bug 骗着
   砍掉一条其实可行的路线。
4. **`"sv"` 路径要绝对**(§3.3)。build.rs 的 cwd 是包根、`sv check` 的 cwd 是 workspace 根。
5. **三种位置编码同时在场**(§3.3):字节(map)/字符(rustc JSON `column_*` 与
   `lib.rs:69` `line_col`)/UTF-16(LSP)。初稿只提了前两种的一半。

### 10.5 补上的"20% 力气拿 80% 收益"

1. **`sv check` + `.vscode/tasks.json` 的 problemMatcher,不写扩展**(§5 a0)。
   既然 a0 的数据源就是 `cargo check`(R1),那 P3 那 1–2 人周的扩展相对于
   "一段正则 + 一段文档"的增量只有"不用自己开终端"。已建议**先用 tasks.json 跑两周
   再决定要不要做 P3**,顺带消解了 a0 的头号未核实风险(未打开文件的诊断可见性)。
2. **`build()` 的 `panic!` → `println!("cargo::error=…")`,列为 P-1,0.2–0.4 人周**(§4.1)。
   实测输出 `error: svdemo@0.0.0: src/Counter.svelte:19:15: 属性 …` + `error: build script
   logged errors`,一行、规整、problemMatcher 直接能吃,而且**去掉一个 panic**。
   **诚实提醒:复核实测它不进 `--message-format=json` 的 `compiler-message`**(一条都没有),
   只上 stderr——`sv check` 仍要读 stderr,但从"解析 panic dump"降级成"匹配一条 error 行"。
3. **a1 不需要落盘**(§5 a1a)。M8c 实测 OUT_DIR 生成文件上 hover/goto 直接可用,
   初稿为 a1 列的三条落盘理由里有两条(URI 体面、r-a 可见)可以用反向映射解决。
   落盘降级为可选的 a1b(买的是可读性,不是能力)。

### 10.6 工作量:初稿乐观,但方向不算离谱

初稿 P0–P4 = 7–11.5 人周,并宣称"低于调研 07 的 9–16"。复核修订为
**P-1+P0–P4 = 7.4–13.4 人周**,与调研 07 的 9–16 基本重合,**"低于调研 07"这个结论撤回**。
上调**只来自 P0**(1–1.5 → 2–3.5):provenance 止血、9 个 parse 入口、包络要真嵌套、
标点插值、完整性测试。调研 07 给 S0(codegen IDE 整备)2–3 人周,现在看它是对的。
P4 因为不落盘反而下调(3–5 → 2–4),对冲了一部分。

**仍然认为初稿的路线判断是对的**:(a) 优先、(c) 排除、(b) 作附加项、
"第一步是 `sv check` 不是 LSP"、"特性排序按映射方向数而非直觉"、
"hover 排 goto 之前"——这几条复核没有找到反例,予以保留。

### 10.7 复核**没能**验证的部分(不要以为已经清了)

- **r-a 为什么不给生成文件发原生诊断**。三种形态(OUT_DIR + `include!`、源码树内
  真 `mod`、二者叠加 flycheck 关闭)都测了,结论一致是"不发",但**根因未查明**。
  有可能是 r-a 只对"属于某个模块且被 didOpen"的文件算,而 `include!` 进来的文件
  归属主模块;也有可能是我的探针在 `initializationOptions` 上还差一个开关。
  **这条不影响本方案任何一步,但它是"能不能做实时诊断"的天花板。**
- **VS Code 命令层**(`vscode.executeHoverProvider` / `languages.getDiagnostics`)
  的行为一次都没测——我只做了直连 r-a 的 LSP client 探针。a0 与 a1a 的落地风险
  主要都在这一层,复核没有降低它。
- **a1a 的反向映射覆盖率**、**高频 didChange 下 r-a 的 salsa 开销**:未测。
- **真实 app 规模的保存→check 墙钟**:只测了最小工程(242–261 ms),
  带 sv-shell 的叶子 crate 未测。
- **`sv-macro`(`view!` 宏前端)是否有同类的 `format_ident!` 丢 span 问题**:
  §9 提了一句,**没查**。若有,`view!` 路径"span 精度免费"这个论断也要打折。
- **`prettyplease` 未来版本的行为**:复核只证明了 0.2.37 在**当前 11 个 `.svelte` 的产物形状上**
  全 token 序列相等。这是一个关于"我们现在生成什么"的性质,不是关于 prettyplease 的保证。
- **本复核所有 r-a 实测都用 VS Code 扩展自带的 0.3.2971**,与初稿同一二进制;
  CI 上 `rustup component add rust-analyzer` 装的版本行为**未核实**。
