# sv:Svelte 风格的 Rust 跨平台桌面 UI 库 — 架构设计(v0 探索版)

> 状态:探索原型。本文综合 `docs/research/` 下 5 份联网核实的调研报告(2026-07-17)
> 与已跑通的原型代码,记录架构决策与路线图。crate 名前缀 `sv-` 为工作代号,可整体更名。

## 1. 愿景与定位

**一句话:把 Svelte 5 的编译哲学搬到 Rust 原生桌面——模板在编译期变成对 retained
场景树的定点更新代码,运行时没有虚拟 DOM、没有 diff、没有重建。**

目标平台:Windows / Linux / macOS / 鸿蒙(HarmonyOS NEXT / OpenHarmony)。

差异化立足点(调研 02 号报告结论,竞品空位已核实):

| 对比 | 它们 | 我们 |
|---|---|---|
| Slint | 自定义 DSL + 独立 LSP,表达式非 Rust;GPL/商业双许可 | 模板内嵌**真 Rust 表达式**;MIT/Apache |
| Dioxus | RSX + 模板 diff(仍有运行时比对) | 编译期定点更新,**零 diff** |
| Xilem | 每次重建 view 树再 diff | signal 图驱动,不重建 |
| Freya/Floem | 运行时细粒度,但无编译器;无鸿蒙 | 编译器 + **鸿蒙一等公民** |

"细粒度响应式 + 编译期模板 + 原生渲染"这个组合在 Rust 生态目前**无人在走**,是真实空位。

## 2. 架构分层

```
┌────────────────────────────────────────────────────────┐
│ 用户组件:view! 模板 + $state/$derived/$effect 风格 API │
├────────────────────────────────────────────────────────┤
│ sv-macro   view! 编译器(parse → IR → codegen)          │
│            编译产物 = 对 sv-ui 的命令式建树 + 绑定调用   │
├────────────────────────────────────────────────────────┤
│ sv-reactive  runes 内核:Signal/Derived/Effect          │
│              thread-local arena + Copy 句柄,push-pull   │
│              三态脏标记,effect 所有权树                  │
├────────────────────────────────────────────────────────┤
│ sv-ui      retained 场景树(桌面版 DOM)+ 绑定原语       │
│            bind_text / bind_style / if_block / each_block│
├────────────────────────────────────────────────────────┤
│ sv-shell   窗口壳 + 渲染器                               │
│   窗口:桌面=winit;鸿蒙=XComponent(窄窗口抽象 trait)   │
│   渲染:v0=CPU(softbuffer+tiny-skia+fontdue)           │
│         v1=vello 家族(vello / vello_hybrid / vello_cpu)│
│   文本:v0=fontdue → v1=Parley(fontique+HarfRust+swash)│
│   布局:v0=行列堆叠 → v1=taffy(flexbox/grid)           │
│   无障碍:AccessKit(Win/mac/Linux),鸿蒙需自研桥        │
└────────────────────────────────────────────────────────┘
```

## 3. 关键决策记录(ADR)

### ADR-1 响应式图:thread-local arena + Copy 句柄(已实现)
所有权问题的标准解法(Sycamore 0.9 / leptos reactive_graph / floem 已验证):节点放
thread-local slotmap,`Signal<T>`/`Derived<T>` 是 `Copy + !Send` 的世代句柄,随意塞闭包。
**不做** Send/Sync(reactive_graph 的 Arc 开销对单线程 UI 纯浪费);后台线程走消息回
UI 线程写 signal。调度采用 Svelte 5 同款 push-pull 三态脏标记
(`Clean/Check/Dirty`),菱形依赖天然 glitch-free,derived 惰性求值 + `PartialEq`
剪枝。effect 构成所有权树:重跑先销毁子树(`{#if}` 分支销毁即免费获得)。
与 Svelte 的刻意差异:effect **创建时同步首跑**(Svelte 是微任务延迟),桌面场景更直观;
后续接入帧调度(见 ADR-6)。放弃的 Svelte 特性:隐式赋值响应(`count += 1` 触发更新)
与 Proxy 深层响应——Rust 里用显式 `set/update` 与未来的 `#[derive(Store)]` 字段级信号替代。

