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
│   渲染:v0=CPU(softbuffer+tiny-skia+swash)             │
│         v1=vello 家族(vello / vello_hybrid / vello_cpu)│
│   文本:v0=swash 直用 → v1=Parley(fontique+HarfRust)   │
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

### ADR-3b(2026-07-18)渲染后端对比判决 + 可切换 Painter 抽象已落地
> 依据调研 13(七类后端逐一对比)与 14(可切换性先例与设计),回答"后端优劣 +
> 可切换可行性"。

**后端排序结论**(13 号,联网核实):vello classic 是唯一对 CSS ⏳ 能力清单
(渐变/模糊/阴影/transform/裁剪/图片)全绿的候选,但 OHOS-GLES 不可用 → 桌面
主力;OHOS 首选 vello_hybrid(仍 early,spike 当周复核,可降级 vello_cpu);
兜底从 tiny-skia 迁 vello_cpu(官方定性 broadly usable),三档共享 imaging model。
skia-safe(构建/OHOS)、femtovg(能力天花板)、OS 原生(Linux 缺口)、自研 wgpu
(GPUI 已停维护)全部出局。**风险入册**:fontdue 自 2025-02 停更,Parley 迁移
优先级上调;tiny-skia 栈能力冻结(CSS ⏳ 项一律不在 CPU 栈实现)。

**可切换判决:高度可行,业界标准做法**(14 号:Slint 四渲染器、iced 双轨自动
回退、Flutter DisplayList 四年灰度迁移、Linebender anyrender 已带四后端)。
**已落地**(`sv-shell/paint.rs` + `render.rs` 拆分,112 测试零回归):
- `Painter` trait 即时调用接口(fill/stroke/glyph_run,词汇对齐 vello Scene);
- 共享 `paint_tree` 遍历器 —— 后端只实现三个动词;
- `TinySkiaPainter` 为首个后端(纯搬运,`render_frame` 签名不变);
- `RecordingPainter` 显示列表实现 —— 命令流金样测试(零像素零 GPU),
  未来新后端先对拍命令流,亦是帧间缓存的载体;
- 文本裁决:painter 拿**定位好的 glyph run**(shaping 在上、光栅在下),
  M2 换 Parley 只动 shaping 门面。
抽象税:dyn 只收在 sv-shell 边界内,每帧低千级调用 ≈ 个位数 µs;与 tachys
泛型污染用户视图树本质不同。

**多后端已落地(2026-07-18)**:vello 0.9(wgpu 29)成为第二个真实后端
(`backend-vello` feature,`vello_backend.rs`):VelloPainter 复用共享 paint_tree,
glyph 走 GlyphPos 的 id/基线原点直上 draw_glyphs(fontdue 与 peniko::FontData
零拷贝共用字体字节);呈现走 render_to_texture + blit(0.9 无 render_to_surface);
离屏回读 parity 测试 **GPU/CPU 非白像素比 1.001**,无 adapter 自动 skip。
切换机制已实现:cargo feature 编入 × `SV_RENDERER=cpu|vello` 覆盖 ×
自动探测失败静默回退 CPU;双后端开窗冒烟均通过。caps:vello 报 `blur: true`
(消费方待 box-shadow 落地)。

