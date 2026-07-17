# Rust GUI 生态全景(2026 年中):我们能复用什么、该与谁差异化

> 调研日期:2026-07-17。版本号、发布日期、许可证均已通过 crates.io API / GitHub / 官方博客联网核实(核实日期即调研日期)。个别标注"仅训练数据"的结论未能联网确认。
> 项目背景:探索 Svelte 风格(编译期模板 → 细粒度命令式更新、runes 心智模型、极小运行时)的 Rust 跨平台桌面 UI 库,目标平台 Windows / Linux / macOS / HarmonyOS NEXT(OpenHarmony)。

---

## 0. 核心判断(TL;DR)

1. **"细粒度响应式 + 原生渲染"这个组合今天没有成熟品**,最接近的是 Floem(Lapce 团队,signals + wgpu/vello,pre-1.0)和刚完成重写的 Freya 0.4(自研响应式 + Skia,但内部仍做 UI tree diffing)。**这个空位是真实存在的,而且"编译期生成更新代码"这条 Svelte 路线没有任何人在走**——所有现存方案的响应式都是纯运行时机制。
2. **可复用的基础设施已经足够好,不需要自研渲染/文本/布局/无障碍**:winit + wgpu + Vello(或 vello_hybrid/vello_cpu)+ Parley + harfrust/swash + taffy + AccessKit 构成了 2026 年事实上的"Linebender 标准栈",Blitz、Masonry、Bevy 都在用。我们的自研应该收敛到**响应式编译器 + 组件运行时 + widget 层**这三样。
3. **HarmonyOS 是全生态的空白,也是我们最大的风险点和最锐利的差异化点**。winit 上游没有 OpenHarmony 支持(issue 无维护者回应);可行路径是 Servo/ohos-rs 社区已验证的 XComponent + EGL/Vulkan + wgpu(GL backend)方案,但窗口层要自己写,AccessKit 没有鸿蒙 adapter,IME 要桥 ArkUI。这块工作量必须在原型期就探底,否则"四平台"承诺立不住。
4. 差异化定位一句话:**"Svelte for native Rust"——模板在编译期直接编译成对 retained widget tree 的定点变更指令,运行时没有 VDOM、没有 view diffing、没有组件重执行;加上鸿蒙一等公民支持和宽松许可证**。这三点分别打中 Dioxus(VDOM diff)、Xilem(view rebuild+diff)、Slint(独立 DSL + 许可证摩擦)的软肋。

---

## 1. 框架逐个评估

### 1.1 总览表(数据截至 2026-07-17,均已联网核实)

| 框架 | 最新版本 / 日期 | 许可证 | 一句话架构定位 | 活跃度 |
|---|---|---|---|---|
| Leptos | 0.8.19(2026-04)、0.9.0-alpha(2026-05) | MIT | Web 全栈框架,细粒度 signal 响应式(reactive_graph) | 很高 |
| Sycamore | 0.9.2 | MIT | Web 框架,细粒度响应式(Reactivity v3,2024-11) | 中(节奏慢于 Leptos) |
| Dioxus | 稳定 0.7.x(2025-11)、0.8.0-alpha(2026-05) | MIT OR Apache-2.0 | RSX + VDOM(带 signals),多渲染目标(web/webview/native) | 极高(VC 资助) |
| Blitz | 0.2.x,pre-alpha→beta | MIT OR Apache-2.0(stylo_taffy 含 MPL-2.0) | 模块化 HTML/CSS 渲染引擎(Stylo+Taffy+Parley+Vello) | 高,目标 2026 生产可用 |
| Freya | **0.4.0(2026-07-16)** | MIT | Skia 渲染的原生声明式 GUI,0.4 起脱离 Dioxus 自研响应式核心 | 高(基本单人维护) |
| Xilem/Masonry | xilem 0.4.0(2025-10) | Apache-2.0 | view diffing 前端 + Masonry retained widget 树(Linebender) | 高(2026Q1 大量进展) |
| Slint | 1.17.1(2026-07-07) | GPL-3.0 OR Royalty-Free OR 商业 | 独立 DSL(.slint)编译到属性绑定运行时,嵌入式+桌面 | 极高(商业公司) |
| Iced | 0.14.0(2025-12-07) | MIT | Elm 架构(Message/update/view),COSMIC 桌面在用 | 高 |
| egui | 0.35.0(2026-06-25) | MIT OR Apache-2.0 | 立即模式 GUI,工具类应用事实标准 | 极高 |
| Makepad | makepad-widgets 1.0.0(2025-05) | MIT OR Apache-2.0 | 全 GPU shader 渲染 + Live DSL 热重载 | 中(crates.io 发版稀疏,repo 活跃) |
| GPUI | crates.io 0.2.2(2025-10);Zed 主仓持续演进 | Apache-2.0 | Zed 的混合立即/保留模式 GPU 框架,平台原生渲染后端 | 主仓极高,crates.io 发版滞后(18 个月 3 次) |
| Tauri | 2.11.5(2026-07-01) | Apache-2.0 OR MIT | 系统 WebView 壳 + Rust 后端;Verso(Servo)运行时实验中 | 极高 |