### ADR-2(修订版,2026-07-17)编译策略:双前端共存,编译器核心独立
> 原版结论是"proc-macro 起步"。经过第二轮探索(调研 06–09 + 两条路线的可运行原型
> 并排实证,见 10 号报告),修订为:**编译器路线可行且长期天花板更高,采用
> "编译器核心独立库 + 双前端(view! 宏 / .sv 文件)共存"**,不做二选一。

**实证事实**(本仓库,两条路线均全绿):
- `crates/sv-macro`:view! 宏(parse/ir/codegen 分层,12 测试);
- `crates/sv-compiler`:.sv SFC 编译器(runes 源变换 + 原汁 Svelte 模板语法 +
  build.rs/OUT_DIR 集成,7 测试 + 端到端行为测试),`examples/counter-sfc` 可运行;
- 两者共享同一编译目标(sv-ui 绑定原语),生成代码形态几乎一致 → 共享内核成立。

**编译器路线独有收益**(proc-macro 做不到):
1. **runes 隐式反应性**:对整个 script 作用域源变换(裸 `count` → `.get()`,
   `count += 1` → `.update`,fmt 宏参数改写,闭包自动 move)——已实测生效;
   健全性靠"shadowing 编译期拒绝 + RHS 预求值 + 宏内 rune 硬错误"守住(调研 08)。
2. **模板语法 100% Svelte**:免引号文本、`{#if}{:else}`、`bind:`、撇号文本这些在
   proc-macro 里全是雷(调研 09)。
3. **热重载天花板**:编译成"数据面(模板结构表+样式表,免 rustc 热替换)+ 代码面
   (表达式闭包,经 rustc)",setup/render 拆分后状态天然保留(调研 09;明确**不做**
   Rust 表达式解释器——那等于再造半个 rustc)。

**编译器路线的结构性代价**(实测 + 调研 07):
1. rustc 类型错误落在生成文件而非 .sv(rustc 无 `#line`;实验对比见 10 号报告),
   proc-macro 则 span 精确到用户源码字符。缓解三层:模板域错误编译器自报(已做到,
   带 .sv 行列)→ 生成代码可读 + 锚点注释 → `sv check` 诊断重映射(sidecar span map
   重写 cargo check JSON,接 r-a `check.overrideCommand`,需 spike)。
2. `.sv` 内 Rust 表达式无 rust-analyzer:唯一解是 Volar/otter.nvim 式转发(生成文件
   落盘为一等产物 + .map 双向映射 + LSP 转发),MVP 约 9–16 人周,第一年只做只读特性。
   **这是 .sv 前端能否转正的最大悬置条件**;双前端策略把这笔税从赌注变成选项。

**构建集成定案**(调研 06,均已核实):build.rs + sv-build 库 + OUT_DIR + include!
(slint-build 三件套:逐文件 rerun-if-changed + rustc-env 包装宏 + 模板域错误自报);
路线 (b) proc-macro 读外部文件被否(`proc_macro::tracked::path` 至今 unstable,只有
include_bytes! hack);crates.io 组件库发布用预生成 vendor(sqlx 模式)。

**落地顺序**(无悔三步,同时服务双前端与热重载,调研 09 §5.3):
① sv-macro 与 sv-compiler 合并为单一编译器核心库(同一 IR/codegen,宏与 build.rs
都是薄壳);② Template 数据化("生成数据而非生成类型");③ codegen 拆 setup/render。
`.sv` 表面语法未决点:`$state`(Svelte 保真,token 级预处理,08 实验可行)vs 普通
函数形态(script 100% 合法 Rust,rustfmt/LSP 友好,09 主张)——留到核心合并时定,
当前原型两种都能支撑。
错误恢复解析仍是 IDE 生命线:parser 永不 panic、残缺输入仍出树、表达式逐字嵌入。

