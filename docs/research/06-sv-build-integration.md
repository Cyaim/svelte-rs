# 06 · 外部 .sv 文件编译进 Rust 构建的集成机制对比

> 调研主题:如果走"独立编译器 + .sv 单文件组件"路线(script 块 = Rust + runes,template 块 = Svelte 语法),
> .sv → Rust 的代码生成应该怎样接入 cargo 构建?四种机制逐一评估,先例深挖,错误定位 hack 全清单,最终给主路线 + 备选 + 骨架代码。
>
> 调研日期:2026-07-17。关键事实(tracked_path/bindeps 稳定化状态、rust-analyzer 对 OUT_DIR 的支持、
> askama/slint/tauri 的具体实现)已联网核实并抽查源码;仅凭训练数据的结论在 §6 单独标注。

---

## 0. 结论先行(TL;DR)

1. **主路线:(a) build.rs + `sv-build` 库 + OUT_DIR 代码生成 + `include!`(slint-build 同构)**。
   决定性理由有三:
   - 路线 (b)(proc-macro 读外部文件)的重编译追踪**没有官方机制**:`proc_macro::tracked::path` 至 2026-04 仍在
     unstable 设计阶段(rust-lang/rust#99515 仍 open,2025-12 libs-api 才刚敲定相对路径语义,无 FCP),
     只能靠 askama 的 `include_bytes!` 注入 hack;
   - .sv 组件之间有 **import 依赖图**(组件引用组件、样式/资源引用),build.rs 里的编译器一次看到全图,
     可以精确产出依赖清单(`cargo::rerun-if-changed` 逐文件),而每个宏调用点各自展开的模型天然做不好跨文件分析与缓存;
   - rust-analyzer 对 build script + OUT_DIR `include!` 的支持已是成熟默认行为(`cargo.buildScripts.enable`
     默认 `true`),生成代码可索引、可跳转。
2. **错误落点是本路线最大的税,但有完整缓解栈**(§3):rustc 没有 `#line` 等价物(核实:2026 年仍无,
   `--remap-path-prefix` 只改路径文本不改行号)。分三层解决:
   ① 模板域错误(语法/未知元素/bind 到只读)**永远不出 sv 编译器**,在 build.rs 里带 .sv 路径行列自报(Slint 模式);
   ② 漏到 rustc 的只剩"用户 Rust 表达式类型错",它们落在**可读的生成文件**里(`include!` 的 span 指向 OUT_DIR
   文件本身,行号真实、r-a 可跳转)——生成文件必须按"第一公民中间产物"标准做:格式化 + `// sv:` 锚点注释 + 用户表达式逐字保留;
   ③ 终局:`cargo sv check` 诊断重映射包装器(sidecar span map + 重写 rustc JSON 诊断),接
   `rust-analyzer.check.overrideCommand` 后诊断可以直接标注在 .sv 文件里——这是 JS 世界 sourcemap 的 Rust 等价物,
   机制全部基于稳定接口,但**无现成先例**,需要 spike。
3. **路线 (c)(预生成提交)不是主路线,是发布通道**:crates.io 上的**组件库**用 `cargo sv vendor` 预生成
   .rs 提交/打包发布(sqlx offline / windows-rs 模式),消费者不付 sv 编译器的 build-dep 成本,docs.rs 友好;
   CI 用 `--check` 防漂移。app crate 内环仍走 build.rs。
4. **路线 (d) 排除作为集成基础**:cargo artifact-dependencies 至今仍是 `-Z bindeps` unstable(cargo#9096 open,
   2025 还在修传播 bug),且它解决的是"怎么拿到编译器二进制",不是集成本身;自定义 `cargo sv` 子命令
   (对标 `dx`/`trunk`/`cargo tauri`)是 **DX 编排层**而非集成机制,叠加在 (a) 之上做 watch/热重载/打包,
   铁律:**plain `cargo build` 必须始终自足**。
5. 与既有 proc-macro `view!` 路线**不冲突**:同一个编译器库(parser/IR/codegen,现 sv-macro 内三件套上提为
   `sv-compiler`)挂两个前端——`view!` 内嵌宏与 `.sv` 文件 build.rs 前端并存(uniffi UDL/proc-macro 双前端、
   Slint macro/build/interpreter 三前端的成熟先例)。`.sv` 的 script 块整体逐字透传进生成文件,r-a 可完整索引。

---

## 1. 四种机制逐一评估

### 1a. build.rs + 代码生成到 OUT_DIR + `include!`

**机制**:crate 声明 `build-dependencies: sv-build`;build.rs 调 `sv_build::compile_dir("ui")`;编译器把
`ui/**/*.sv` 生成为 `$OUT_DIR/sv-gen/*.rs`;用户代码 `sv::include_components!()`(内部即
`include!(env!("SV_INCLUDE_GENERATED"))`)。这正是 slint-build 的成熟形态,实现细节已抽查源码
([slint api/rs/build/lib.rs](https://github.com/slint-ui/slint/blob/master/api/rs/build/lib.rs)):

- 编译器返回"生成本文件用到的**全部输入文件**清单"(含 import 的 .slint、图片资源),逐个
  `println!("cargo:rerun-if-changed={path}")`;另有一组 `cargo:rerun-if-env-changed=SLINT_*`;
- 生成文件路径通过 `cargo:rustc-env=SLINT_INCLUDE_GENERATED=...` 传给 rustc,`slint::include_modules!()`
  展开为 `include!(env!("SLINT_INCLUDE_GENERATED"))` —— 用户不用写 `concat!(env!("OUT_DIR"), ...)` 咒语;
- .slint 里的错误由编译器 `diag.print()` 打到 stderr、build script 返回 Err —— cargo 在 build script
  失败时会显示其输出,错误落点是 .slint 的真实路径+行列。

**变更检测与增量**(语义已按 [Cargo Book build-scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html) 核实):

- `cargo::rerun-if-changed` 基于 mtime 与上次运行的缓存时间戳比较;指向**目录**时递归扫描整个目录。
- 若 build script **不发任何** rerun-if 指令,cargo 保守地在"包内任何文件变化"时重跑——所以必须发,否则每次
  `cargo build` 都重跑编译器。
- 陷阱一:**逐文件清单检测不到"新增文件"**。`ui/` 下新建 `Foo.sv` 不在上次清单里 → build script 不重跑。
  解法:逐文件清单之外**再发一条目录级** `cargo::rerun-if-changed=ui`(目录递归扫描覆盖新增/删除)。
- 陷阱二:**内容不变也写文件 = 白付一次全 crate 重编**。build script 重跑后若无脑重写 OUT_DIR 文件,mtime
  变化会触发 rustc 重编。解法:生成前比较内容,不变则不写(经典 codegen build script 优化,sv-build 必做)。
- 增量粒度:.sv 改 → build script 重跑(sv 编译器对单文件改动应只重新生成受影响的 .rs)→ rustc 重编该 crate。
  crate 内 rustc incremental 正常生效。粒度与 proc-macro 路线一致(都是 crate 级),差别只在多付一次
  build script 进程启动(sv 编译器须保持轻快,目标 <100ms 空转)。

**workspace 多 crate 缓存**:

- `sv-build` 作为 build-dependency 按 **host** profile 编译,整个 workspace 只编一次,各 crate 的 build script 共享;
- **交叉编译天然友好**(对鸿蒙关键):build.rs 永远在 host 上跑,`aarch64-unknown-linux-ohos` 目标构建时
  编译器不需要为目标平台交叉编译——proc-macro 同理,两条路线打平;
- 工程建议:把 .sv 密集的 UI 拆成叶子 crate,.sv 改动只重编该 crate 及下游;
- 代价:sv-compiler 自身改版本 → 所有 build script 重跑 → 全部 UI crate 重编(proc-macro 路线同样)。

**IDE(rust-analyzer,已核实 [r-a Book Configuration](https://rust-analyzer.github.io/book/configuration.html))**:

- `rust-analyzer.cargo.buildScripts.enable` **默认 true**:r-a 会跑 build script、注入 OUT_DIR 与 cfg,
  OUT_DIR 里的生成代码被完整索引——生成的组件类型/函数在用户 Rust 代码里有补全、跳转、类型提示;
  goto definition 会跳进生成文件(所以生成文件要可读,见 §3)。这条能力就是当年
  [r-a#1964](https://github.com/rust-lang/rust-analyzer/issues/1964)(`include!(concat!(env!("OUT_DIR"),…))`
  should work)的落地。
- `buildScripts.rebuildOnSave` 默认 true,但注意其语义是"**proc-macro 或 build script 源码**变化时重跑"——
  编辑 .sv **不会**触发 r-a 重跑 build script。实践上:改 .sv 后需要一次 cargo check(保存任意 Rust 文件触发
  flycheck,或手动)r-a 的世界观才刷新。这个"数据文件编辑不被 IDE 感知"的时滞是 (a)(b) 共有的缺陷,
  (b) 更隐蔽(连生成文件都看不到)。终局解法在 `cargo sv dev` watch(§1d)与 sv LSP(远期)。
- .sv 文件内部:r-a 零支持(它不是 Rust 文件),与 04 号报告对路线 (b) 的判断一致——模板内表达式补全
  要等 sv 自己的 LSP(svelte2tsx 模式,§2.7),这是选独立编译器路线本身的成本,四种集成机制都一样,不构成机制间差异。

**错误信息落点**:两级。模板域错误在 build script 内自报(落点完美:.sv 路径+行列,可做彩色 miette 风格渲染);
用户 Rust 表达式的类型/借用错误落在 OUT_DIR 生成文件(行号真实、可跳转,可读性靠 codegen 纪律保证,缓解栈见 §3)。

**发布体验(crates.io)**:app crate 无问题(build-dep 不传染给依赖者——只有构建**该包本身**时才需要)。
真正的痛点是**发布组件库**:库 crate 若在自己的 build.rs 里编 .sv,所有消费者都要构建 sv-build
(slint 的现实痛点:slint-build 拉全套 slint compiler,冷构建以分钟计)。缓解:组件库走路线 (c) 预生成后发布,
见 §1c;sv-build 自身控制依赖面(不需要 syn——它解析的是 .sv 不是 Rust;codegen 直接产出字符串,
格式化用 prettyplease 可选)。

**评分小结**:变更检测 ★★★(依赖清单精确可控)/ 增量 ★★☆ / IDE ★★☆(Rust 侧全通,.sv 内空白)/
错误落点 ★★☆(两级模型,可持续改进到 ★★★)/ 发布 ★★☆(app 好,库须配合路线 c)。

### 1b. askama 风格:proc-macro/derive 读外部文件

**机制**:`#[derive(Template)] #[template(path="foo.html")]`(askama)或假想的
`sv_component!("Counter.sv")`。宏展开时用 `fs::read` 读文件、就地生成代码。

**重编译追踪——本路线的阿喀琉斯之踵(全部联网核实)**:

- cargo 对 proc-macro 在展开期读的文件**一无所知**:dep-info 只记录 rustc 自己通过 `include_str!`/
  `include_bytes!`/`include!` 读的文件。宏 `fs::read` 的文件变了,cargo 认为什么都没变 → **陈旧构建**。
- 官方解法 `proc_macro::tracked::path` / `tracked::env_var` 的现状(核实
  [rust#99515](https://github.com/rust-lang/rust/issues/99515),state: open,44 评论,最后更新 2026-04-07):
  feature gates 现为 `proc_macro_tracked_env` / `proc_macro_tracked_path`;API 已从 `proc_macro::tracked_*`
  迁到 `proc_macro::tracked::*` 模块;2022 年 m-ou-se 即指出"公共 API 几乎没讨论过,未决设计问题一堆";
  2025-12-09 libs-api 会议才刚决定"path 参数按 `File::open` 语义、相对 CWD 解释";2026-04 还在同步 issue
  描述与实现。**判断:短期(1–2 年)内别指望 stable,不能作为架构前提。**
- 生态的实际解法 = **askama 的 `include_bytes!` 注入 hack**(已抽查
  [askama_derive/src/generator.rs](https://github.com/askama-rs/askama/blob/master/askama_derive/src/generator.rs)
  源码确认):生成代码里对每个模板文件塞一条
  ```rust
  const _: &[core::primitive::u8] = ::core::include_bytes!(#path);
  ```
  源码注释原话:*"Make sure the compiler understands that the generated code depends on the template files."*
  ——把外部文件"洗"进 rustc dep-info,cargo 从而追踪它。**有效但是 hack**:模板文件内容被字面嵌进
  rmeta(体积)、且要求宏能拿到稳定的绝对路径。
- 路径解析的老大难已缓解:`proc_macro::Span::{file, local_file, line, column}` **已在 Rust 1.88.0
  (2025-06-26)稳定**([公告](https://blog.rust-lang.org/2025/06/26/Rust-1.88.0/)、
  [PR#140514](https://github.com/rust-lang/rust/pull/140514))——宏在 stable 上可拿到调用点源文件的真实磁盘路径,
  "模板路径相对于当前源文件"终于不用 `CARGO_MANIFEST_DIR` 猜。这是 2025 年后路线 (b) 唯一实质变好的地方。
- 反面教材:**tauri 的 `generate_context!`** 读 `frontendDist` 目录嵌资产,但不做任何追踪 → 前端产物更新而
  Rust 侧不重编,发布出陈旧资产,社区靠手工在 build.rs 加 `cargo:rerun-if-changed=../dist` 兜底
  ([案例分析](https://takazudomodular.com/pj/zudo-tauri/docs/deployment/cargo-cache/))。宏读外部文件不追踪 = 必然事故。

**IDE**:宏展开对 r-a 照常工作(生成的组件 API 有补全)。但两个减分项:
- r-a 缓存宏展开,.sv 变化它感知不到(`rebuildOnSave` 只看 proc-macro **源码**),过期比 (a) 更隐蔽
  ——(a) 至少还有个能打开看的生成文件;
- 生成代码不落盘,类型错误指到宏调用点那**一行属性**上,secondary span 全部糊在一起,无法"跳进生成代码看上下文"。

**错误落点**:比 (a) 差。span 无法指向外部文件(`proc_macro::Span` 不能伪造任意文件位置,这是 API 设计使然),
只能走 askama 的路:错误**消息文本**里拼 `template.html:12:8`(已核实 askama `FileInfo` 的 Display 实现,
输出 `{file_path}:{row}:{column}`),而诊断本身仍挂在 derive 那一行。用户体验:能看懂,但 IDE 不会在 .sv 里标红。

**增量**:文件变化(经 include_bytes hack)直接触发该 crate rustc 重编,**不需要 build script 进程**,
比 (a) 少一跳——这是 (b) 唯一的结构性优势。但每次编译所有宏调用点全量重新展开、重复解析共享的 import,
无跨调用点缓存(proc-macro 无合法的持久化缓存点),.sv 数量大了以后是 O(N) 全量解析。

**判断**:适合 askama 那种"每文件独立、无 import 图、表达式是自制迷你语言"的模板。sv 的 .sv 有组件依赖图、
表达式是真 Rust(错误落点需求高)、要做全局组件解析——(b) 的每个短板都正好戳中。**不作主路线**;
可留一个 `sv!("Inline.sv")` 单文件快捷宏作为糖(实现上复用同一编译器库 + include_bytes hack,几十行)。

### 1c. CLI / cargo-xtask 预生成 .rs 提交进仓库(sqlx offline / uniffi / windows-rs 风格)

**先例(核实)**:
- **sqlx offline**:`cargo sqlx prepare` 把查询元数据写进 `.sqlx/` 目录提交进版本库,宏离线读取;
  `--workspace` 生成 workspace 级单一目录;`prepare --check` 供 CI 验证不漂移;`DATABASE_URL` 存在时优先走
  在线([sqlx-cli README](https://github.com/launchbadge/sqlx/blob/main/sqlx-cli/README.md))。
  注意 sqlx 提交的是**元数据**而非生成代码,宏仍在每次编译时跑——它解决的是"构建时不要外部服务",与我们
  "构建时不要重型编译器"同构。
- **uniffi**:外语言 bindings 由 `uniffi-bindgen` CLI 显式生成(不进 cargo 构建);Rust 侧脚手架历史上走
  UDL + build.rs `generate_scaffolding`,官方生态现已明确**建议尽量用 proc-macro 前端**
  ([uniffi user guide](https://mozilla.github.io/uniffi-rs/latest/)、
  [uniffi-bindgen-java](https://lib.rs/crates/uniffi-bindgen-java):*"We highly recommend you use UniFFI's
  proc-macro definition instead of UDL where possible"*)。
- **windows-rs**:`windows-bindgen` 预生成的 bindings 直接提交/发布,消费者零 build-dep(训练数据,§6 标注)。
- **prost/tonic 的混用形态**:`tonic_build` 默认 OUT_DIR,但提供 `.out_dir("src/generated")` 把生成码指进源码树
  提交——大量项目为摆脱 protoc 外部依赖这么干([tonic-build docs](https://docs.rs/tonic-build/latest/tonic_build/))。

**优点**:消费者**零 build-dep、零编译器构建成本**;docs.rs / 离线 / 审计 / vendoring 全友好;
生成的 .rs 就在 `src/` 里,**r-a 当普通源码索引,IDE 体验反而是四条路线里最好的**;错误落点=真实源码文件。

**缺点**:漂移风险(改 .sv 忘了重新生成是日常事故,必须 CI `--check` 兜底);diff 噪音大;贡献者要装工具;
**作为主内环 DX 不可接受**——UI 迭代是秒级循环,每次改 .sv 手跑一条命令直接杀死心流。

**判断**:不是集成主路线,而是**发布与协作通道**:
`cargo sv vendor --out src/generated`(+ CI `--check`)用于 crates.io 组件库发布与"消费者不想要 build-dep"
的场景。工程上几乎免费——同一个 sv-compiler 库换个输出目录而已,值得从第一天就把"输出路径可定制"留好
(slint 的 `compile_with_output_path` 就是为此存在的)。

### 1d. cargo artifact-dependencies / 自定义 cargo 子命令

**artifact-dependencies(bindeps)现状(核实)**:仍是 nightly-only `-Z bindeps`
([cargo#9096](https://github.com/rust-lang/cargo/issues/9096) open;
[Cargo Book unstable](https://doc.rust-lang.org/cargo/reference/unstable.html) 仍列为 unstable;
2025 年还在修 artifact 依赖向 proc-macro/build-dep 错误传播的 bug
[cargo#15788](https://github.com/rust-lang/cargo/pull/15788))。即便稳定,它解决的也只是
"build.rs 如何依赖 sv 编译器**二进制**"——而我们把编译器做成库直接链进 build.rs 更好:结构化传参与诊断、
无需子进程协议、版本天然由 Cargo.toml 锁定。**排除作为集成基础。**

**自定义 cargo 子命令**(`cargo sv dev` / `cargo sv build`,对标 Dioxus `dx`、`trunk`、`cargo tauri`):
要认清它**不是集成机制,是 DX 编排层**——watch、dev server、热重载推送、打包签名。tauri 的教训(§2.6):
`beforeBuildCommand` 由 CLI 编排,但**cargo 单独构建也不能坏**(顶多用旧资产)。对 sv:
`cargo build` 必须永远能出正确二进制(build.rs 保证),`cargo sv dev` 在其上加 watch + 模板热推送
(04 号报告 M3/M4 的热重载通道)。把"正确性"和"爽"分开供给。

### 对比矩阵

| 维度 | (a) build.rs + OUT_DIR | (b) 宏读外部文件 | (c) 预生成提交 | (d) bindeps / CLI |
|---|---|---|---|---|
| 变更检测 | ★★★ rerun-if-changed 逐文件+目录兜底,语义清晰 | ★★☆ 靠 include_bytes hack;tracked_path 未稳定 | ★☆ 人肉+CI check | (不适用,编排层) |
| 新增文件感知 | ★★★(目录级指令) | ★☆(新 derive 调用点本身是 Rust 改动,尚可) | ☆ 手跑 | — |
| 增量粒度 | crate 级 + build script 一跳;内容比较可免空重编 | crate 级,少一跳但宏全量重展开 | 最优(纯 Rust 增量) | — |
| workspace 缓存 | 编译器 host 编一次共享;交叉编译免费 | 同左(proc-macro host 编译) | 消费者零成本 | bindeps 仍 -Z |
| IDE(Rust 侧) | ★★★ OUT_DIR 索引成熟(r-a 默认开) | ★★☆ 展开可见但生成码不落盘 | ★★★ 普通源码 | — |
| IDE(.sv 内) | ☆(四路线同,待 sv LSP) | ☆ | ☆ | — |
| 错误落点 | 模板错自报 + Rust 错落可读生成文件;可升级 check 重映射 | 全糊在宏调用点一行;消息里拼 file:line | 落真实源码,最好 | — |
| 发布(crates.io 库) | 消费者付 build-dep 编译成本(slint 之痛) | 消费者付 proc-macro 编译成本(略轻) | ★★★ 零成本 | crates.io 不支持二进制分发 |
| 实现成本 | 低(slint 模式照抄) | 低 | 低(共用编译器库) | 中(CLI 后置) |

---

## 2. 先例深挖:各家怎么解决增量与 IDE

### 2.1 slint-build —— 本报告推荐路线的原型

核心三件套(源码核实,见 §1a):依赖清单→逐文件 `rerun-if-changed`;`cargo:rustc-env=SLINT_INCLUDE_GENERATED`
+ `include_modules!()` 消灭用户侧咒语;DSL 错误在 build script 内自报。IDE 缺口(.slint 文件内)用**独立
slint LSP** 补(живой preview、补全、跳转)——印证 04 号报告"路线 (b) 想要模板内体验最终滑向自建 LSP"的判断,
但注意 Slint 是把它当产品核心投入的;sv 的模板内嵌真 Rust,LSP 缺口形状不同(§2.7)。
另有 `compile_with_output_path` 显式支持"脱离 cargo 的输出定制"——即我们的路线 (c) 复用点。

### 2.2 askama —— 路线 (b) 的天花板样本

`include_bytes!` 注入(§1b 已引源码)解决追踪;错误用 `CompileError` + `FileInfo`(`file:row:column` 拼进
消息文本)解决"能看懂",解决不了"IDE 在模板里标红"。askama 能忍受 (b) 的根本原因:模板表达式是自制
Jinja 迷你语言,**类型错误在宏内自查自报**,极少漏到 rustc——sv 反之(真 Rust 表达式),错误落点需求高一个量级。
另一个可借鉴点:`askama.toml` crate 根配置文件(模板目录、语法定制)——sv 可对应 `sv.toml`(组件搜索路径、
主题常量注入),但注意宏读配置文件同样有追踪问题(askama 用同一 hack)。

### 2.3 prost / tonic —— OUT_DIR 模式的最大规模实战

`prost-build`/`tonic-build` 默认 OUT_DIR + `tonic::include_proto!`(与 slint 同构;macro 展开为
`include!(concat!(env!("OUT_DIR"), "/{package}.rs"))`)。两条教训:
- **外部工具链是万恶之源**:prost-build 依赖系统 `protoc` 后,NixOS/CI/Windows 上的 PROTOC 环境变量问题
  连绵不绝,逼出 [protox](https://github.com/andrewhickman/protox)(纯 Rust protobuf 编译器,专为在
  build.rs 里替掉 protoc)。sv-build 天然纯 Rust,此坑不存在,但要引以为戒:**永远别让 .sv 编译依赖
  任何非 cargo 分发的东西**(包括 Node、包括未来可能的资源处理工具);
- `.out_dir()` 逃逸口被大量用户用于提交生成码进 src——路线 (a)/(c) 混用是真实需求,API 第一天就该支持。

### 2.4 cxx-qt —— 混合分工样本

Rust 侧接口走 `#[cxx_qt::bridge]` proc-macro(r-a 可见、可展开),build.rs(`cxx-qt-build`)只负责
C++ 代码生成、Qt 链接、QML 模块(qmlcachegen AOT 编译 .qml 进 Qt Resource System)
([CxxQtBuilder docs](https://docs.rs/cxx-qt-build/latest/cxx_qt_build/struct.CxxQtBuilder.html))。
启示:**"Rust 可见面用宏、外部资产用 build.rs"的分工是被验证的**——对 sv 即:组件的 Rust API 面(类型、
props)尽量让 r-a 直接看到(生成码可索引即达成),重资产(样式、图片、未来的主题包)走 build.rs 数据通道。

### 2.5 uniffi —— 双前端并存的活证据

UDL 外部文件(build.rs `generate_scaffolding`)与 proc-macro 两个前端共享同一套内核,且官方生态方向是
proc-macro 优先(§1c 已引)。对 sv 的直接映射:`view!` 宏与 `.sv` build.rs 前端**共享 sv-compiler 核心**,
谁是"推荐入口"可以随生态反馈调整,架构上不必今天押注。

### 2.6 tauri —— 前端资产的反面 + 正面教材

反面:`generate_context!` 宏嵌 dist 资产但不追踪(§1b 已述,陈旧资产事故)。正面:**dev/release 双路径**——
dev 模式 WebView 直连 Vite dev server(`devUrl`),资产根本不进 Rust 构建;只有 release 才嵌入。
对 sv 热重载的启示:内环里 .sv 变更走**旁路数据通道**(模板序列化热推送,04 号报告 M3),
不必每次穿过 cargo;cargo 构建只需保证"下次冷构建正确"。

### 2.7 Svelte/Vite —— JS 世界的等价物与差距清单

Svelte 编译器以 Vite 插件形态挂在 transform 钩子上:**按模块**编译、Vite module graph 管依赖与失效、
HMR 边界组件级(Svelte 5 起 HMR 集成进 compileOptions;样式单独热替换可 100% 保组件状态,
[vite-plugin-svelte](https://github.com/sveltejs/vite-plugin-svelte))。IDE 是**独立 svelte-language-server**:
`svelte2tsx` 把 .svelte 变换成**虚拟 TSX** 交给 TypeScript 分析,再用 sourcemap 把诊断/补全映射回 .svelte
([Svelte & TypeScript](https://svelte.dev/blog/svelte-and-typescript))。
逐项对照 Rust 缺什么:
- transform 钩子 → build.rs(有,但粒度 crate 级 vs 模块级);
- module graph 失效 → rerun-if-changed(有,够用);
- sourcemap 诊断回映 → **rustc 无 sourcemap 概念,这是唯一的结构性缺口**,§3 的 check 包装器就是补这一块;
- svelte2tsx 虚拟文件 → r-a **不接受虚拟文件输入**,同款方案做不了;最近似可行形态:sv LSP 自己维护
  ".sv → 生成 Rust"映射,把 .sv 内 Rust 表达式区域的请求转发给 r-a 对**生成文件**的分析(LSP 转发代理)。
  远期项目,报告仅立此存照。

---

## 3. rustc 生成代码的错误定位:没有 `#line`,以及全部已知 hack

**事实(核实)**:rustc 至今没有 C 预处理器 `#line` 指令的等价物
([论坛长年讨论无进展](https://users.rust-lang.org/t/rust-equivalent-of-line-directive/44731));
`--remap-path-prefix` 只做**纯文本路径前缀替换**(诊断/debuginfo/宏展开里的路径显示),**不改行号**;
其细分控制 `--remap-path-scope` 仍 unstable([rustc book](https://doc.rust-lang.org/beta/rustc/remap-source-paths.html))。
`proc_macro::Span` 不能伪造指向任意外部文件的位置(伪 span 之路也堵死)。

已知 hack 全清单(按投入产出排序,前三条即推荐配置):

1. **`include!` 的天然行为就是半个解**:被 include 的 OUT_DIR 文件是真实文件,错误 span 落在**该文件内**,
   行号列号真实,r-a 点击可跳转。因此第一原则:**把生成文件当第一公民中间产物**——prettyplease 格式化、
   每个用户表达式独立成行、逐字保留(表达式 token 一个不改,用户搜自己写的代码能搜到)、顶部写清
   "GENERATED FROM ui/Counter.sv — do not edit"。用户看到错误 → 点进去 → 看到自己的表达式加一行锚点注释,
   已经是可接受的 DX 底线(prost/tonic/slint 用户十年来就活在这个水平)。
2. **锚点注释**:每段来自 .sv 的代码前注 `// sv: ui/Counter.sv:12:8`。零成本、grep 友好、
   与人肉调试兼容。比"行号对齐"技巧(在生成文件里垫空行让用户表达式行号与 .sv 一致)更稳——
   行号对齐在一个表达式展开为多处(如 `bind:` 的读+写)时必破,且脆弱;可作尽力而为的锦上添花
   (第一次出现处对齐),不作为承诺。
3. **模板域错误零泄漏原则**:sv 编译器自查一切模板域问题(语法、未知元素/属性、`bind:` 到只读 signal、
   import 失败、key 类型不符),带 .sv span 从 build.rs 报出。**能漏到 rustc 的只应剩用户 Rust 表达式自身的
   类型/借用错**。这决定了"错误落点税"的实际税基大小——Slint 把这做到了 100%(它不嵌宿主表达式),
   sv 做不到 100% 但能把高频错误全兜住。
4. **诊断后处理映射(`cargo sv check`)——sourcemap 的 Rust 等价物,终局方案**:codegen 时随每个 .rs 产出
   sidecar span map(生成文件 byte range → .sv byte range);包装器跑
   `cargo check --message-format=json`([rustc JSON 格式](https://doc.rust-lang.org/rustc/json.html)),
   凡 span 落在生成文件的,查表重写 file/line/col 与 rendered 文本再输出。
   接 IDE:`rust-analyzer.check.overrideCommand`(默认 null,要求输出 rustc JSON)指向它,r-a flycheck
   按 file+range 发布诊断 → **红波浪线直接出现在 .sv 文件里**。已核实机制可行(overrideCommand 是稳定配置,
   r-a 对诊断文件路径无"必须在 crate 内"限制,另有 `diagnostics.remapPrefix` 佐证路径重写是被支持的语义);
   **未找到任何现成工具做这件事**(搜索确认空白)——需要 spike 验证两点:多 span 诊断(primary 在生成文件、
   secondary 在用户 .rs)的混合重写;宏展开层 span(`span.expansion` 链)的处理。工程量估计:一周内出可用原型。
5. **`--remap-path-prefix` + r-a `diagnostics.remapPrefix`**:只能把 OUT_DIR 长路径显示成
   `<sv-gen>/Counter.rs` 之类的短标识,行号不动。美化用,优先级低。
6. **proc-macro 路线专属(对照组)**:`quote_spanned!` span 保真只在"表达式 token 来自宏输入"时可用——
   外部文件路线用不上;askama 式消息拼 file:line 是 (b) 的唯一选项,已在 §2.2 论述其上限。
7. **防呆**:生成文件头部 `#[rustfmt::skip]`? 不——我们主动格式化;真正要防的是用户**误编辑生成文件**
   (改了下次构建被覆盖):文件头大字注释 + 只读权限位(Windows 上 set readonly attribute)可选。

---

## 4. 给 svelte-rs 的推荐:主路线、备选与骨架

### 4.1 决定

- **主路线 = (a)**:新增 `sv-build` crate(库),内核复用现 sv-macro 的 parse/ir/codegen(三件套上提为
  独立 `sv-compiler` 库,sv-macro 变薄壳——正是 04 号报告 ADR-2 预留的演进);slint-build 三件套照抄:
  依赖清单 rerun-if-changed、`rustc-env` + `sv::include_components!()`、模板错误自报。
- **备选/配套**:
  - 路线 (c) 以 `cargo sv vendor`(+ CI `--check`)形态并存,服务组件库发布与"拒绝 build-dep"用户;
  - 路线 (b) 以 `sv!("File.sv")` 单文件糖形态可选提供(include_bytes hack,几十行);
  - 路线 (d) 的 CLI(`cargo sv dev`)延后到热重载里程碑,是编排层不是集成层;bindeps 不碰。
- **proc-macro `view!` 与 .sv 前端并存**,共享 sv-compiler。短期 `view!` 仍是默认入口(IDE 表达式体验好),
  .sv 前端作为独立编译器路线的探索载体;两者 codegen 同构,切换/共存零运行时成本。
- **错误策略**:§3 的 1+2+3 立即执行(就是 codegen 纪律);4(check 包装器)列为 M2 的 DX spike,
  成了就是相对 Slint 的差异化能力("模板嵌真 Rust 且诊断映射回模板")。

### 4.2 最小 build.rs 骨架

用户侧(app crate):

```toml
# Cargo.toml
[build-dependencies]
sv-build = { version = "0.1" }
```

```rust
// build.rs
fn main() {
    // 扫描 ui/**/*.sv,编译到 $OUT_DIR/sv-gen/,失败时带 .sv 路径行列报错并使构建失败
    sv_build::compile_dir("ui").unwrap_or_else(|e| {
        // e 的 Display 已含 miette 风格的 .sv 源码摘录与行列指示
        panic!("\n{e}");
    });
}
```

```rust
// src/main.rs
sv::include_components!();   // 展开为 include!(env!("SV_INCLUDE_GENERATED"))

fn main() {
    sv_shell::run(App::new()); // App 来自 ui/App.sv 生成的组件
}
```

`sv-build` 内部骨架(要点全部对应 §1a 的陷阱清单):

```rust
// sv-build/src/lib.rs(骨架,省略错误类型)
use std::{env, fs, path::{Path, PathBuf}};

pub fn compile_dir(dir: impl AsRef<Path>) -> Result<(), CompileError> {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join(dir.as_ref());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("sv-gen");
    fs::create_dir_all(&out).unwrap();

    // 1) 目录级兜底:新增/删除 .sv 文件也能触发重跑(逐文件清单做不到)
    println!("cargo::rerun-if-changed={}", root.display());

    // 2) 全量解析 → 组件图(import 解析、跨文件类型检查都在这里,宏路线做不到的全局视角)
    let graph = sv_compiler::load_project(&root)?;   // 模板域错误在此带 .sv span 返回

    let mut mods = String::new();
    for unit in graph.units() {
        // 3) 逐输入文件精确追踪(含 import 的 .sv 与引用的资源)
        for dep in unit.file_dependencies() {
            println!("cargo::rerun-if-changed={}", dep.display());
        }
        let (code, span_map) = sv_compiler::codegen_rust(unit)?; // code 已格式化、含 // sv: 锚点
        let rs = out.join(unit.module_name()).with_extension("rs");
        write_if_changed(&rs, &code);                 // 4) 内容不变不写,避免无谓的全 crate 重编
        write_if_changed(&rs.with_extension("rs.map"), &span_map.to_json()); // 5) 供 cargo sv check 重映射
        mods.push_str(&format!(
            "#[path = {:?}] pub mod {};\n", rs, unit.module_name()));
    }
    // 6) 索引文件 + rustc-env:用户端一句 include_components!() 完事(slint 同款)
    let index = out.join("mod.rs");
    write_if_changed(&index, &mods);
    println!("cargo::rustc-env=SV_INCLUDE_GENERATED={}", index.display());
    Ok(())
}

fn write_if_changed(path: &Path, content: &str) {
    if fs::read_to_string(path).map(|old| old == content).unwrap_or(false) { return; }
    fs::write(path, content).unwrap();
}
```

说明两处取舍:
- 索引文件用 `#[path]` mod 声明而非把所有组件拼进单文件——每个 .sv 对应独立 .rs,错误落点、
  goto-definition、span map 都按文件对齐,可读性好;`include!` 只进一次索引文件。
- `cargo::`(新语法,MSRV 1.77+)与 `cargo:` 旧语法按 MSRV 政策二选一;骨架用新语法。

### 4.3 风险与未决问题

1. **`cargo sv check` 重映射与 r-a flycheck 的集成细节**(多 span/宏展开链/增量诊断清除)无先例,须 spike;
   失败的兜底就是 §3.1–3 的"可读生成文件"底线,不致命。
2. **.sv 编辑的 IDE 时滞**(r-a 不 watch 数据文件):`cargo sv dev` watch 可缓解(改 .sv → 自动 touch 一次
   cargo check),终局在 sv LSP;需实测体感。
3. **组件库发布的双形态**(build.rs 版与 vendored 版)如何在一个 crate 里优雅共存:
   feature gate(`vendored` 特性跳过 build.rs 编译)还是发布时替换?倾向后者(发布物干净),待 vendor 命令落地时定。
4. **sv-build 的冷构建成本**要有预算线(目标:比 syn+quote 的 proc-macro 基线不差),从第一天在 CI 里测。
5. .sv 内 Rust 表达式的**编辑期智能**(补全/类型提示)是独立编译器路线相对 `view!` 宏的固有缺口,
   四种集成机制都救不了,只有 sv LSP(svelte2tsx 的转发变体)能补——这是"要不要把 .sv 前端转正"的
   最大悬置条件,与集成机制选型解耦。

---

## 5. 来源

- proc_macro tracked_path 跟踪 issue(open,2026-04-07 更新;API 现为 `proc_macro::tracked::*`):https://github.com/rust-lang/rust/issues/99515
- `Span::{file, local_file, line, column}` 稳定于 Rust 1.88.0:https://blog.rust-lang.org/2025/06/26/Rust-1.88.0/ · https://github.com/rust-lang/rust/pull/140514
- Cargo build scripts(rerun-if-changed 目录语义、默认保守重跑、mtime):https://doc.rust-lang.org/cargo/reference/build-scripts.html
- cargo artifact-dependencies 跟踪 issue(仍 -Z bindeps):https://github.com/rust-lang/cargo/issues/9096 · https://doc.rust-lang.org/cargo/reference/unstable.html · https://github.com/rust-lang/cargo/pull/15788
- rust-analyzer 配置(buildScripts.enable 默认 true、rebuildOnSave 语义、check.overrideCommand、diagnostics.remapPrefix):https://rust-analyzer.github.io/book/configuration.html
- r-a OUT_DIR include! 支持的源头 issue:https://github.com/rust-lang/rust-analyzer/issues/1964
- slint-build 源码(逐依赖 rerun-if-changed、SLINT_INCLUDE_GENERATED、diag 自报):https://github.com/slint-ui/slint/blob/master/api/rs/build/lib.rs · https://docs.rs/slint-build
- askama include_bytes 追踪 hack 源码与注释:https://github.com/askama-rs/askama/blob/master/askama_derive/src/generator.rs
- askama 模板机制:https://askama.rs/en/stable/configuration.html · https://docs.rs/askama
- sqlx offline(prepare / .sqlx / --check):https://github.com/launchbadge/sqlx/blob/main/sqlx-cli/README.md
- uniffi(UDL vs proc-macro 双前端、proc-macro 优先建议):https://mozilla.github.io/uniffi-rs/latest/ · https://mozilla.github.io/uniffi-rs/latest/proc_macro/index.html · https://lib.rs/crates/uniffi-bindgen-java
- tonic-build / prost-build(OUT_DIR、include_proto、out_dir 逃逸):https://docs.rs/tonic-build/latest/tonic_build/
- protox(纯 Rust protobuf 编译器,替代 protoc):https://github.com/andrewhickman/protox
- cxx-qt-build(QmlModule、qmlcachegen):https://docs.rs/cxx-qt-build/latest/cxx_qt_build/struct.CxxQtBuilder.html · https://github.com/KDAB/cxx-qt
- tauri generate_context 与资产陈旧问题:https://docs.rs/tauri/latest/tauri/macro.generate_context.html · https://takazudomodular.com/pj/zudo-tauri/docs/deployment/cargo-cache/
- vite-plugin-svelte(HMR、compileOptions):https://github.com/sveltejs/vite-plugin-svelte
- svelte2tsx / svelte-language-server:https://svelte.dev/blog/svelte-and-typescript
- rustc 无 #line 等价物(社区讨论):https://users.rust-lang.org/t/rust-equivalent-of-line-directive/44731
- --remap-path-prefix / --remap-path-scope(路径文本替换,不改行号;scope 仍 unstable):https://doc.rust-lang.org/beta/rustc/remap-source-paths.html
- rustc JSON 诊断格式(check 包装器的输入):https://doc.rust-lang.org/rustc/json.html

## 6. 仅凭训练数据、未逐一核实的点

- windows-rs 以预生成 bindings 发布(不跑 build.rs codegen)的细节与动机;
- r-a 对 OUT_DIR 生成文件磁盘变更的 VFS 刷新时机(是否 notify watch)——报告按保守假设("需 cargo check 触发")行文;
- uniffi `generate_scaffolding` 是否为 UDL 文件 emit rerun-if-changed(未抽源码);
- tauri dev 模式 devUrl 直连(不嵌资产)的行为细节(有多来源佐证,未读源码);
- `tonic::include_proto!` 展开形态(`include!(concat!(env!("OUT_DIR"), …))`)为训练期稳定事实;
- Dioxus `dx` / trunk 的编排细节仅作类比引用。

以上各点均不影响主结论(机制选型的决定性事实——tracked_path/bindeps 未稳定、askama hack、slint 三件套、
r-a 默认行为——全部已联网核实)。