### 1.2 逐个分析与"可复用部分"

**Leptos**:响应式内核 `reactive_graph`(0.2.x)是**独立 crate**,算法源自 Reactively(push-pull 混合、惰性 memo),经过三年生产打磨,与 DOM 无耦合——官方文档明确支持用它驱动任意 GUI。但注意:视图层 tachys 曾设计过泛型 Renderer 以支持非 DOM 后端,**因为泛型导致灾难性编译时间和链接错误,在 0.7 发布前被移除**(issue #1743)。→ **可复用:reactive_graph(直接依赖或 fork);不可复用:tachys 视图层。tachys 的失败是我们的重要前车之鉴:靠泛型做渲染器抽象在 Rust 里代价极高,编译期代码生成(我们的路线)恰好绕开这个坑。**

**Sycamore**:0.9 的 Reactivity v3 用单一 `Root` arena 管理响应式图,取代了到处 `Rc<RefCell>` 的旧设计,实现干净、体量小,适合作为**自研响应式运行时的参考实现**(相比 reactive_graph 更简单,无 Send+Sync 负担)。Web-only,无桌面野心。→ 可复用:sycamore-reactive 的设计思路。

**Dioxus + Blitz**:Dioxus 0.7(2025-11)最大卖点是 Rust 热更新(hot-patching)和 dioxus-native(Blitz + wgpu 全 GPU 绘制)。架构仍是"组件重跑 + VDOM diff",signals 用于把 re-render 限制在组件粒度,**不是节点级细粒度**。Blitz 本身与 Dioxus 解耦:blitz-dom(DOM 树)+ Stylo(Firefox 的 CSS 引擎)+ Taffy + Parley + Vello/AnyRender + AccessKit(blitz-shell),官方定位"radically modular",明确欢迎第三方前端驱动。状态 pre-alpha,官方声明不建议生产使用,目标 2026 年内 production-ready。→ **可复用:Blitz 是"我们只写编译器和响应式层、渲染整个外包"的候选路径(编译产物直接调 blitz-dom 的节点变更 API);风险是其成熟时间表不受我们控制,且拉进 Stylo 这个巨型依赖(编译时间、二进制体积、MPL)。**

**Freya**:**0.4.0 于 2026-07-16(本周)发布,彻底移除 Dioxus 依赖**,改用自研 freya-core("components-based runtime、reactive primitives、**UI tree diffing**")+ 自研布局引擎 Torin + Skia(skia-safe)渲染 + winit + AccessKit。是我们最直接的对标物,但注意三点坑:(a) 仍做 tree diffing,组件函数重跑,不是编译期细粒度;(b) skia-safe 是重型 C++ 绑定,交叉编译(尤其鸿蒙)痛苦;(c) 基本单人项目(marc2332),0.4 刚大改,API 极不稳定。→ 可复用:Torin 的布局思路、其组件库的 API 设计参考;直接复用代码价值低。

**Xilem/Masonry**:Xilem 是 SwiftUI 式 view diffing(每次重建轻量 view 树、diff 后补丁到 retained 层)。**真正值钱的是下层 Masonry**:一个明确"bring your own frontend"的 retained widget 树,2026Q1 完成了新布局系统、`imaging` 渲染抽象(可用 vello_cpu 纯 CPU 渲染)、ui-events 抽象(IME 等系统集成不再绑死 winit)、widget 集扩充(Svg/Split/Switch/RadioButtons 等),AccessKit 深度集成。→ **可复用:Masonry 是"编译产物直接驱动现成 widget 树"的另一候选(比 Blitz 轻,无 CSS 引擎,Apache-2.0);坑:widget 集仍偏基础,styling 系统远不如 CSS 完整,API pre-1.0。**

**Slint**:商业公司驱动,发版稳定(1.17.1,2026-07),平台矩阵最全(桌面+嵌入式+Android/iOS 预览)。架构上 .slint DSL 编译成属性绑定图,**证明了"编译 DSL → 原生渲染"商业上可行**。但 DSL 是独立语言:逻辑仍要写在 Rust 侧、通过 callback/property 桥接,表达力受限;许可证三选一(GPL / 带署名要求的 Royalty-Free / 商业)对库型产品是采纳摩擦。→ 可复用:无代码层面复用,但其编译器架构(property dependency graph、常量折叠)值得研读。

**Iced**:0.14(2025-12)加入 reactive rendering、IME 支持、headless testing、热重载,被 COSMIC 桌面背书。Elm 架构 = 每次 update 后重建 view + diff,性能靠 widget 树 diff 优化。→ 可复用:其 wgpu 渲染管线(iced_wgpu)和 cosmic-text 集成经验;架构与我们路线相反。

**egui**:立即模式,每帧重建。心智模型简单但与"编译期细粒度更新"哲学完全相反,无障碍支持依赖 AccessKit 但语义树每帧重建。→ 对我们只有参考价值:其集成层(egui-winit、egui-wgpu)是 winit/wgpu 版本适配的活教材。

**Makepad**:全 GPU、自带 Live DSL 热重载和 shader 化样式,技术激进。1.0(2025-05)后 crates.io 再无大版本,生态采纳极窄(主要是 Robius/Robrix 社区)。→ 可复用:少;其 DSL 热重载思路可参考。

**GPUI + gpui-component**:Zed 出品,hybrid immediate/retained,渲染用平台原生 API(macOS Metal;Win/Linux 由 blade/DirectX 路径,文本走 DirectWrite/CoreText/font-kit)。**没有独立发版承诺**(crates.io 18 个月 3 次发布,滞后主仓),API 随 Zed 需求破坏性变更。longbridge/gpui-component(60+ 组件,shadcn 风格)证明了 GPUI 可在 Zed 外做严肃产品,但等于绑定 Zed 的开发节奏。→ 可复用:其 retained element tree + taffy(Zed 维护自己的 taffy fork)的性能实践、文本系统平台分层设计;不宜作为依赖。

**Tauri**:WebView 方案,与我们"原生渲染"定位正交。但 **tauri 组织维护的周边 crate 是桌面集成的事实标准:muda(菜单)、tray-icon(托盘)、rfd 生态兼容**,全部可直接复用。Verso(Servo webview)集成仍属实验性。→ 可复用:muda/tray-icon/(wry 不用);另外 Tauri 的 mobile 打包工具链设计值得参考。

---

## 2. 基础设施逐个评估

| Crate | 版本 / 日期(已核实) | 许可证 | 定位与可复用判断 |
|---|---|---|---|
| winit | 0.30.13(2026-03);0.31.0-beta.2(2025-11,拆分为 winit-core/winit-x11/winit-wayland 等模块化工作区) | Apache-2.0 | 窗口/输入事实标准。**直接复用**。风险:0.31 大重构在途,API 将破坏;**无 OpenHarmony 后端**(issue #4081 无维护者回应)。 |
| wgpu | 29.0.4 / 30.0.0(2026-07-01) | MIT OR Apache-2.0 | 跨平台 GPU 抽象。**直接复用**。社区已验证可跑 OpenHarmony(GL backend,richerfu 的 wgpu-demo-with-winit)。发版节奏快(约 12 周一个 major),需要锁版本策略。 |
| Vello(classic) | 0.9.0(2026-05-15) | MIT OR Apache-2.0 | GPU compute-centric 2D 渲染。质量高但需要较新 GPU 特性。 |
| vello_cpu / vello_hybrid(sparse strips) | 0.0.7(2026) | 同上 | **新一代路线**:CPU 光栅 + GPU 合成(hybrid,"beta 质量"),纯 CPU(vello_cpu)可配 softbuffer 兜底低端设备/软渲染环境。Masonry 已切到可用 vello_cpu 的 imaging 抽象。**推荐作为渲染层主选,风险是 0.0.x 版本号所示的不成熟。** |
| Parley | 0.11.0(2026-06-26) | MIT OR Apache-2.0 | 富文本布局(styled spans、BiDi、行断、编辑)。2026 修复了大段落非线性性能 bug,补充 AccessKit 文本属性、macOS 系统字体枚举。已被 Blitz、Masonry、**Bevy** 采用。**推荐**。风险:系统字体发现/回退(尤其 CJK)仍在完善;Glifo(glyph atlas 缓存)刚拆出仍在孵化。 |
| cosmic-text | 0.19.0(2026-04-22) | MIT OR Apache-2.0 | System76 的文本全家桶(shaping 已切到 **harfrust** + swash 渲染),Iced/COSMIC 生产验证。**Parley 的备胎**:更成熟的 fallback 行为,但富文本样式模型较弱,与 AccessKit 无深度集成。 |
| swash | 0.2.9(2026-06-12) | Apache-2.0 OR MIT | 字体内省 + glyph 光栅(含彩色 emoji)。Parley/cosmic-text 共同的底层。复用。 |
| harfrust | 0.12.0(2026-07-03) | MIT | HarfBuzz 官方组织的 Rust port(对齐 HarfBuzz 13.0,慢 <25%),基于 read-fonts,是 rustybuzz 的继任者。**shaping 首选**。 |
| taffy | 0.10.1(2026-06/07) | MIT | Flexbox + Grid + Block 布局,Blitz/Bevy/Zed(fork)/Floem 都在用。**直接复用,几乎无争议**。 |
| AccessKit | 0.24.1(2026-06-12) | MIT OR Apache-2.0 | 无障碍树抽象 + 平台 adapter:Windows(UIA)、macOS(NSAccessibility)、Unix(AT-SPI)、Android、winit 封装(accesskit_winit)。RustWeek 2026 有专题演讲,生态标准。**直接复用。无 HarmonyOS adapter——需自研桥接 ArkUI 无障碍 API。** |
| softbuffer | 0.4.8(2025-12) | MIT OR Apache-2.0 | CPU 帧缓冲上屏(配 vello_cpu/tiny-skia 做无 GPU 兜底)。复用。 |
| tiny-skia | 0.12.0(2026-02) | BSD-3-Clause | Skia 光栅子集的 CPU port。若采用 vello_cpu 则基本不需要;保留为备选。 |
| muda / tray-icon | 0.19.3 / 0.24.1(2026-06) | MIT OR Apache-2.0 | 原生菜单 / 托盘(Tauri 组织维护,亿级下载)。**直接复用**(桌面三平台;鸿蒙不适用)。 |
| rfd | 0.17.2(2026-01) | MIT | 原生文件对话框。直接复用。 |

**一个重要的生态观察**:Parley + Vello + Taffy + AccessKit + winit 这套"Linebender 栈"在 2026 年已经被 Blitz、Masonry、Bevy 三个大项目共同压测,修 bug 的速度和方向由多方共担——选它不是选单个库,而是加入一个正在收敛的联盟。cosmic-text/Iced 栈和 skia-safe/Freya 栈是另外两个孤岛。

---

## 3. 三个核心问题

### 3.1 "细粒度响应式 + 原生渲染",现在谁最接近?

按接近程度排序:

1. **Floem(lapce/floem,2026-02 仍在更新,MIT)**——虽不在指定清单但必须提:它就是"leptos_reactive 式 signals + 原生渲染(wgpu/vello/vger,可选 Skia)+ taffy"的现成组合,README 原话 "view tree 只构建一次",signal 直接绑定到 view 属性。**坑**:响应式是纯运行时的(闭包订阅),没有编译期优化;pre-1.0 且明确会破坏 API;团队精力被 Lapce 编辑器牵引;组件生态薄。
2. **Freya 0.4**——自研响应式核心 + Skia,刚发布。**坑**:freya-core 仍是"组件重跑 + UI tree diffing",本质是运行时 Dioxus-lite 而非细粒度;Skia C++ 绑定的交叉编译负担;单人项目的 bus factor。
3. **Leptos(reactive_graph)+ 自研视图层**——响应式内核最成熟,但视图层需要从零做(tachys 不可用,泛型渲染器已被证伪)。这其实就是"我们自己"的起点方案。
4. **Blitz**——原生渲染最完整(完整 CSS),但自身无响应式,dioxus-native 前端是 VDOM。让细粒度信号直接驱动 blitz-dom 在架构上成立,但没人做过。

**结论:没有任何项目做到"编译期把模板编译成细粒度更新代码"。Floem/Freya 的细粒度都是运行时机制,Svelte 的核心创新(编译器承担依赖分析、运行时只剩 signal 原语和定点 DOM/widget 操作)在 Rust 原生 GUI 领域是无人区。** 这既是机会(定位清晰)也是警告(tachys 的教训说明 Rust 的类型系统会让"视图抽象"变得昂贵——但我们用宏生成具体代码而非泛型抽象,恰好是绕开该坑的路线)。

### 3.2 推荐的"复用栈"

| 层 | 首选 | 理由 | 风险与对冲 |
|---|---|---|---|
| 窗口/输入 | **winit 0.30 → 0.31** | 事实标准,AccessKit/IME/多平台适配现成 | 0.31 破坏性重构在途 → 用薄封装层隔离;**鸿蒙无后端 → 自研 winit-ohos 风格后端**(XComponent + NativeWindow,参考 Servo 上游的 OpenHarmony 支持和 ohos-rs 的 winit fork 实验) |
| GPU 抽象 | **wgpu** | 唯一同时覆盖 Vulkan/Metal/DX12/GL(鸿蒙走 GL/Vulkan)的成熟方案 | major 版本节奏快 → 锁版本、随 Vello 升级 |
| 2D 渲染 | **vello_hybrid,vello_cpu+softbuffer 兜底** | sparse strips 是 Linebender 主推方向,CPU/GPU 同源算法,低端设备可软渲染;Masonry 已验证 | 0.0.x 成熟度 → 渲染调用收敛到自有 `Painter` trait,保留切换 classic Vello / tiny-skia 的能力 |
| 文本 | **Parley + harfrust + swash** | 富文本 spans、AccessKit 文本属性、与 Vello 的 Glifo glyph 缓存协同;Bevy/Blitz 共建 | CJK 字体回退和 IME 成熟度未知 → 原型期专项测试;备胎 cosmic-text(生产验证更多,鸿蒙字体桥接需自己做,两者皆然) |
| 布局 | **taffy 0.10** | 无争议标准,Flexbox+Grid+Block | 几乎无;注意 measure 函数与文本层的集成方式参考 Blitz |
| 无障碍 | **AccessKit + accesskit_winit** | 唯一选项,四大平台 adapter 齐全 | 无鸿蒙 adapter → 立项自研 accesskit_ohos(桥 ArkUI accessibility),或先接受鸿蒙无 a11y 并公开说明 |
| 桌面集成 | **muda + tray-icon + rfd** | Tauri 组织维护,亿级下载 | 鸿蒙不适用,需条件编译 |
| 响应式内核 | **fork reactive_graph 或参考 sycamore-reactive v3 自研** | reactive_graph 算法成熟(Reactively);但其 Send+Sync/Arc 设计为 SSR 服务,桌面单线程 UI 可用更快的 thread-local arena | 先用 reactive_graph 出原型验证编译器,再决定是否换自研 arena 实现 |
| widget 层 | **自研薄 widget 树;把 Masonry 作为研究对象而非依赖** | 编译产物需要直接定点操作 widget 属性,自有树可为此定制;Masonry 的 pass 架构/a11y 集成直接抄作业 | 若自研树进度失控,退路是编译到 Masonry(Apache-2.0,明确支持第三方前端) |

**明确不推荐**:skia-safe(C++ 绑定拖累鸿蒙交叉编译)、Stylo/Blitz 全家桶(除非我们决定做"HTML/CSS 兼容"产品——那会稀释差异化并背上巨型依赖)、GPUI(无发版承诺)。

### 3.3 差异化定位怎么立得住

**vs Slint(独立 DSL)**:Slint 的模板是另一门语言,业务逻辑与 UI 之间隔着 property/callback 桥,复杂状态逻辑表达力差;许可证三选一对开源库作者和商业闭源用户都有心智成本。我们:**模板宏内嵌在 Rust 里,`$state`/`$derived` 直接是 Rust 变量,编译期同样能做依赖分析(proc-macro 能看到完整模板 AST),MIT/Apache 双许可零摩擦**。Slint 反过来验证了"编译到属性绑定图"的性能路线可行——我们是把这条路线搬进宿主语言。

**vs Dioxus(RSX + VDOM-ish)**:Dioxus 0.7 的 signals 只把重渲染限制到组件粒度,组件内仍是模板重跑 + diff;其战略重心是全栈 web + 移动,native 渲染(Blitz)是多目标之一。我们:**无 diff——编译器已经知道"这个 signal 只影响这个文本节点的这个属性",直接生成 `node.set_text(...)` 调用;桌面原生是唯一目标而非之一**。运行时体积和更新延迟是可量化的宣传点(参考 Svelte vs React 的论证结构)。

**vs Xilem(view diffing)**:Xilem 每次状态变化重建 view 树再 diff 到 Masonry,状态传递依赖 lens/adapt 的类型体操,学习曲线陡。我们:**signal 图取代 view 重建,更新路径在编译期定死,心智模型是"变量变了 UI 就变"而非"描述 UI 的函数被重新求值"**。同时我们大方复用 Linebender 的全部底层——差异化只压在响应式模型这一层,竞争面最小。

**立得住的前提(诚实的自检)**:(a) Svelte 风格的价值主张在 Rust 里必须重新证明——Rust 没有 JS 的 bundle size 焦虑,卖点要转译成"更新延迟、内存占用、增量编译速度、心智模型";(b) proc-macro 模板的 IDE 体验(rust-analyzer 补全、错误定位)决定生死,这是 Slint 拿独立 DSL + LSP 换来的东西,我们必须用 macro 内 span 映射做到接近;(c) **鸿蒙一等公民支持是目前所有对手(Slint 无、Dioxus 草稿 PR、其余为零)都没有的,是最稀缺的差异化,但也最贵**——Servo 的 OpenHarmony 上游支持(华为系工程师 jschwe 维护)和 ohos-rs 工具链(SDK action、napi 绑定、测试 runner)证明技术上可行。

---

## 4. HarmonyOS 专题(现状与路径)

已核实的现状(2026-07):

- **winit 上游无支持**:issue #4081(2025-01 开)无维护者回应、无人认领;ohos.rs 社区有基于 fork 的 winit/glutin-winit 实验(2025-01 博客)。
- **wgpu 可用**:社区 demo(wgpu + winit fork,GL backend)已在 OpenHarmony 跑通;Servo(用 wgpu 家族的 surfman/webrender 路线)已正式上游支持 OpenHarmony(servoshell,PR #32594 已合并)。
- **Dioxus**:OpenHarmony 支持 PR #4508 仍是 Draft,依赖 wry/tao 上游改动,维护者表态有兴趣但"先稳基础"。
- **AccessKit / muda / tray-icon / rfd**:均无鸿蒙支持。
- 结论:**渲染层(wgpu)和构建工具链(ohos-rs、setup-ohos-sdk)已有社区地基,窗口/生命周期/IME/无障碍四件事需要自研**。建议原型期先做"XComponent + wgpu 三角形 + 触摸事件"的最小验证,再决定鸿蒙是 tier-1 还是 tier-2 承诺。

---

## 5. 风险清单

| 风险 | 等级 | 对冲 |
|---|---|---|
| vello sparse strips 尚在 0.0.x,API 与性能未定型 | 高 | Painter 抽象层 + classic Vello 退路 |
| 鸿蒙窗口/IME/a11y 全自研,工作量不可控 | 高 | 原型期探底;必要时降级为 tier-2 平台 |
| proc-macro 模板的 rust-analyzer 体验 | 高 | 立项即验证(span 映射、错误恢复);参考 leptos view! 宏的现状与抱怨 |
| winit 0.31 破坏性重构 | 中 | 薄封装隔离;关注 ui-events(Masonry 已用它解耦 winit) |
| Parley CJK 回退/IME 集成不成熟 | 中 | 与 cosmic-text 做 A/B 原型 |
| reactive_graph 的 Send+Sync 开销不适配单线程 UI | 中 | 先复用后替换,接口对齐使切换成本可控 |
| Blitz 2026 年内成熟后挤压"轻量原生渲染"生态位 | 低-中 | 我们的差异化在响应式编译层,渲染层甚至可以未来兼容 Blitz |

## 6. 待原型验证的开放问题

见文末结构化输出(与本报告同源):编译目标选自有树还是 Masonry、鸿蒙最小可行窗口层、宏内模板的 IDE 体验、Parley vs cosmic-text 的 CJK/IME 实测、reactive_graph 单线程性能。

---

## 7. 来源链接

**框架**
- Leptos releases / 0.8: https://github.com/leptos-rs/leptos/releases · https://docs.rs/crate/leptos/latest
- reactive_graph: https://crates.io/crates/reactive_graph · 自定义渲染器讨论 https://github.com/leptos-rs/leptos/issues/1743
- Sycamore 0.9 公告: https://sycamore.dev/post/announcing-v0-9-0
- Dioxus 0.7: https://dioxuslabs.com/blog/release-070/ · https://github.com/DioxusLabs/dioxus/releases/tag/v0.7.0
- Blitz: https://github.com/DioxusLabs/blitz · https://blitz.is/about
- Freya(v0.4.0,2026-07-16): https://github.com/marc2332/freya · 重写 PR https://github.com/marc2332/freya/pull/1351
- Xilem/Masonry: https://github.com/linebender/xilem · Linebender 2026 Q1: https://linebender.org/blog/tmil-25/
- Slint(1.17.1)与许可证: https://crates.io/crates/slint · https://slint.dev/pricing · https://github.com/slint-ui/slint/blob/master/LICENSE.md
- Iced 0.14: https://crates.io/crates/iced · https://www.phoronix.com/news/Iced-0.14-Rust-GUI-LIbrary
- egui 0.35: https://crates.io/crates/egui · https://github.com/emilk/egui/releases
- Makepad 1.0: https://crates.io/crates/makepad-widgets · https://news.ycombinator.com/item?id=43971829
- GPUI: https://crates.io/crates/gpui · https://github.com/zed-industries/zed/tree/main/crates/gpui · gpui-component: https://github.com/longbridge/gpui-component
- Tauri 2.11: https://crates.io/crates/tauri · Verso 集成: https://v2.tauri.app/blog/tauri-verso-integration/ · https://github.com/versotile-org/tauri-runtime-verso
- Floem: https://github.com/lapce/floem

**基础设施**
- winit(0.30.13 / 0.31-beta): https://crates.io/crates/winit · OpenHarmony issue: https://github.com/rust-windowing/winit/issues/4081
- wgpu(29/30): https://crates.io/crates/wgpu
- Vello 0.9 / sparse strips: https://crates.io/crates/vello · https://github.com/linebender/vello/releases · https://linebender.org/blog/tmil-24/
- Parley 0.11: https://crates.io/crates/parley · https://github.com/linebender/parley
- cosmic-text 0.19: https://crates.io/crates/cosmic-text · https://github.com/pop-os/cosmic-text
- swash 0.2.9: https://crates.io/crates/swash
- harfrust 0.12: https://crates.io/crates/harfrust · https://github.com/harfbuzz/harfrust
- taffy 0.10: https://crates.io/crates/taffy · https://github.com/DioxusLabs/taffy
- AccessKit 0.24: https://crates.io/crates/accesskit · https://accesskit.dev/ · RustWeek 2026: https://2026.rustweek.org/talks/matt/
- softbuffer / tiny-skia: https://crates.io/crates/softbuffer · https://crates.io/crates/tiny-skia
- muda / tray-icon / rfd: https://crates.io/crates/muda · https://crates.io/crates/tray-icon · https://crates.io/crates/rfd

**HarmonyOS**
- ohos.rs winit for Harmony: https://ohos.rs/blog/2025-01-24
- ohos-rs / openharmony-rs: https://github.com/ohos-rs/ohos-rs · https://github.com/openharmony-rs
- wgpu OpenHarmony demo: https://github.com/richerfu/wgpu-demo-with-winit
- Servo OpenHarmony 支持: https://github.com/servo/servo/pull/32594
- Dioxus OpenHarmony PR(Draft): https://github.com/DioxusLabs/dioxus/pull/4508

**未能联网核实、仅基于训练数据的点**:GPUI 在 Linux/Windows 上的具体渲染后端(blade/DirectX 细节)、Zed 的 taffy fork 现状、Floem 渲染后端组合的最新细节(其 repo 2026-02 更新为准)、Slint 移动端支持的最新成熟度。