### ADR-3 渲染:CPU 栈起步,vello 家族为归宿(调研 05)
v0 用 softbuffer + tiny-skia + fontdue(零 GPU 依赖,分钟级构建,本原型已跑通中文渲染)。
v1 迁移 Linebender 标准栈(被 Blitz/Masonry/Bevy 共同压测):桌面 vello 0.9(wgpu),
鸿蒙 vello_hybrid(GLES 级 GPU,匹配 wgpu-OHOS 目前仅 GLES 后端的现实),
兜底 vello_cpu + softbuffer,三档共享同一 imaging model。渲染调用收敛到自有 Painter
trait,保留换后端余地。排除 skia-safe(C++ 构建重、拖累鸿蒙交叉编译)、GPUI(2026 已暂停
社区维护)。文本 v1 用 Parley 栈(fontique 字体发现/回退 + HarfRust 整形 + swash 光栅),
CJK/emoji/双向文本齐活;布局换 taffy;无障碍 AccessKit(鸿蒙无后端,需自研桥接 ArkUI
无障碍接口,列为风险)。

### ADR-4 窗口层:窄抽象 trait,不以 winit 为架构前提(调研 03)
winit 上游没有鸿蒙 backend(issue 仍 open,无回应)。抽一个六七个接口的窄窗口 trait
(建窗/尺寸/scale/重绘请求/输入事件/vsync):桌面端 winit 实现;鸿蒙端基于
openharmony-ability(社区,类 android-activity)起步,保留手写 XComponent glue 退路。

### ADR-5 鸿蒙:技术可行性无硬伤,列第二梯队立项(调研 03,已联网核实)
- 工具链零风险:`aarch64/x86_64-unknown-linux-ohos` 是 Tier 2 with host tools,rustup 直装。
- 渲染路径已被走通:ArkTS 薄壳(鸿蒙无纯 native 入口)→ XComponent → OHNativeWindow →
  EGL/GLES3;wgpu 2025-02 已合入 OHOS GLES 支持;Vulkan(VK_OHOS_surface + ash 0.39)是
  免费升级路径。桥接用 napi-ohos/ohrs,渲染热路径零 NAPI 调用。
- 先例:Flutter-OHOS(官方 SIG)、Servo-OHOS(Rust+EGL,与我们同构)。"自绘过不了审"是谣言;
  真实约束是性能基线(冷启动/帧率)与 IME/无障碍完成度。
- CI:Linux Command Line Tools + hvigorw 全自动(Servo 已验证);签名材料需 DevEco 生成一次。
- 风险最高的两个工程点:自绘 surface 上的 IME(中文组合文本/候选窗定位)与 OH_NativeVSync
  帧循环,应尽早真机验证。

### ADR-6 帧调度(未实现,最大开放设计点)
Svelte 的 microtask flush 要换成窗口系统帧管线:事件 → batch 写入 → 帧前 flush
(pre → render → layout → paint → user effects),配 `flush_sync` 逃生舱。桌面接
winit redraw 时机,鸿蒙接 OH_NativeVSync。目前原型是写入即同步 flush,正确但未对齐帧。

### ADR-7 each 块:保留 Svelte 的 keyed reconcile 设计(未实现)
现状是整块重建(`sv_ui::each_block`)。目标形态:每项持有 `Signal<Item>`,内容变化走
原地 set,reconcile 只处理 key 的增删移;seen-set 启发式 vs LIS 待场景树搬移成本基准后定。

### ADR-8(2026-07-17)CSS 无缝支持策略:真语法封闭子集 + 编译期样式表,永不引入运行时选择器引擎
> 问题:Svelte 开发者在浏览器写真 CSS(级联/继承/选择器/伪类/单位),桌面端如何
> 最小化心智迁移?依据调研 11(业界五档光谱 + Rust 基建)与 12(语义逐项映射)。

**核心判断**:
1. 业界口碑分界线在**"真 CSS 语法 + 选择器 + 状态伪类 + 变量 + 动画"**(Lynx 档),
   不在完备性;RN 的"属性名对象"档留下大量迁移吐槽,Flutter 的零 CSS 是心智断崖。
   Svelte 的 scoped-by-default 又把实际需求压缩到:扁平类规则、状态伪类、盒模型、
   变量、@media、继承子集、transition、简写展开——**八条**。