**文本栈已迁 swash(2026-07-18,调研 18)**:fontdue 急切解析 CJK 轮廓
(~173MB)且 2025-02 起停更,整体替换为 swash 0.2.10(skrifa 后端零拷贝懒解析):
基线内存 198→27MB、首帧 573→11ms、CPU 光栅 30k 档快约一倍,GPU/CPU parity
1.017。shaping 仍是线性排版(无 kerning/连字),M2 Parley 迁移点不变,只动
shaping 门面。字形缓存为两代(hot/cold)淘汰——超限整代降冷而非清空,
消除"缓存清空帧"的 1% low 长尾。离屏 vello 自建 device 按 adapter 能力抬高
`max_storage_buffer_binding_size`(修复 100k 档 scene buffer 192MB > 128MB
默认上限的崩溃;窗口路径仍受 RenderContext 默认值,上游工程项)。

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
- **C1 语法真化 ✅ 已落地(2026-07-18)**:标准属性名、padding/margin 四值简写与
  长手(Edges 盒模型)、border 实线、rem、hsl()/hwb()/现代颜色语法/#hex-alpha/
  命名色 ~60、**继承管线**(color/font-size 哨兵 + 渲染期父链解析、currentColor)、
  :root{--x}+var()、CSS 嵌套 &:pseudo、:hover/:active 双状态接线、元素类型规则、
  cursor。em/%/:focus/:disabled 留 C2(动态基准/taffy/焦点链)。
  证据:`css_c1_box_model_vars_nesting` 等测试 + showcase;逐项见
  [CSS-SUPPORT.md](CSS-SUPPORT.md) "C1 已落地"节。
- **C2 行为完备(= 迁移无感线)**:margin/border/box-sizing(缺省 border-box)、
  flex/grid 属性面 taffy 直通、CSS 自定义属性 + var() 继承做主题(撤销 09 的
  @theme 自造语法)、transition 属性(与 transition:fade 指令双轨正交:属性变化 vs
  进出场)、@media(窗口尺寸 + prefers-color-scheme)、组件内后代组合子(编译期
  静态匹配)。
- **永不支持清单**(文档化 + 替代写法):伪元素、:global、:nth-child 结构伪类
  (P2 复评)、!important、inherit 关键字、@layer/@supports——业界共识裁剪与
  Svelte 低频使用面双重印证。
- **现代 CSS 全面差距表**(91 项逐条,含 2023–2026 新浪潮的逐项裁决)见
  [CSS-SUPPORT.md](CSS-SUPPORT.md)。

### ADR-9(2026-07-18)规模策略:视口虚拟化,帧成本与逻辑控件数解耦
> 目标"百万控件 1% low 稳定 144fps"(帧预算 6.94ms)。全量建树在 10k+ 档
> 已是几十 ms(调研 17/18),常数优化救不了数量级——答案是架构性削减每帧工作集。

`sv_ui::virtual_list` 原语:视口 N 行固定槽位(每槽 `Signal<Option<T>>`),
滚动 = 逐槽 `.set()` 走 bind_text 定点更新,**零节点创建/销毁、零结构变化**
(1% low 稳定性的来源);`item_at` 懒取数,逻辑条目永不物化。
**实测验收(调研 18)**:100 万控件(20 万行×5)连续滚动最坏工况,CPU 后端
p99=5.28ms、1% low=174fps、WS 28MB;窗口口径 ~800fps(softbuffer 无 vsync)。
配套:字形缓存分代淘汰(见 ADR-3b 附注)。
后续阶梯:帧调度 ADR-6(mailbox 让 vello 窗口口径突破 vsync 60)→ 增量场景
编码(RecordingPainter diff,惠及全量档)→ 局部布局(dirty 子树)→ 滚动物理。

## 4. 原型现状(本仓库,全部测试绿)

| crate | 内容 | 测试 |
|---|---|---|
| sv-reactive | runes 内核完整实现(state/derived/effect/batch/untrack/on_cleanup/create_root,三态脏标记、所有权树、循环保护、derived 写保护) | 17 |
| sv-ui | 场景树 + bind_text/bind_style/if_block/each_block;版本号 + on_mutate 驱动重绘;**焦点链 + 键盘路由 + 快捷键注册表(R1,调研 20)**;**TextInput 编辑内核 + IME + 剪贴板 provider(R1,调研 21)** | 6+17 |
| sv-macro | view! 宏前端(parse/IR/codegen 分层) | 12 |
| sv-compiler | .sv SFC 编译器前端(sfc/script runes 变换/template/style/codegen,prettyplease 可读产物,错误带 .sv 行列) | 7 |
| sv-shell | winit 窗口 + CPU 渲染(HiDPI、圆角、CJK、命中测试、离屏 PNG);**键盘接入 + 点击设焦 + 默认焦点环 + IME 全流程 + push_clip/pop_clip 双后端 + arboard 剪贴板(R1)**;**滚动体系(R2)**;**taffy 0.12 布局引擎 + UAX #14 折行 + flex 第一批(R2)** | 22 |
| examples/counter | 计数器 · view! 宏路线(开窗交互 + `--png` 离屏验证) | — |
| examples/counter-sfc | 计数器 · .sv 编译器路线(build.rs 集成 + 端到端行为测试) | 1 |

