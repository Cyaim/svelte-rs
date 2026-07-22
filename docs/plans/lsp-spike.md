# `.sv` IDE 体验(LSP)spike 方案

> 状态:方案 / 未开工。对应 DESIGN.md §6 风险清单第 1 条(`.sv` 的 IDE 体验是编译器路线
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
     也就是说:**每一次对反应式变量的读/写(`.sv` 里最常见、也最常出类型错误的
     那个 token),provenance 会被无声丢弃**。全示例实测 98 处。详见 §3.2 第零步。
3. **`prettyplease 重排版之后行号还对不对得上` 这个真问题,答案是:不要在格式化之前建
   映射。** 做法:格式化后把输出文本**重新 parse 一遍**,拿到每个 token 在**输出文本**里的
   精确字节区间,再与格式化前的 token 流做**锚点并行走**(实测 M1/M2/M5:CJK 长行被
   prettyplease 折成 4 行,映射毫发无损)。
4. **推荐路线:§5 的 (a),并拆成 a0/a1 两档**。a0 = 沿用现有 OUT_DIR,只做"诊断搬运"
   (零 r-a 实例、零虚拟文档、零 LSP server);a1 = 生成文件落盘 + 正反向位置转发,换
   hover/goto。**(c) 完整 Volar 式虚拟文件明确不做**;(b) 模板域薄 LSP 是 a1 的附加项,
   不是替代品——它救不了"`.sv` 里的 Rust 表达式没有 IDE"这个真痛点。
5. **第一步(1–2 人周)不是 LSP,是 `sv check`**。理由不是保守,是杠杆:今天 `.sv` 用户
   写错一个类型,错误落在 `target/debug/build/<pkg>-<hash>/out/counter.rs:14:31`
   ——一个用户**永远不会打开**的路径。把这条错误搬回 `Counter.sv:14:31`,是整条路线上
   单位成本体感最高的一步,而且 100% 在我们自己的代码里,可被 `cargo test` 验收。

---

## 1. 现状核实(读源码 + 实测,不是转述)

### 1.1 编译管线的形状

`compile_sv_with`(`lib.rs:120`)四步:`sfc::split` → `script::transform` →
`template::parse` → `codegen::generate`。产物由 `build()`(`lib.rs:178`)写到
`$OUT_DIR/<fn_name>.rs`(`lib.rs:213`),示例侧 `include!(concat!(env!("OUT_DIR"), "/counter.rs"))`
(`examples/counter-sfc/src/main.rs:9`)。

| 事实 | 位置 | 对映射的含义 |
|---|---|---|
| 模板表达式带 .sv 字节偏移 | `template.rs:36` `ExprSrc { src, offset }` | **provenance 的原料已经在了** |
| 30 处模板表达式走 `self.expr(` 这一个入口 | `codegen.rs:200` `parse_expr` / `:207` `expr`(`grep -c "self.expr("` = 30) | 但**它不是唯一的用户文本 parse 入口**,见下一行 |
| **还有 6 处 `parse_str` 直接吃用户文本** | `codegen.rs:267`(`$props` 类型)、`:1021/:1081/:1112/:1388/:1403`(`bind:` 目标)、`:1688`(`{#each}` 模式 `Pat::parse_single`) | 「插桩只改一个函数」**不成立**;这些走的是无 pad 的裸 `parse_str`,其 token 的 `line` 恒为 1 → 会被本方案的判据**误判为胶水而静默丢弃** |
| `proc-macro2` 已开 `span-locations` | `crates/sv-compiler/Cargo.toml`(锁定 1.0.106) | 不用加依赖;`script.rs:454` `syn_err` 已经在用它反算行列(**既有先例**) |
| script 块**整块一次** parse | `script.rs:367` `format!("{{\n{pre}\n}}")` + `parse_str` | 注意 wrapped 前缀是 `{\n` = **2 字节**,pad 之外还要减这 2 |
| parse 前的两次纯文本改写里,**只有一次会漂** | `script.rs:404` `replace_runes`(6 个 rune 一律 `$xxx`→`__sv_xxx`,**恒 +4 字节/处**);`script.rs:237` `extract_props` **字节等长空白替换**(`:347-357`) | 漂移表 = 一张 "+4 计数表";`extract_props` 可当恒等处理(**初稿此处写错,已改**) |
| **`Rewriter` 用 `format_ident!` 重造用户变量名** | `script.rs:884` `.set()`、`:899` `.update()`、`:1038` `.get()` | **provenance 在这里被丢掉**(call_site);全示例 98 处。这是本方案最大的实现缺口 |
| 生成代码经 `syn::parse2` + `prettyplease::unparse` | `codegen.rs:126-134` | 格式化前的 token 流可拿到;格式化后位置要靠重解析 |
| **元素与文本节点**用递增唯一名 `__{prefix}{n}` | `codegen.rs:195` `fresh`,只有两个调用点:`:318` `fresh("t")`、`:503` `fresh("el")` | 哨兵**只覆盖元素/文本节点**;`{#if}`/`{#each}`/`{#key}`/组件调用/script 语句**没有哨兵**(初稿写"每个节点"是错的) |
| 生成文件唯一注释是文件头一行 | `codegen.rs:132` | 没有任何锚点注释,"跳进生成文件看" 目前靠猜 |
| 编译错误只在**编译器域**自报 .sv 行列 | `lib.rs:44` `CompileError`、`:69` `line_col`(**列是字符数,不是字节**) | 模板/CSS/runes 错误已经是好体验;rustc 错误是零体验 |
| 编译器域错误经 **`panic!`** 冒出来 | `lib.rs:207` `panic!("\n\n.sv 编译失败\n  --> {e}\n")` | 它**不进 cargo 的 JSON 流**,只在 cargo stderr 的 panic dump 里,`sv check` 必须单独处理(§4.1 已补) |

**结论:现在没有任何 span 映射(生成代码 → .sv),这一条与任务描述一致,已核实。**

### 1.2 生成产物的规模(实测 M9)

对全部 11 个 `.sv` 跑 `compile_sv`(release,50 次取均值):

| 文件 | .sv 行 | 生成行 | 生成字节 | 编译耗时 |
|---|---|---|---|---|
| Card.sv | 13 | 73 | 2 576 | 0.47 ms |
| Counter.sv | 23 | 180 | 6 382 | 2.41 ms |
| TodoItem.sv | 23 | 172 | 6 372 | 1.84 ms |
| InputDemo.sv | 63 | 394 | 14 530 | 5.06 ms |
| Settings.sv | 57 | 476 | 18 906 | 5.25 ms |

两个可直接用于决策的数:**生成放大 5.6–8.4×(行)**;**单文件编译 0.4–5.2 ms**
(复核独立复现:0.44 / 1.65 / 1.56 / 3.35 / 3.67 ms,同量级、系统性偏低约 30%;
token 与锚点数完全一致:Counter 生成 180 行 / 1017 token / 425 锚点)。

后者意味着**编译器不是 IDE 路径上的瓶颈**,"要不要做增量编译器"这个分心项
**第一年不需要**。但这句话的适用范围要收窄(复核修订):

- 真正决定"保存 → 看到诊断"的不是 sv-compiler,是 **build.rs 重跑之后 cargo 必须重编
  整个叶子 crate**。复核实测(TEMP 里 counter-sfc 的等价工程,依赖只有 sv-reactive +
  sv-ui):改一次 `.sv` → `cargo check` 墙钟 **242–261 ms**;空跑 129 ms;
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
  这是本方案的地基:r-a 侧**什么都不用做**,我们只需要一层搬运。
  (同一工程用 `rust-analyzer diagnostics .` CLI 也能查出这两个错,但 CLI 打印的
  文件名是被扫描的根文件而非诊断所属文件,容易误读——**以 LSP 层的结果为准**。)

**这条实测把调研 07 §2.1 的 open question(overlay-only 文件能否被模块树接收 / OUT_DIR
体验"二等")直接跳过了**:OUT_DIR 路径下的生成代码,r-a 已经在正常分析并正常发诊断。
调研 07 因为 issue #13520 对 OUT_DIR 打了"二等"的标签,在 2026-07 的 r-a 上**至少诊断这条
路径是通的**。剩下的二等之处只有一个,而且是**产品问题不是技术问题**:那个 URI 用户
永远不会打开。

---

## 2. 问题一:最小可用切片(排序与理由)

判据不是"这个特性有多爽",而是 **"它需要几个方向的映射 + 需不需要改协议"**。
按此排序,顺序是天然的:

| # | 特性 | 需要的映射方向 | 需要新组件 | 体感 | 裁决 |
|---|---|---|---|---|---|
| 0 | **语法高亮/折叠**(TextMate injection) | 无(不走 LSP) | 无 | 从"纯文本"到"正常语言" | **并行做掉,不占 LSP 预算** |
| 1 | **诊断重映射** | 仅**反向** gen→sv | `sv check` CLI(+ 可选扩展) | 最大 | **P0** |
| 2 | **悬停类型 hover** | **正向** sv→gen(结果文本不用回映) | VS Code 扩展 | 大 | **P1** |
| 3 | **跳转定义 goto** | 正向 + 结果**反向**(目标落在生成文件时要改指 .sv) | 同上 | 中 | P2 |
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
写错 `.sv` 时看到的东西,全是它。调研 07 §7 的判断"没有 LSP 时,诊断质量就是 DX 的
全部下限"在本仓库尤其成立,因为**模板域/CSS 域/runes 域的错误已经是精确的 .sv 行列**
(`lib.rs:44`),唯独 rustc 域是断的——补上这一块,`.sv` 的错误体验就**整体**及格了。

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
   → 并行走**只能走 Ident/Literal**,标点一律跳过。(这正好够用:映射只需要锚在
   标识符与字面量上。)

### 3.2 机制:虚拟行号 provenance + 格式化后锚点并行走

分三步,每步都已在 `%TEMP%` 里单独跑通(§8 复现):

**第一步 · 给用户 token 打上可区分的来源(改 `codegen.rs:200` `parse_expr` 一处)**

不要直接 `syn::parse_str(&e.src)`,而是**前面垫 N 个换行**再 parse:

```rust
// 伪码,示意签名变化;实际实现进 Cg,用 Cell/RefCell 存表避免 &mut self 借用冲突
fn parse_expr(&self, e: &ExprSrc) -> Result<syn::Expr, CompileError> {
    let pad = self.map.alloc_lines(e.src.lines().count().max(1)); // 单调递增,从 1 开始
    let expr = syn::parse_str(&format!("{}{}", "\n".repeat(pad), e.src))?;
    self.map.record_site(pad, e.offset, e.src.len());            // 虚拟行段 → .sv 偏移
    Ok(expr)
}
```

实测 M4 保证了这一步的正确性:`pad=4` + `"if a {\n  b\n} else { c }"` 时,
`if`/`a` 报 `line=5`、`else` 报 `line=7`,而 **`byte_range.start - pad` 恰好等于
token 在原表达式串里的偏移**(0 / 3 / 13)。于是:

- **表达式归属** = `span.start().line - 1` 落在哪个已登记的虚拟行段;
- **表达式内偏移** = `span.byte_range().start - pad`;
- **.sv 偏移** = `ExprSrc.offset + 表达式内偏移`。

**合成 token 天然可分辨**:`quote!` 造的 token 一律 `line = 1`、`byte_range = 0..0`
(实测 M4)。把虚拟行号从 1 开始分配,`line == 1` 就是"胶水"的判据,零歧义。

**script 块同理但更简单**:它是**整块一次 parse**(`script.rs:367`),给整块分配一个虚拟
行段即可。**但必须补一张偏移漂移表**:`extract_props` 删块、`replace_runes` 把 `$state`(6B)
换成 `__sv_state`(10B),两次纯文本改写各自制造位移。改法是让这两个函数顺手返回
`Vec<(orig_start, orig_len, new_len)>`,合成一个分段线性映射即可(两者都是简单的
非重叠替换,不需要通用 diff)。**这是本方案里最容易被漏掉、也最容易在测试里逮到的坑**
——`map_roundtrip_all_examples`(§6)会当场变红。

**第二步 · 格式化之后,重解析,锚点并行走**

```rust
let file: syn::File = syn::parse2(file_ts)?;      // codegen.rs:126 既有
let pre  = anchors(file.to_token_stream());        // Vec<(text, span)>,只收 Ident/Literal
let out  = prettyplease::unparse(&file);           // codegen.rs:133 既有
let post = anchors(syn::parse_file(&out)?.to_token_stream());
debug_assert_eq!(pre.len(), post.len());           // 失配 = 降级信号,见 §7
for (a, b) in pre.iter().zip(post.iter()) {
    if a.text != b.text { /* 降级 */ }
    // a.span → (哪个表达式, 表达式内偏移) → .sv 偏移
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

只有 token 级散点是不够的:rustc 的错误 span 经常落在胶水上(见 §3.4)。所以同时记录
**包络区间**:每个模板节点、每条 script 语句在生成文件里的字节区间 → .sv 区间。
包络的"哨兵"现成:`codegen.rs:195` 的 `fresh()` 已经给每个节点发了唯一名 `__el7`,
在**格式化后文本**里搜这个名字的首次出现即可定位节点起点,下一个 `__elN` 的首次出现即
终点。**零新增机制**。

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
  "sv": "src/Counter.sv",
  "gen": "counter.rs",
  "sv_len": 1043, "gen_len": 6382,          // 廉价的一致性校验(比 hash 便宜,够用)
  "sv_hash": "fnv1a:3f2a…",                  // .sv 内容 hash;不匹配 → 拒绝使用该 map
  // 精确段:token 级,按 gen 起点排序、互不重叠 → 二分查找
  // [gen_start, gen_end, sv_start, sv_end]
  "tokens": [[1180,1185,512,517], [1188,1189,520,521], …],
  // 包络段:节点/语句级,**可嵌套**,查找取最内层
  // [gen_start, gen_end, sv_start, sv_end, kind]  kind: 0=script 语句 1=元素 2=块 3=属性
  "spans":  [[900,1100,470,620,1], …]
}
```

- **两侧全用字节偏移**,不用行列。理由是实测 M6:rustc JSON 直接给 `byte_start/byte_end`,
  查表零转换;行列只在最后输出给人看/给 LSP 时才算(LSP 还要 UTF-16,见 §7 坑 4)。
- **粒度裁决:token 级(锚点级)为主,包络级兜底,不做"行级"**。行级看似便宜,实际上
  在本仓库是**错的**:prettyplease 会把一条语句摊成 5 行(见 §1.2 的 `update_style`),
  一行里也可能挤着两个不同来源的表达式。既然锚点并行走已经把 token 级做出来了,
  行级反而是多余的中间态。
- **体量**(实测):`Counter.sv` 的生成文件(181 行)展平后 1017 个 token,其中
  **Ident+Literal 锚点 425 个**;真正来自用户源码的锚点是其中的少数(其余是
  `::sv_ui::` 路径、`__elN`、闭包骨架这些胶水)。→ map 是 KB 级,可以无脑随 `.rs`
  一起写盘、一起被 `rerun-if-changed` 逻辑管理。

### 3.4 这套机制**做不到**的事(必须写进文档,不能装看不见)

1. **runes 改写让"一个用户表达式"不再是一段连续生成代码**。实测(`Counter.sv` 的真实产物):

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
2. **`<style>` 块没有 provenance**。CSS 值在编译期就折叠成字面量
   (`Color::rgba(255u8, 62u8, 0u8, 255u8)`,`style.rs`),生成代码里的这些 token 全是合成的。
   **这没关系**:CSS 域的错误本来就由编译器自报 .sv 行列(`css_c1_*` 测试族已覆盖),
   rustc 永远看不到 CSS 错误。
3. **列精度止步于 token**。rustc 有时把 span 划在子表达式的一部分(如 `.to_string()` 的
   方法名),映回去只能定位到最近的用户 token。可选的廉价补救:用 rustc JSON 的
   `spans[].text[].text`(那一行的原文)在 .sv 的表达式源码里做一次唯一匹配来收窄列
   ——**是启发式,匹配不唯一就放弃**,不要为了几个列位置引入不可解释的行为。

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
  │               命中 tokens[]  → 改写为 .sv 精确区间          (kind=exact)
  │               命中 spans[]   → 收敛到最内层包络的 .sv 区间   (kind=approx)
  │               都没命中       → **降级**,见 4.2             (kind=unmapped)
  │
  ├─ 4. 递归处理 children[](note/help 子诊断)与 suggested_replacement
  │
  └─ 5. 输出:human(用 .sv 原文**重新渲染**)/ json / github-annotations
         退出码:有 error → 1;仅 warning → 0(`--deny-warnings` 可改)
```

**路径匹配的两个 Windows 坑(实测 M6 里就能看到)**:rustc 吐出来的路径是
`C:\…\out/counter.rs` ——**反斜杠与正斜杠混用**(前半段来自 cargo,`/counter.rs`
来自 `include!` 的字面量拼接)。必须先规范化再比较;直接字符串相等**一定**匹配不上。
其次,同一个包在 `target/debug/build/` 下可能残留多个 `<pkg>-<hash>/out` 目录(旧的构建),
**只认本次 cargo 运行吐出来的 `out_dir`**,不要 glob。

### 4.2 失败模式(每条都必须"降级"而不是"丢")

**铁律:输入 N 条诊断,输出必须 N 条。**"我映射不了所以我不说了"是最坏的失败——用户会
以为编译通过了。对应验收测试 `check_never_drops_diagnostic`(条数守恒断言)。

| 失败模式 | 触发条件 | 降级行为 |
|---|---|---|
| **查不到映射段** | 主 span 落在纯胶水(如 `bind_text` 闭包骨架) | 输出**保留原诊断全文**,位置指向 **`<组件名>.sv`(整文件)**,附一行:`该错误落在 sv-compiler 生成的胶水代码上(<gen路径>:<行>:<列>),通常是编译器 bug,请附 .sv 源码上报` |
| **map 文件缺失/过期** | `.sv` 改了但 build.rs 没重跑;或 hash 不匹配 | **完全不重映射**,原样透传 rustc 诊断(指向生成文件)+ 一条 warning:`未找到与 <gen> 匹配的 span map,诊断保持生成文件坐标` |
| **同一诊断多 span** | rustc 的 `expected due to this` 等多标注 | **逐 span 独立映射**;主 span 能映、次 span 不能 → 主 span 用 .sv 坐标,次 span 降级为 note 文本(不硬凑位置) |
| **suggestion 落在胶水区** | `suggested_replacement` 的区间不在 tokens[] 内 | **丢弃该 suggestion**(不是丢诊断)。理由:把胶水代码的修复建议应用到 .sv 会**毁用户源码**,这是唯一一处"丢"是正确的 |
| **诊断跨越多个映射段** | 错误 span 覆盖 `用户表达式 + 胶水` | 取**包络**(最内层 spans[]),标 approx;不要拼接不连续的 .sv 区间 |
| **`rendered` 字段** | 永远 | **重渲染**。透传等于把 `target/.../out/counter.rs` 的路径与生成代码摘录甩给用户,比不重映射还糟 |
| **同一个 .sv 被多个 crate 编译** | workspace 里两个包引用同一目录 | 按 `out_dir` 分别处理,输出去重(key = .sv 路径 + 区间 + 消息) |
| **cargo 本身失败**(链接错误等) | 无 compiler-message | 原样透传 cargo 的 stderr,退出码跟随 |

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
  `DiagnosticCollection` 发布到 `.sv`;`.sv` 保存时触发 build.rs(或直接调编译器 CLI 重生成)。
- 工作量:**1–2 人周**(前提:§3 的 map 已就位)。零 r-a 实例、零虚拟文档、零 LSP server、
  零 crate graph 改动。
- 收益:`.sv` 文件里出现波浪线与 Problems 面板条目——**"IDE 支持"这个观感的 80% 来自这一条**。
- 风险:VS Code 的 `onDidChangeDiagnostics` 对**未打开文件**的可见性需首日冒烟
  (Problems 面板确实会列出未打开文件的诊断,但扩展 API 能否读到未打开 URI 的诊断
  **未核实**)。**如果这条不成立**,退路是扩展自己跑 `sv check`(慢一档,但必定可用)。

**a1 · 落盘为一等产物 + 正/反向位置转发(hover/goto/补全的前提)**
- 做什么:生成文件从 OUT_DIR 挪到源码树(`src/components/Todo_sv.rs` + `mod` 聚合,
  `.gitignore` 默认忽略),扩展用 `vscode.executeHoverProvider` 等命令借用用户已在跑的
  r-a,位置正/反向换算。
- 为什么要落盘(而不是继续 OUT_DIR):**不是为了让 r-a 看见**(它已经看见了),而是为了
  ① **"Go to generated code" 是个能用的产品特性**——用户跳过去看到的是稳定路径,不是
  带 hash 的 target 深处;② goto definition 的目标落在生成文件时,给用户的 URI 得体面;
  ③ 编辑期 overlay 更新与磁盘内容不打架。
- 工作量:**3–5 人周**,置信度**低**(这一档是本方案里未 spike 面最大的部分)。
- 代价(诚实列):源码树被写入(`_sv.rs` 后缀 + 文件头警告 + gitignore 缓解,Dart
  `.g.dart` 同型);build.rs 与编辑器**双写者**需要约定(见 §7 坑 3)。

### (b) 自建薄 LSP,只做模板域,Rust 域直接放弃

- 能做:标签/属性名补全(属性表在 `codegen.rs`/`template.rs` 里是现成的)、
  `class="…"` 补全(`<style>` 块里的类名已解析)、组件名与 `$props` 字段补全
  (`PropsRegistry`,`lib.rs:79` **已经存在**,build 的第一遍就在建)、块结构折叠、
  未闭合标签实时诊断。这些**全部零依赖 r-a**,而且质量上限比转发路线高(我们比 r-a 更懂 `.sv`)。
- 工作量:**2–4 人周**(tower-lsp 或 r-a 团队的 `lsp-server` crate + 复用 sv-compiler 的 parser)。
- **裁决:不作为替代品,作为 a1 之后的附加项**。理由很硬:它一行也没有缓解"最大悬置"
  ——`.sv` 里的 **Rust 表达式**没有类型信息。做完 (b) 之后,用户在 `{count.| }` 处按
  Ctrl+Space 依然什么都没有。把 4 人周花在这里而不花在诊断上,是拿好看的换有用的。
- 但有一条**现在就该顺手做**:`PropsRegistry` 已经存在,意味着 **"组件 props 拼错"
  这类错误已经能在编译器域精确报 .sv 行列**(测试 `component_call_with_props_and_default`
  已验证)。这是 (b) 的价值在**不做 LSP** 的前提下就能兑现的部分——继续加厚编译器域诊断,
  比做 (b) 的 LSP 壳更划算。

### (c) 完整 Volar 式虚拟文件

- 需要:虚拟文档生命周期、`VirtualCode`/`mappings` 抽象、能力标记、自持或代理宿主分析器、
  编辑器无关的 LSP server、双 r-a 实例(内存 GB 级/实例)。
- 工作量:调研 07 估 S4 = 8–16 人周,**在 a1 之上**。
- **裁决:不做,而且不是"以后再说"式的不做**。原因是 M8 已经证明**虚拟文档这一层是多余的**
  ——Volar 之所以要造虚拟文档,是因为 `.vue` 的 TS 代码在磁盘上**不存在**;而我们的生成
  文件**本来就是真文件,r-a 本来就在分析它**。为了一个已经解决的问题引入一整套框架,
  是把调研 07 §1.1 里 Volar 三次架构大改的学费重付一遍。

### 推荐:**(a)**,按 a0 → a1 推进,(b) 作为 a1 之后的附加,(c) 明确排除

一句话理由:**M8 把 (a) 的地基白送了,于是 (a) 的第一档只剩"搬运"这一层薄纸;
(b) 解决不了真痛点;(c) 在解决一个我们没有的问题。**

---

## 6. 问题五:分步落地、人周与验收

**总原则:第一步必须在 1–2 人周内出可感知的东西**,否则这件事永远排不上
(这是本仓库 R1–R4 的既有节奏,不是空话)。

| 阶段 | 内容 | 人周 | 置信度 | 退出标准(测试名) |
|---|---|---|---|---|
| **P0** | span map:虚拟行号 provenance(`parse_expr` 一处)+ 锚点并行走 + `.rs.map` sidecar + script 块偏移漂移表 | **1–1.5** | **高**(机制已在 %TEMP% 跑通,见 §8) | `map_roundtrip_all_examples`、`map_anchor_walk_is_total`、`map_survives_prettyplease_reflow`、`map_script_offsets_after_rune_replace` |
| **P1** | `sv check`:cargo JSON 解析 + 重映射 + human 输出 + 全部降级路径 | **1.5–2.5** | 中高 | `check_remaps_type_error_to_sv`、`check_never_drops_diagnostic`、`check_degrades_on_glue`、`check_drops_suggestion_in_glue`、`check_rejects_stale_map` |
| **P2** | TextMate grammar + language-configuration(**并行,不占 LSP 预算**) | 0.5–1 | 高 | 人查 + `grammar_snapshot`(用 `vscode-tmgrammar-test` 的 fixture) |
| **P3** | VS Code 扩展 · a0 诊断搬运 | 1–2 | 中 | `ext_diag_relocation`(单测重映射纯函数)+ 手测清单 |
| **P4** | a1:生成文件落盘 + hover/goto 正反向转发 | 3–5 | **低** | `map_bidirectional_roundtrip`、`lsp_hover_smoke`(#[ignore]) |
| **P5** | 补全(无 auto-import) | 3–6 | 低 | — |
| — | 通用 LSP server(Neovim/Helix/Zed) | 6–12 | 低 | **第一年不做** |

**"日常可用"= P0–P4 ≈ 7–11.5 人周**,低于调研 07 的 9–16。**差额的来源必须诚实说清**:
① M8 让"生成文件被 r-a 索引"从工作量变成既成事实;② 我们砍掉了虚拟文档层与通用 LSP
server;③ 只做 VS Code、只做只读。**任何一条不成立,估算作废**——尤其第③条:一旦有人
要 Neovim 支持,直接 +6–12 人周,那是另一个决策。

### 6.1 第一步的具体切片(1–2 人周,必须出东西)

**只做 P0 的表达式部分 + P1 的 human 输出**,刻意砍掉:script 块映射(第一版降级到
"整个 script 块 → 一个包络段")、json/annotations 输出、suggestion 处理。

**可感知的交付物**:在 `examples/counter-sfc/src/Counter.sv` 里把 `{count}` 改成
`{count + "x"}`,然后:

```
$ cargo run -p sv-check
error[E0308]: cannot add `&str` to `i32`
  --> examples/counter-sfc/src/Counter.sv:12:38
   |
12 |   <text font-size="20">Count: {count + "x"} · 双倍 = {double}</text>
   |                                      ^ no implementation for `i32 + &str`
```

—— 与今天的对照(形态取自 M6 的同型实测,行号未逐一核对):
`--> <target>\debug\build\counter-sfc-<hash>\out\counter.rs:47:31`,后面跟着**生成代码**的
摘录行。**这个前后对照就是这件事能不能排上号的全部说服力,应该作为 PR 描述的第一张图。**

### 6.2 LSP 的验收怎么自动化(这是本节最容易糊弄的地方)

**关键手法:把每个 LSP 特性劈成"位置换算"(纯函数)与"协议转发"(IO)两半,
把 95% 的验收压在前一半。**

1. **映射的自校验 golden(不需要人工维护 golden 文件)**——最强的一条:
   对每个 `examples/**/*.sv`,生成 `.rs` + `.map`,然后断言
   **`gen_text[seg.gen_start..seg.gen_end] == sv_text[seg.sv_start..seg.sv_end]`**
   对 `tokens[]` 里每一段成立(token 文本两侧必然逐字相等——这正是"逐字区"的定义)。
   任何一处映射错位当场红,而且**不需要任何人工标注**。测试名 `map_roundtrip_all_examples`。
2. **锚点全覆盖断言**:`pre.len() == post.len()` 且逐个文本相等,否则 fail
   (测试名 `map_anchor_walk_is_total`)。这条同时是 prettyplease 升级的哨兵——
   它哪天改了打印策略,这个测试先红,而不是用户先遇到错位的诊断。
3. **重排版鲁棒性**:构造一个必然触发折行的用例(一行 40+ 汉字 + 深嵌套),断言映射不变
   (`map_survives_prettyplease_reflow`)。这是 §3.2 那个真问题的回归卫兵。
4. **诊断重映射的 fixture 双档**:
   - **快档(进 `cargo test`)**:把 rustc JSON **录制成 fixture**(`tests/fixtures/*.json`),
     只测重映射逻辑,毫秒级、无 cargo 依赖。
   - **慢档(CI 单独 job)**:`tests/fixtures/bad_*.sv`,每个文件顶部一行
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
诊断仍然指到 `.sv` 的正确元素上。**这个降级必须在 P0 就实现,而不是等失配了再写**,
否则失配当天整条管线不可用。

### 卡死点 2:胶水区命中率过高 —— **这是真正该止损的信号**

**症状**:P1 的 fixture 库跑起来,发现**真实类型错误里超过一半落在胶水区**
(尤其 runes 改写产物:`__sv_rhs`、`update(|__v| …)`、`bind_text` 闭包)。
**含义**:"精确回映"是自欺,用户看到的多数是"表达式级 + 一句抱歉"。
**止损动作**:**立刻停掉 LSP 方向的一切投入**,把预算转到 **codegen 的错误落点纪律**
——调研 07 §4 的"每个用户表达式先 `let` 绑定到期望类型显式标注的局部变量"。
理由:落点纪律**同时**改善 rustc 直接输出与我们的映射,而映射只改善后者。
**量化判据**:fixture 库(≥30 个真实错误场景)里 `kind=exact` 占比 **< 60%** 就触发。

### 卡死点 3:双写者(build.rs vs 编辑器)—— a1 才会遇到,但会很恶心

**症状**:编辑 `.sv` 时 cargo 反复重编译、文件锁错误、r-a 索引抖动、诊断闪烁。
**根因**:落盘方案下,build.rs 与扩展都在写同一个 `_sv.rs`,而 r-a 的 watcher 在中间。
**止损**:编辑期**只用内存 overlay 不落盘**(保存 `.sv` 时才落),或干脆退回 a0(OUT_DIR)。
**预防**:codegen 输出必须**确定性 + 内容不变时不写盘**(`if new == old { return }`)
——这一条应该在 P0 就加进 `lib.rs:214` 的写盘处,成本一行,收益是省掉整类玄学问题。

### 卡死点 4:位置编码(UTF-16)在 CJK 上翻车 —— 本仓库尤其致命

**症状**:`.sv` 里含中文的行,诊断的波浪线偏移几个字符;错位量恰好等于该行中文字数。
**根因**:LSP 默认 `utf-16` code units;rustc 给字节;我们的 map 用字节;
**本仓库所有示例都是中文界面**(`create_text("sv 计数器(.sv 编译器路线)")`),
所以这个坑一定会踩,而且只在中文行踩——**很容易被当成"偶发"而漏掉半年**。
**止损**:不需要止损,需要**预防**:map 内部一律字节;所有"字节 → LSP Position"的换算
收敛到**一个函数**;测试里放一条**故意的中文行**做回归(测试名 `map_cjk_utf16_columns`)。

### 卡死点 5:r-a 对高频 didChange 的增量性能

**症状**:编辑 `.sv` 时诊断延迟 > 2s,或 CPU 常驻高位。
**已知数据**:483 行落盘生成文件上,`textDocument/hover` **3 ms**(实测 M10,单次)。
`didChange → publishDiagnostics` 的往返**未核实**——探针在这一步里 r-a 退出了
(stderr 只有 notify 的路径警告,无 panic),没有复现出稳定数字。**列为 P3 首日冒烟项**。
**止损**:退到"保存时才重生成"(`.sv` 保存 → 重生成 → r-a 自然重算),
体验从 keystroke 级降到保存级——**这依然远好于今天**,所以这条不构成路线级风险。

### 卡死点 6:上游形态变化

r-a 无插件 API、无官方途径复用编辑器里已跑的实例(调研 07 §2.5),我们的耦合面只有
**标准 LSP + cargo JSON** 两个最稳的接口 —— 这已经是最小耦合姿势。
**但有一条本方案自己引入的新耦合必须记账:`prettyplease` 的打印行为**(§3.2 第二步依赖
"token 序列保持")。它是 dtolnay 的库、语义稳定,但**我们的映射正确性挂在它身上**。
`map_anchor_walk_is_total` 就是这条耦合的保险丝。

### 什么信号出现就该整体止损、退回"只读特性 + 好错误信息"

**任一条成立即止损**:

1. **P1 交付后,fixture 库里 ≥ 80% 的日常错误已经能落到正确的 `.sv` 行**
   —— 这时 LSP 转发的边际收益只剩"补全"一项,而补全是 P5(3–6 人周、置信度低)。
   **这是"成功导致的止损",最可能发生,也最该被坦然接受**:把剩下的预算给编译器域诊断
   (did-you-mean、属性名拼错、`$props` 类型不匹配)与文档,收益/成本远高于继续做 LSP。
2. **卡死点 2 触发**(exact 命中率 < 60%)——方向错了,先修 codegen 落点纪律。
3. **P3 做完两周内没人用**(团队内部 `.sv` 编写者仍然习惯"跳进生成文件改")
   —— 说明痛点判断错了,真实痛点在别处(可能是 `.sv` 语法本身或组件模型)。
4. **单人维护面告警**:DESIGN.md §6 风险 5 已经列了"单人/小团队维护面过宽"。
   LSP 是**长期税**而非一次性成本(调研 07 §0.4)。如果 R5(鸿蒙)开工与 LSP 撞期,
   **LSP 让路**——因为 `.sv` 还有 `view!` 宏这个双前端退路(ADR-2:"双前端策略把这笔税
   从赌注变成选项"),而鸿蒙没有退路。

**退回后的止血包**(全部落在 P0–P2 之内,即便止损也已到手):
`sv check` 的精确诊断 + TextMate 高亮 + 生成代码可读性(应当在 P0 顺手加上
**节点锚点注释** `// <button onclick> Counter.sv:14:5` ——map 里已经有这些区间,
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
| M5 | **端到端映射原型跑通**:一行 28 汉字被 prettyplease 折成 4 行,后续表达式仍精确映射到 .sv 偏移 200/208/300/308 | §3.2 三步串起来的 60 行原型 |
| M6 | rustc JSON:`file_name` = 生成文件,带 `byte_start/byte_end`,**`expansion: null`**(即使错误在 `println!` 参数里) | 临时 crate:build.rs 写 `$OUT_DIR/counter.rs`(埋 E0599 + E0425),`main.rs` `include!`,跑 `cargo check --message-format=json` |
| M7 | cargo 吐 `{"reason":"build-script-executed", …, "out_dir":"…"}` → OUT_DIR 可确定性发现 | 同上,过滤该 reason |
| M8 | **rust-analyzer 把生成文件的诊断发布到生成文件自己的 `file://` URI,位置正确** | 自建 LSP client(Python,Content-Length 分帧):`initialize`(带 `cargo.buildScripts.enable`)→ `initialized` → `didOpen(src/main.rs)` → 收 `textDocument/publishDiagnostics` |
| M9 | 11 个 `.sv` 的生成放大 6–8×(行);单文件编译 0.47–5.25 ms(release,50 次均值) | 临时 crate 调 `sv_compiler::compile_sv` 循环计时 |
| M10 | 483 行落盘生成文件上 `textDocument/hover` **3 ms** | 同 M8 的探针,改发 hover |

**未核实清单(不要当成事实引用)**:

- `didChange → publishDiagnostics` 的往返延迟(M10 的探针在这一步 r-a 退出,原因未查明)。
- VS Code 扩展 API 能否读到**未打开文件** URI 上的诊断(a0 的关键前提,P3 首日冒烟)。
- `vscode.executeHoverProvider` 等命令在生成文件 URI 上的行为细节(调研 07 §6 风险 3)。
- 让 r-a 的 `check.overrideCommand` 直接输出 `.sv` 坐标的假 rustc JSON 是否可行
  —— 沿用调研 07 §4 的判断(r-a 会因 `.sv` 不在其 VFS 而丢弃或告警),**本方案未测**,
  且**不打算走**:映射责任应留在我们这层。
- 落盘方案下 build.rs 与编辑器双写者的真实表现(卡死点 3)。
- 本机 stable 工具链未装 `rust-analyzer` component;所有 r-a 实测用的是 VS Code 扩展
  自带的 0.3.2971 二进制。CI 上的版本行为可能不同。

---

## 9. 与既有文档的关系

- **DESIGN.md §6 风险 1**:本方案是它的落地稿。若 P0+P1 落地,该风险的描述应改为
  "rustc 诊断已重映射回 `.sv`;只读 LSP 特性(hover/goto)未做",杀伤力从第 1 位下调。
- **ADR-2(双前端共存)**:本方案不改变 ADR-2,反而给它加了一条实证——
  `view!` 宏路径的 span 精度是免费的,`.sv` 路径的 span 精度要花 7–11.5 人周买。
  这正是 ADR-2 "双前端把这笔税从赌注变成选项" 的定量版本。
- **调研 07**:结构与结论沿用,三处修订已在 §0 列明(M8 好消息、逐字区假设不成立、
  虚拟文档层多余)。
- **调研 06(build.rs/OUT_DIR 定案)**:a0 完全兼容现状;a1 会动这条决策(落盘),
  届时需要在 DESIGN.md 里补一条 ADR 或修订 ADR-2 的"构建集成定案"段。