2. **继承是最重要的隐形单项**(color/font-* 沿树继承是 CSS 直觉的地基,开发者
   意识不到自己在依赖它):实现为 layout 前一次 O(n) 自顶向下 resolve 遍历
   (InheritedContext 顺路下传,产出 ComputedStyle),**不进响应式图**,失效靠既有
   doc 版本号——不造浏览器式失效引擎。
3. **不做 specificity 计数与 !important**:组件内规则用"声明序 + 通道优先级"
   (类 < 内联 style < 条件类 < 伪类 < style: 指令)——与 CSS 同 specificity 时的
   声明序规则一致;Svelte 自己也用 `:where` 压平 specificity,佐证该心智可接受。
4. **选择器匹配全部编译期做掉**(模板树编译期已知):后代/结构伪类编译成布尔条件
   patch,保持"类=编译期样式表索引、零运行时选择器"的架构差异化;stylo 全引擎
   (Blitz 路线)与此相斥,仅列 C3 可选项(触发条件:未来要渲染任意 HTML)。
5. 解析器:两份调研分歧点——11 号推荐换 lightningcss(规范级解析,MPL-2.0),
   12 号推荐**继续自写**(封闭属性集错误定位统一、零许可证复杂度)。裁决:C1 期
   自写,lightningcss 作差分测试基准,C2 复评;MPL 依赖若引入需法务口径确认。

**分阶段路线**(C1 3–5 人周;C2 +6–10 人周踩 M1 taffy;C3 可选):
- **C1 语法真化**:标准属性名 + 简写展开(padding 四值)+ 单位(px/em/rem/%)+
  全颜色格式 + 状态伪类(:hover/:active/:focus/:disabled)+ **默认继承子集**
  (color/font-*/line-height)+ :root{--x} 编译期常量;超子集报错带 .sv 行列与
  did-you-mean。**已落地的首期原型**:标准属性名别名(background-color/color/
  border-radius/flex-direction)、px 单位(其它单位引导报错)、rgb()/rgba()/颜色名、
  `.类:hover`(编译期自动生成悬停状态 + 指针事件接线 + 与用户回调合成),
  见 `css_compat_names_units_hover` 测试与 showcase。
- **C2 行为完备(= 迁移无感线)**:margin/border/box-sizing(缺省 border-box)、
  flex/grid 属性面 taffy 直通、CSS 自定义属性 + var() 继承做主题(撤销 09 的
  @theme 自造语法)、transition 属性(与 transition:fade 指令双轨正交:属性变化 vs
  进出场)、@media(窗口尺寸 + prefers-color-scheme)、组件内后代组合子(编译期
  静态匹配)。
- **永不支持清单**(文档化 + 替代写法):伪元素、:global、:nth-child 结构伪类
  (P2 复评)、!important、inherit 关键字、@layer/@supports——业界共识裁剪与
  Svelte 低频使用面双重印证。

## 4. 原型现状(本仓库,全部测试绿)

| crate | 内容 | 测试 |
|---|---|---|
| sv-reactive | runes 内核完整实现(state/derived/effect/batch/untrack/on_cleanup/create_root,三态脏标记、所有权树、循环保护、derived 写保护) | 17 |
| sv-ui | 场景树 + bind_text/bind_style/if_block/each_block;版本号 + on_mutate 驱动重绘 | 6 |
| sv-macro | view! 宏前端(parse/IR/codegen 分层) | 12 |
| sv-compiler | .sv SFC 编译器前端(sfc/script runes 变换/template/style/codegen,prettyplease 可读产物,错误带 .sv 行列) | 7 |
| sv-shell | winit 窗口 + CPU 渲染(HiDPI、圆角、CJK、命中测试、离屏 PNG) | 2 |
| examples/counter | 计数器 · view! 宏路线(开窗交互 + `--png` 离屏验证) | — |
| examples/counter-sfc | 计数器 · .sv 编译器路线(build.rs 集成 + 端到端行为测试) | 1 |