## 5. 路线图

> 2026-07-18 商用化修订:依据调研 19(分档判决)与 20–25(六项落地方案),
> 原 M1–M4 重排为商用导向的 R1–R5;每阶段带验收标准与人周估算(出自对应调研,
> 粗粒度)。原分期映射:M1 ≈ R1+R2+内核合并;M2 ≈ R3;M4 发布部分并入 R4
> (热重载/Store/IDE 仍为 M4 独立项);M3 = R5。

- **R0(已完成,原 M0+)**:响应式内核 + 场景树 + CPU/vello 双后端 + 双前端闭环;
  Svelte 语法面 43/77;CSS C1;虚拟化 1M@144fps(ADR-9);swash 文本栈;
  双语文档中心 + 3-OS CI;详见各 ADR 与支持矩阵。
- **R1 输入地基**(调研 20/21):键盘通道+焦点链+快捷键(4.5–6 人周;裁决:焦点
  不做 signal,`:focus` 走 `__fc` 复用 `:hover` 接线;Tab 用树序不做数值
  tabindex;.sv 走 Svelte 5 `onkeydown` 属性形态)
  ——**✅ 2026-07-18 档 A 切片落地**:Key/KeyEvent/Mods 自有类型、focusable 位
  (Button/Checkbox 默认开)、focus/blur/next/prev + remove 清焦点、四段
  dispatch_key(冒泡→Tab 导航→Enter/Space 激活→快捷键)、快捷键注册表
  (on_cleanup 自动注销、后进先出)、winit 接入(map_key、synthetic 过滤、
  点击设焦)、默认焦点环、双前端 onkeydown/onfocus/onblur/autofocus;
  `:focus` 伪类接线与 keyup/捕获段留档 B(调研 20 §6);TextInput+剪贴板+IME
  (6.5–10.5 人周;裁决:编辑态 `Box<InputState>` 在节点内不进 Signal、EditOp
  纯模型词汇对齐 Parley PlainEditor、剪贴板 arboard、`bind:value` 复刻
  bind:checked 模板;Painter 新增 push_clip/pop_clip——与 R2 滚动共享)
  ——**✅ 2026-07-18 档 A 切片落地**(调研 21 步 1–5):`<input>` 元素 +
  InputState/EditOp 编辑内核(UTF-8 边界、选区、光标折叠)、IME 全流程
  (handle_ime 纯函数 + set_ime_allowed 焦点同步 + cursor_area 候选窗跟随、
  预编辑 over-the-spot 下划线)、arboard 剪贴板(provider 注入,测试假实现)、
  push_clip/pop_clip 双后端(tiny-skia Mask / vello push_clip_layer)、
  bind:value/oninput/onsubmit/placeholder 双前端;**验收通过**:TodoMVC 键入
  新条目 + Tab 遍历 + Enter 提交(todo-sfc 测试)、IME 事件序列自动化、
  input-demo 手测台 + 三平台清单入库(真机手测待勾选);拖拽选择/双击选词/
  undo/多行留档 B(调研 21 步 6 + M2 Parley)。