## 5. 路线图

- **M0(已完成)**:响应式内核 + 场景树 + CPU 渲染壳 + 计数器闭环;view! 宏 v0;
  .sv 编译器 v0(runes 源变换 + Svelte 模板语法 + build.rs 集成)+ 双路线实证对比;
  Svelte 特性面扩展(组件+$props v0、{#each}{:else}、{#key}、{@const}、style: 指令、
  $derived.by/$state.raw/$inspect/$sig、onclick;支持矩阵见 [SVELTE-SUPPORT.md](SVELTE-SUPPORT.md))。
- **M1 编译器与运行时补全**:双前端合并为单一编译器核心(同一 IR/codegen)+
  Template 数据化 + setup/render 拆分(ADR-2 无悔三步);keyed each;组件模型
  (props/$bindable/snippet);taffy 布局;文本换行;键盘焦点链;滚动;帧调度(ADR-6);
  `sv check` 诊断重映射 spike;TodoMVC 级 demo。
- **M2 渲染升级**:Painter trait;vello(wgpu)桌面后端;Parley 文本(IME 组合文本);
  AccessKit 接入;深色模式;多窗口。
- **M3 鸿蒙探底 spike**:XComponent + wgpu(GLES)三角形 → 场景树渲染 → 触摸事件 →
  真机 IME/VSync 验证;窄窗口 trait 落地;hvigorw CI。
- **M4 工程化**:热重载(模板数据化后接 Dioxus subsecond 路线);`#[derive(Store)]`;
  错误恢复解析 + IDE 体验打磨;性能基准(更新延迟/内存/增量编译)。

## 6. 风险清单(按杀伤力排序)

1. **`.sv` 的 IDE 体验**是编译器路线转正的最大悬置(Volar 式转发 LSP 未 spike;
   第一年靠"生成代码可读 + sv check 诊断重映射 + 只读 LSP 特性"止血,调研 07)。
2. **鸿蒙 IME/无障碍**完成度(自绘 surface 上无免费午餐;AccessKit 无 OHOS 后端)。
3. **vello_hybrid 成熟度**(sparse strips 仍 beta)——有 vello classic 与 vello_cpu 双兜底。
4. **编译时间**——坚持"生成数据而非类型";增量编译基准纳入 CI。
5. 单人/小团队维护面过宽——渲染/文本/布局/无障碍全部复用 Linebender,自研面收敛到
   编译器 + 响应式 + 组件运行时三样。

## 7. 调研报告索引

- [01 Svelte 5 编译模型与 Rust 映射](research/01-svelte-model.md)
- [02 Rust GUI 生态全景与差异化](research/02-rust-gui-landscape.md)
- [03 鸿蒙 Rust 自绘可行性](research/03-harmonyos.md)
- [04 编译器策略(proc-macro vs 外部文件 vs 自定义语言)](research/04-compiler-strategy.md)
- [05 四平台渲染/文本/布局/无障碍选型](research/05-rendering-stack.md)

第二轮(编译器路线专题,2026-07-17):
- [06 .sv 构建集成机制(build.rs/OUT_DIR 定案)](research/06-sv-build-integration.md)
- [07 .sv 的 IDE/LSP 策略(Volar 式转发可行性)](research/07-sv-ide-lsp.md)
- [08 runes 源变换语义与健全性(变换规则 + 拒绝清单)](research/08-sv-runes-transform.md)
- [09 .sv 格式设计 + 热重载架构(数据面/代码面)](research/09-sv-sfc-format-hotreload.md)
- [10 双路线动手实证对比(本仓库原型)](research/10-route-comparison-hands-on.md)

第三轮(CSS 无缝支持专题,2026-07-17):
- [11 业界桌面/跨端框架 CSS 策略光谱 + Rust 基建](research/11-css-industry-strategies.md)
- [12 CSS 语义逐项映射设计(继承/伪类/盒模型/@media/心智兼容表)](research/12-css-semantics-mapping.md)