- **R2 视口与布局**(调研 22/23):滚动体系(5.5–8.5 人周;裁决:`Style.overflow`
  非新 ElementKind、offset 真源在节点内 + Signal 可选桥、tiny-skia 手动矩形裁剪
  /vello push_layer、滚动条 shell 合成绘制不入树、指针捕获通道一并补齐)
  ——**✅ 2026-07-18 档 A 切片落地**(调研 22 S1/S2/S2'/S3/S5):Painter 裁剪
  双后端(CPU 矩形交集弃 Mask、vello push_layer 绕 issue #1198)、
  `Style.overflow` + 节点内 scroll 真源 + content_override、place 平移/clip
  传播/Placed.clip + ScrollArea 旁路、滚轮 + 最近可滚祖先滚动链(route_wheel
  纯函数离屏可测)、滚动条合成绘制(只绘不拖,thumb 几何纯函数)、
  `onscroll`/`bind:scrolly`(链式保留)双前端、virtual_scroll 桥(100k 行
  虚拟高度接真实滚轮);**留档 B**:S4 thumb 拖拽 + 指针捕获、S6 平滑/惯性、
  触摸滚动、overflow-x/y 按轴拆分;
  ——**✅ 2026-07-18 taffy + 换行落地**(调研 23 T1/T2/T3):taffy 0.12
  变更帧重建封在 layout_tree 内(`Vec<Placed>` 契约不动,全部金样/回路/
  滚动测试零回归;disable_rounding 保 HiDPI;缺省 align_items=Start、
  flex_shrink=0 保迁移零回归,单测钉死);swash + unicode-linebreak 折行
  (UAX #14 CJK 断点/标点禁则/超长强断,计划内报废,M2 换 Parley 门面)
  + text-align;flex 第一批(justify/align/grow/shrink/wrap/min-max)
  样式键落地;**验收通过**:settings-sfc 设置面板超一屏可滚 + flex 对齐 +
  长文本折行(测试 + 离屏人查);ADR-9 复验:1M 虚拟化 p99=5.56ms/
  1% low 178fps 达标;**30k 全量档 2ms 触发线已越**(实测 ~130–160ms:
  taffy 裸 ~45ms + 叶子 measure ~70ms)→ 按预案将"低层 trait 增量布局"
  列入档 B(2–3 人周);
  → **档 A 达成(内部工具可用)**:R1 输入地基 + R2 视口与布局全部落地;
  档 A 收尾清单 = 真机 IME 三平台手测勾选(input-demo README)。
  taffy 0.12 + 换行(~9 人周;裁决:变更帧重建 TaffyTree + measure fn 封在
  layout_tree 内,`Vec<Placed>` 契约不动;换行不等 Parley,swash + 
  unicode-linebreak 过渡——Slint 同款,计划内报废)。验收:设置面板 demo
  超一屏可滚 + flex 对齐 + 长文本折行;ADR-9 预算复验(虚拟化恒 ~34 节点不受
  taffy 拖累;全量档设 2ms 触发线决定是否升级增量布局)。
  → **档 A 达成(内部工具可用)。R1+R2 档 A 切片合计 ≈18–23 人周(约 1.5–2 个季度全职)**
- **R3 文本栈/无障碍/弹层**(调研 24/25):Parley 0.11 迁移(10–15 人周;前置
  "载体扩宽"——GlyphKey/glyph_run 签名/glyph_cache/VelloPainter 四处单字体假设,
  修正 ADR-3b"只动 shaping 门面"的乐观表述
  ——**✅ 2026-07-18 P0 载体扩宽落地**:`FontHandle{key}` 身份句柄、
  `GlyphKey` 加 font_key、`Painter::glyph_run(FontHandle, ..)`、光栅缓存
  按字体分桶、vello 端按 key 缓存 FontData;行为不变(仍单字体),
  全量测试零回归——fontique/Parley 接管后同帧多字体载体无需再动;
  **✅ 2026-07-18 P1 Parley 接管落地**:TextEngine 门面(text.rs,全仓唯一
  parley import,锁 0.11)——fontique 系统字体发现 + HarfRust shaping +
  script fallback(CJK/Latin 混排双字体 run,.notdef 消除)+ zh-Hans locale
  + overflow-wrap: anywhere;排版恒逻辑 px(画/量同源保 HiDPI);字体注册
  按 Blob id 建键(**保留键 0 归内置字体,注册键高位恒 1**——撞键 = Latin
  全员错字,实测踩过并有回归卫兵);measure 两代淘汰缓存;旧
  swash+unicode-linebreak 折行门面按计划报废退役;TextInput 仍走旧线性
  路径(编辑几何与显示同源,P3 随 PlainEditor 切换);已知噪音:上游
  icu_segmenter 缺 cjdict 数据的 `ICU4X data error` stderr 告警(断行
  功能正常回退,上游议题);
  PlainEditor 外包 TextInput 编辑内核);
  AccessKit(egui PR #2294 形态:懒激活 + Doc 版本号节拍推送,TreeUpdate.focus
  与 R1 焦点链强耦合,树映射纯函数金样测试)
  ——**✅ 2026-07-18 P4/P5 落地**:a11y.rs 纯函数 `build_tree_update`
  (NodeId=ViewId 世代键 ffi、role 映射五元素、bounds=Placed.rect 命中同源、
  focus 必填走焦点链、TextInput value/占位符名称、`aria-label` 双前端属性
  含响应式形态)+ accesskit_winit 0.33 适配器(窗口先隐身建 adapter、
  每事件 process_event、懒激活 InitialTreeRequested、版本节拍
  update_if_active 全量推送、Click/Focus/Blur 动作回派纯函数);
  金样与动作往返测试零窗口零平台;**待办**:NVDA/VoiceOver/Orca 真机朗读
  冒烟(bounds 坐标空间平台实测校准,调研 24 风险 5)、增量 TreeUpdate
  与列表/滚动语义(档 B 打磨);弹层体系(8–13 人周;裁决:离散层
  Base→Popup→Tooltip + `overlay_block` 原语 + `<overlay>` 内建元素,不做通用
  z-index、不发明 {#teleport})
  ——**✅ 2026-07-18 O1/O2/O3/O5 落地**:游离弹层子树 + 注册表(注册序即
  层内叠序)、`overlay_block`(on_dismiss 只回写 signal 单一数据源)、
  渲染壳布局尾段锚定(四侧 + 越界翻转 + clamp)+ `OverlayRegion` 区间表
  (Placed 追加末尾,树序绘制/`rev()` 命中零改动)、关闭策略三值 + Esc
  LIFO(进 dispatch_key)、**modal 区间阻断 + 焦点陷阱**(Tab 环限定、
  关闭恢复原焦点)、tooltip(悬停代数计数 + tasks 延时,Tooltip 层不可
  命中);`z-index` 进 ADR-8 永不清单;
  **✅ 同日 O4/O6 落地**:Popup 内 ArrowDown/Up = 焦点导航(菜单免费
  获得方向键;dispatch_key 导航段)、`<overlay>` .sv 内建元素
  (open 必填/anchor 五值锚**父容器**/gap/modal/close/ondismiss/style,
  children 编译成 overlay_block build 闭包)、Esc 语义纠偏
  (CloseBehavior 只管指针手势,Esc 看 on_dismiss——模态对话框
  "点外不关、Esc 可关"惯例)、examples/overlay-demo 组件自举
  (Dialog.sv $bindable open 居中模态 + 下拉菜单 + @attach 挂 tooltip,
  行为测试 + 离屏人查);**弹层体系全量完成**,余子菜单侧向锚定
  (嵌套 overlay 已可表达,组件层能力)。
- **R4 发布工程 + API 冻结**(调研 25,4–6.5 人周):改名 ADR-10 **先于 crates.io
  首发**(实查:`sv` 已被 Bitcoin SV 库占用,sv-*/svello 等空闲);六 crate 依赖序
  首发 + 0.x semver 政策 + CHANGELOG + semver-checks/audit/clippy 门禁;去 panic
  审计(sv-shell 非测试 expect 8 处);打包 cargo-packager 主力 + cargo-dist 参照;
  API 冻结前置 = 双前端合并(原 M1)+ 帧调度 ADR-6(语义级 breaking 一次出清)。
  → **档 B 达成(单桌面平台可商用;校准业界先例 2–3 年全职,含打磨周期)**
- **R5 鸿蒙(档 C,原 M3 不变)**:XComponent + wgpu(GLES)三角形 → 场景树渲染 →
  触摸事件 → 真机 IME/VSync 验证;窄窗口 trait 落地;hvigorw CI;
  另加 accesskit-ohos 桥(调研 05 估 2–4 人周)。
- **M4 遗留独立项**(不阻塞商用分档):热重载(模板数据化后接 subsecond 路线);
  `#[derive(Store)]`;错误恢复解析 + LSP(及格线 9–16 人周,调研 07);
  性能基准 CI 化。

## 6. 风险清单(按杀伤力排序)

1. **`.sv` 的 IDE 体验**是编译器路线转正的最大悬置(Volar 式转发 LSP 未 spike;
   第一年靠"生成代码可读 + sv check 诊断重映射 + 只读 LSP 特性"止血,调研 07)。
2. **鸿蒙 IME/无障碍**完成度(自绘 surface 上无免费午餐;AccessKit 无 OHOS 后端)。
3. ~~fontdue 急切解析 CJK 字体 ≈188MB + 停更风险~~ **已消除**(2026-07-18
   swash 迁移落地,基线 27MB,调研 18);遗留:线性排版无 kerning/换行,M2 Parley。
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

第四轮(渲染后端专题,2026-07-18):
- [13 七类渲染后端逐一优劣对比(对准本项目工作负载)](research/13-render-backends.md)
- [14 可切换 Painter 抽象:先例、设计与迁移八步](research/14-switchable-painter.md)
- [15 三类场景现状分析(轻量内存/复杂界面/复杂界面+3D,含实测基线)](research/15-scenario-analysis.md)
- [16 分场景内存基准测试与分析(membench 测试台,0.5KB/控件,字体占 97.7%)](research/16-memory-benchmarks.md)
- [17 分后端×分场景内存构成与帧率(CPU vs vello;三个阴性实验;device 固定成本实测)](research/17-backend-memory-fps.md)
- [18 百万控件@144fps:swash 迁移(198→27MB)+ 视口虚拟化(1M p99=5.28ms/1% low 174fps)](research/18-million-controls-144fps.md)
- [19 距离可商用还有多远:四路审计、业界九项交集、分档判决(档A 1–2 季度/档B 2–3 年)](research/19-commercialization-gap.md)

第六轮(商用路线落地方案,2026-07-18,支撑路线图 R1–R4):
- [20 键盘事件通道+焦点链+快捷键(4.5–6 人周;树序 Tab、__fc 复用 :hover 接线)](research/20-keyboard-focus.md)
- [21 文本输入+IME+剪贴板(6.5–10.5 人周;编辑态不进 Signal、arboard、bind:value)](research/21-text-input-ime-clipboard.md)
- [22 滚动体系(5.5–8.5 人周;Style.overflow、push_clip/pop_clip、virtual_list 桥)](research/22-scroll-system.md)
- [23 taffy 接入+文本换行(~9 人周;变更帧重建+measure fn、unicode-linebreak 过渡)](research/23-taffy-text-wrap.md)
- [24 Parley 迁移+AccessKit(10–15 人周;先做载体扩宽,修正 ADR-3b 乐观接缝)](research/24-parley-accesskit.md)
- [25 弹层体系+发布工程(8–13 + 4–6.5 人周;离散层非 z-index;sv 名已被占需改名 ADR-10)](research/25-overlay-release-engineering.md)

第七轮(生态探索,2026-07-18):
- [26 arco.design 视觉标准 UI 组件库(sv-arco)可行性:条件可行 B 档;token 层即刻可开工、组件四波跟 R1–R3 能力线,A0–A5 ≈17–26 人周约 30 组件;最大风险图标管线(需路线图外新增 fill_path + SVG 转译)](research/26-arco-design-ui-kit.md)
