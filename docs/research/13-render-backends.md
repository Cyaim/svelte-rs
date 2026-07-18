# 13 · 候选渲染后端逐一优劣对比(对准本项目工作负载)

> 调研日期:2026-07-18。所有版本号/维护状态均于当日联网核实(crates.io API、GitHub releases、
> Linebender 博客);个别工程估计值(启动毫秒数、二进制增量)标注"⚠ 估计"。
> 前置阅读:DESIGN.md ADR-3/ADR-5、[05 号调研](05-rendering-stack.md)、
> `crates/sv-shell/src/render.rs`、[CSS-SUPPORT.md](../CSS-SUPPORT.md) ⏳ 项。

---

## 0. TL;DR

- **近期(M1,补全编译器/布局期间)**:维持 ① tiny-skia+softbuffer 不动,但立刻抽 `Painter` trait
  把 render.rs 的绘制调用收口——这是所有后续选项的免费保险。
- **中期(M2 桌面)**:主后端 ② **vello classic 0.9(wgpu 29)**,兜底 ③ **vello_cpu**(sparse
  strips,替换 tiny-skia 路径);⏳ 项(渐变/box-shadow/blur/transform/clip-path/图片)一次到位,
  且与 Parley 文本栈同生态零缝合。
- **OHOS 特化(M3)**:③ **vello_hybrid via wgpu-GLES** 首选(为 WebGL2/GLES 档 GPU 设计),
  vello_cpu 兜底;vello classic 在 OHOS-GLES 上不可用(需完整 compute,wgpu GL 后端不满足)。
- ④ skia-safe、⑤ femtovg、⑥ wgpu 自绘、⑦ OS 原生:全部**不作主后端**;各自的保留价值与
  排除理由见小结卡。此结论与 ADR-3 一致,本次核实未发现推翻性事实,但把"为什么不是别家"
  逐一补齐了证据。

---

## 1. 我们的工作负载画像(评判基准)

从 `sv-shell/src/render.rs` 现状 + CSS-SUPPORT ⏳ 项归纳,后续所有打分都对着这张表:

**现有指令集**(v0,已跑通):纯色圆角矩形填充、圆角描边(中心线内缩)、文本字形
(fontdue A8 覆盖手工 blend)、复选框、组不透明度近似(祖先链 alpha 乘积,无合成层)。
逻辑坐标布局、物理坐标绘制(×scale),每帧全量重绘。

**即将需要**(CSS-SUPPORT ⏳,按优先级):
1. `transform: translate/scale/rotate`(动画刚需,标注"优先级高")→ 需要矩阵通道;
2. `filter`/`backdrop-filter` 毛玻璃(桌面质感刚需,"列高优先")→ 需要合成层 + 高斯模糊;
3. `box-shadow` → 模糊圆角矩形原语或通用 blur;
4. 渐变 linear/radial/conic;
5. `background-image`/图片(解码是独立议题,渲染侧要纹理/位图通道);
6. `mask`/`clip-path` 矢量裁剪;
7. `z-index`/stacking context → 分层合成;
8. 广色域 display-p3(M2 后评估)。

**负载特征**(与游戏/地图/富文档截然不同):
- 小场景树:典型桌面 UI 数百到低千节点,单帧图元数 <10^4;
- 更新稀疏:细粒度响应式意味着多数帧只有零星属性变化,大量时间完全静止(应当零功耗);
- **HiDPI 文本为主**:一帧里字形数量远超几何图元,文本质量=第一观感;
- 交互延迟敏感(点击→上屏一帧内),吞吐不敏感(不追 10 万图元/帧);
- 冷启动敏感(桌面工具型应用,秒开是底线);
- 目标矩阵:Win/mac/Linux + OHOS(GLES-only,ADR-5)。

由此得出四条**否决线**:无 OHOS 可行路径 = 硬伤;文本上屏质量差 = 硬伤;构建/体积与
"Svelte 式极小运行时"定位相悖 = 硬伤;维护停摆 = 硬伤。

---

## 2. 逐后端小结卡

### ① 现状:tiny-skia + softbuffer + fontdue(纯 CPU)

| 项 | 核实结果(2026-07-18) |
|---|---|
| 版本 | tiny-skia **0.12.0**(2026-02-02,BSD-3-Clause,3700 万+下载);softbuffer **0.4.8**(2025-12-13,MIT/Apache);fontdue **0.9.3**(**2025-02-12**,MIT/Apache/Zlib) |
| 维护 | tiny-skia/softbuffer 稳定低频维护;**fontdue 自 2025-02 起无新版本,事实维护停滞** |

**优势**:零 GPU 依赖、零启动成本(无 adapter/管线编译)、内存占用≈一块帧缓冲、构建秒级、
二进制增量 <1MB(⚠ 估计)、行为 100% 确定(CI 截图测试无驱动变量)、OHOS 理论可行(纯
CPU 代码无平台风险)。对"小 UI 树 + 局部更新"其实够快——加脏区(damage rect)后数百节点
UI 的 CPU 帧成本可控。

**劣势对照 ⏳ 项**:渐变 tiny-skia 有(linear/radial,conic 无);**高斯模糊/毛玻璃没有**
(box-shadow、backdrop-filter 全堵死,自写可分离模糊在 4K HiDPI 上 CPU 成本不可接受);
transform 有(路径级仿射);矢量裁剪有(clip mask);图片有(Pixmap 绘制)。即 ⏳ 清单里
**权重最高的两项(blur 系)正是 CPU 栈的死穴**。文本侧 fontdue 无整形(shaping)、无
fallback、无 BiDi,且已停更——换 Parley 时它整个被替换,而 tiny-skia 无字形 API,swash
光栅后仍要手工 blit(render.rs `blend_pixel` 那条路)。

**结论**:作为 v0 原型和永久 CI 基准保留;不值得再往里投能力开发(每一项 ⏳ 都在为
迁移目标重复造轮子)。softbuffer 无 OHOS 后端(⚠ 未见支持声明),OHOS 上 CPU 兜底
需改走"CPU 像素→GLES 纹理上屏"。

---

### ② vello classic(wgpu compute)

| 项 | 核实结果 |
|---|---|
| 版本 | **0.9.0**(2026-05-15,依赖 **wgpu 29**;wgpu 最新已到 30.0.0/2026-07-01,vello 下版本预计跟进) |
| 许可/维护 | Apache/MIT;Linebender 持续快速迭代(0.7→0.8→0.9 半年三版),官方明确 classic 线继续开发、无弃用计划 |
| 0.9 亮点 | brush_transform(渐变/图片画刷独立变换)、image atlas 跨帧驻留、font_embolden、模糊图片半像素修复 |

**能力对照 ⏳ 项:唯一"全绿"的候选**——渐变(linear/radial/sweep)、**模糊圆角矩形原语
(box-shadow 直达)**、层模糊(backdrop-filter 可组合)、任意仿射 transform、矢量 clip 层、
blend/合成模式、图片(atlas 驻留)、彩色 emoji。imaging model 即"CSS 渲染所需全集"
(Blitz 用它渲 HTML 是活证)。

**工作负载匹配**:小场景吞吐完全溢出(它是按 10^5 图元/帧设计的);代价是**每帧全场景
重编码 + GPU dispatch 的固定开销**——对我们"多数帧只变一个文本"的模式,固定成本占比高,
需要靠"静止帧不重绘"(现有 on_mutate 驱动已具备)而不是靠增量渲染;vello 本身无脏区
概念(全帧重画,GPU 上无所谓)。功耗上,静止=零提交,符合桌面预期。

**成本**:启动需 wgpu adapter/device + compute 管线编译,冷启动 +50~300ms(⚠ 估计,
D3D12 上首次 shader 编译占大头,可用管线缓存缓解);二进制 +8~15MB(wgpu+naga+vello,
⚠ 估计);构建从秒级变分钟级;显存/内存驻留数十 MB。**要求完整 compute shader 支持**:
D3D12/Metal/Vulkan 都满足;**wgpu-GL(ES) 后端不满足 → OHOS-GLES 上不可用**,这是它
唯一的平台缺口。

**文本栈**:与 Parley 同生态,glyph run API 直接吃 skrifa outline,零缝合;字形走 GPU
路径渲染,HiDPI 下质量佳;1x 低分屏无 hinting/次像素(Glifo 正在补 hinting,详见 ③)——
我们"HiDPI 文本为主"的画像恰好避开其短板。

**结论**:桌面主后端的最优解,ADR-3 维持成立。

---

### ③ vello_hybrid 与 vello_cpu(sparse strips)

| 项 | 核实结果 |
|---|---|
| 版本 | 双双 **0.0.9**(2026-05-30);2026 年内 0.0.5→0.0.9 五连发,投入重心明显 |
| 官方定性 | vello_cpu:"**broadly usable**,imaging model 相当完整,性能有竞争力",但无 API 稳定承诺;vello_hybrid:"**early stages**,特性尚未与 vello_cpu 对齐"(约 beta) |
| 能力 | 0.0.7 起 filter effects(实验性);文本装饰(下划线 skip-ink)、VARC 字形;vello_cpu 有 u8 快速/f32 精确双管线,fearless_simd + 多线程(>4 线程收益递减);Glifo 0.1.x 字形渲染(outline/彩色 emoji/**hinting**)三形态共用 |

**vello_cpu**:定位=tiny-skia 的官方继任者,imaging model 与 GPU 版一致。对我们的意义:
**兜底路径从"能力残缺的 tiny-skia"升级为"能力齐全的 CPU 渲染器"**——blur/渐变/裁剪在
CPU 档也有(慢,但正确),三档视觉一致成为可能。多线程+SIMD 对小 UI 树够用。风险:
0.0.x 破坏性变更常态(五个月三次 minor 均含 breaking),image 资源 API 明说要改。

**vello_hybrid**:CPU 粗光栅 + GPU 细光栅,显式面向 **WebGL2/GLES 档 GPU**——正中
ADR-5"OHOS 只有 wgpu-GLES"的现实,是 OHOS 的第一人选。但成熟度是三兄弟里最低的
("early stages"),filter/blur 等特性落后于 vello_cpu(2026-07 仓库活动显示 filter region
management 仍在进行中);**押注它 = 押注 2026H2~2027 的补齐节奏**。风险敞口有限:
imaging model 同族,hybrid 不齐就先用 vello_cpu 顶(OHOS 上 CPU 渲染中低复杂度 UI
可接受),API 由 Painter trait 隔离。

**文本栈**:Glifo 是 sparse strips 的原生字形通道(含 hinting——低分屏文本反而比 classic
好);Parley 直通。

**结论**:vello_cpu = 兜底位转正;vello_hybrid = OHOS 首选但**排期上做"可推迟决策"**
(M3 spike 时按当时成熟度定 hybrid or cpu)。

---

### ④ skia-safe

| 项 | 核实结果 |
|---|---|
| 版本 | **0.99.0**(2026-06-19),跟踪 Skia chrome/m150;bindings MIT + Skia 本体 BSD-3-Clause;维护正常(月更节奏) |

**能力**:七候选中唯一"超集"——⏳ 全项 + 次像素文本 + SkParagraph 富文本 + 色彩管理
(display-p3 现成)+ Ganesh/Graphite GPU 后端(gl/vulkan/metal/d3d)+ CPU 光栅。文本
质量业界基准(hinting/LCD 次像素/字体族回退全有,但要接 DirectWrite/CoreText 平台字管)。

**否决理由**(量级问题,非能力问题):
1. **构建**:C++ 大件。有 prebuilt 时下载数十 MB 二进制;无 prebuilt 组合(feature 变体、
   交叉目标)冷构建数十分钟且需 GN/clang 工具链——**`*-linux-ohos` 无官方 prebuilt,
   OHOS 交叉编译要自己维护 Skia 工具链配置**(Flutter-OHOS 证明可行,但那是引擎级
   团队的工程量,与我们"自研面收敛"的风险清单第 5 条直接冲突)。
2. **体积**:二进制 +15~40MB(⚠ 估计,取决 feature),与极小运行时定位相悖。
3. **生态缝合**:与 Parley/AccessKit/taffy 无协同,文本要么用 Skia 全家桶(再引 ICU)
   要么自己缝。
4. 供应链:Rust 项目里养一个 Skia 版本升级流(m150→m151→…)是长期税。

**保留价值**:若未来做"像素级对标浏览器"的产品(富文本编辑器),作为 Painter 的可选
后端接入,而非地基。

---

### ⑤ femtovg(GL / nanovg 血统)

| 项 | 核实结果 |
|---|---|
| 版本 | **0.25.1**(2026-05-29,MIT/Apache);2026 年活跃(0.23→0.25 三个月四版);后端 **OpenGL ES 3.0+ 与 wgpu**(0.24+ 新增,Slint 已有 renderer-femtovg-wgpu) |
| 采用 | Slint 的 GL 渲染器(生产验证);pn-editor 等 |

**能力对照 ⏳ 项**:渐变 linear/radial/**box gradient**(nanovg 式羽化矩形——box-shadow 的
**近似**而非真高斯);**无通用 filter/backdrop 模糊**(毛玻璃堵死);transform 有(2D 仿射);
裁剪只有**矩形 scissor,无路径裁剪**(clip-path ⏳ 项堵死);图片 pattern 有;合成模式有。
明确不支持:stroke dashing、path scissoring、自定义 shader。**⏳ 清单命中率约一半,
且缺的正是高优先级的 blur 系与矢量裁剪**。

**匹配度**:stencil-cover 即时模式,小场景快、启动快(GL 上下文即用)、依赖轻、
GLES 3.0 起步——**OHOS 兼容性其实是七家里最现成的之一**(EGL+GLES 标准路径)。
文本栈自带(shaping+atlas),但 Slint 文档自己都注明"text and path rendering quality
can sometimes be sub-optimal";与 Parley 对接要绕开其内置文本走自管 atlas,等于
自研文本上屏(见 ⑥)。

**结论**:能力天花板不够(blur/路径裁剪/合成层),文本质量存疑,不选。若 vello_hybrid
长期难产,它是"OHOS GL 应急后端"的候补——但那时更可能选 vello_cpu 兜底。

---

### ⑥ 直接 wgpu 自绘(GPUI/Makepad 式自研)

| 项 | 核实结果 |
|---|---|
| 参照物现状 | **GPUI**:2026 官方明确"community-facing work paused"("2026 要聚焦业务"),crates.io 发布属 courtesy、无独立维护;社区 fork gpui-ce 落后主线 381 commits、单位数合并 PR——**作为依赖已死,作为参考实现仍有价值**。Makepad:1.0(2025-05),无无障碍、DSL 封闭,不变。wgpu 本体 **30.0.0**(2026-07-01,MIT/Apache),OHOS-GLES 支持已合入(Dioxus 在用) |

**思路**:不接任何矢量库,针对我们的小指令集手写管线——GPUI 路线:圆角矩形/边框/阴影
全部 SDF 单 shader 搞定(模糊阴影有解析近似解),字形 swash 光栅进 atlas,一帧一两次
draw call。**这是"小 UI 树 + 局部更新 + HiDPI 文本"负载的性能理论最优解**,启动比
vello 快(无 compute 管线)、体积比 vello 小(无 naga 之外的负担有限)。

**否决理由**:能力增长曲线残酷——SDF 打发得了矩形/阴影,但 ⏳ 清单往后是**任意路径
(clip-path/mask)、conic 渐变、backdrop 模糊、图片滤镜**,每一项都是从零手写 GPU 算法;
最终会收敛成"自研一个小号 vello",正是风险清单第 5 条(维护面失控)点名要避免的。
GPUI/Makepad 能走这条路是因为渲染就是它们的产品核心;我们的核心在编译器+响应式。

**结论**:不选。但其**架构遗产要吸收**:UI 图元 fast-path(vello sparse strips 已在做
矩形快速路径)、字形 atlas 常驻、一层薄 abstraction 直出 draw list。

---

### ⑦ OS 原生(Direct2D / CoreGraphics / OHOS Drawing)

**现状**(平台 API 常青,无版本风险):Windows Direct2D/DirectWrite(windows-rs 绑定成熟);
macOS CoreGraphics/CoreText/CoreAnimation(objc2 系绑定);**Linux 没有"OS 原生矢量 API"**
——事实标准是 Cairo(GTK 系,能力老旧、无 GPU 合成保证),这是矩阵上的**结构性缺口**;
OHOS 有 ArkGraphics 2D(`OH_Drawing_*` NDK C API,系统渲染服务背后是 Skia 系,联网核实
支持 shadow/gradient/blur/文本引擎)。

**优势**:零第三方依赖、最小体积、平台文本质量天花板(ClearType/CoreText)、深色模式/
色彩管理免费。

**否决理由**:
1. **一份指令集 × 4 套实现**:每个 ⏳ 能力要写 3~4 遍(D2D effect graph、CG/CA layer、
   OH_Drawing),测试矩阵同倍数膨胀——单人/小团队不可承受;
2. **像素不一致**:四平台渲染结果不同(文本度量都不同),截图测试与"三档共享 imaging
   model"的既有裁决全部作废;布局依赖文本测量,连布局都会平台漂移;
3. Linux 缺口无解(Cairo 半死,Blend2D 是 C++ 又非"OS 原生");
4. CoreGraphics 本质 CPU 光栅(GPU 合成靠 CA layer 拼装),blur 语义与 D2D effect 不对齐。

**保留价值**:单平台"薄壳深耦合"产品(只发 Windows 的工具)可考虑 D2D;本项目不适用。
OHOS 的 OH_Drawing 值得记一笔:若 wgpu-GLES 在某些 SoC 驱动上翻车,它是 OHOS 侧
"系统兜底"的最后退路(代价是像素与桌面不一致)。

---

## 3. 横向对比总表

打分:● 好 / ◐ 及格或有条件 / ○ 差或缺失。"能力"列对照 §1 的 ⏳ 清单。

| 维度 | ① tiny-skia CPU | ② vello classic | ③a vello_cpu | ③b vello_hybrid | ④ skia-safe | ⑤ femtovg | ⑥ wgpu 自绘 | ⑦ OS 原生 |
|---|---|---|---|---|---|---|---|---|
| 版本(2026-07 核实) | 0.12.0 | 0.9.0 | 0.0.9 | 0.0.9 | 0.99.0 (m150) | 0.25.1 | wgpu 30.0.0 | 平台常青 |
| 成熟度 | ● 稳定 | ● 活跃迭代 | ◐ broadly usable | ○ early stages | ● 工业级 | ● 稳定 | —(自研) | ● |
| 小 UI 树/稀疏更新匹配 | ●(加脏区) | ◐ 固定开销偏高但静止零耗 | ● | ● | ◐ | ● | ●● 理论最优 | ● |
| HiDPI 文本上屏 | ◐ fontdue 停更、无整形 | ●(HiDPI 佳;1x 无 hinting) | ●(Glifo 含 hinting) | ● 同左 | ●● 次像素全有 | ◐ 质量存疑 | ◐ 全自研 atlas | ●● 平台文本 |
| 渐变 | ◐ 无 conic | ● | ● | ◐ 补齐中 | ● | ◐ 无 conic/真高斯 | ○ 手写 | ● |
| box-shadow/blur/毛玻璃 | ○ 死穴 | ●(模糊圆角矩形原语+层模糊) | ●(filter 实验性) | ◐ 落后于 cpu | ● | ○ 仅 box 近似 | ◐ SDF 近似阴影,backdrop 难 | ◐ 平台各异 |
| transform | ● | ● | ● | ● | ● | ● | ◐ 手写 | ● |
| 矢量裁剪 clip-path/mask | ● | ● | ● | ● | ● | ○ 仅矩形 scissor | ○ 手写 | ● |
| 图片 | ● | ●(atlas 驻留) | ◐ API 将变 | ◐ | ● | ● | ◐ 手写 | ● |
| 合成层/z-index 前景 | ○ | ● | ● | ● | ● | ◐ | ○ 手写 | ◐ 各异 |
| 启动 | ●● ~0 | ◐ +50~300ms ⚠估计 | ●● ~0 | ◐ GL 上下文级 | ◐ | ● | ◐ | ● |
| 构建成本 | ●● 秒级 | ◐ 分钟级纯 Rust | ● | ● | ○ C++ 大件/prebuilt | ● | ◐ | ●(绑定层) |
| 二进制增量 ⚠估计 | ●● <1MB | ◐ +8~15MB | ● +2~3MB | ● | ○ +15~40MB | ● +2~4MB | ◐ wgpu 底价 | ●● ~0 |
| Win/mac/Linux | ● | ● | ● | ● | ● | ●(GL 需注意 mac 弃用) | ● | ○ Linux 缺口 |
| **OHOS(GLES-only)** | ◐ 需自写上屏 | **○ 不可用**(需完整 compute) | ●(纯 CPU) | ●● 为此档设计 | ○ 无 prebuilt 自维护工具链 | ◐ GLES3.0 理论可行未验证 | ●(wgpu-GLES 已合入) | ◐ OH_Drawing |
| Parley 文本栈缝合 | ○ 手工 blit | ●● 同生态直通 | ●● Glifo | ●● Glifo | ○ 平行体系 | ○ 内置栈绕不开 | ◐ 自建 atlas | ○ 平台文本各写 |
| 许可 | BSD-3/MIT/Zlib | Apache/MIT | Apache/MIT | Apache/MIT | MIT+BSD-3(捆绑三方) | MIT/Apache | MIT/Apache | 平台 SDK |
| 维护信号 | ◐ fontdue 停滞 | ● Linebender+Blitz/Masonry/Bevy 共压 | ● 投入重心 | ◐ 追赶中 | ● rust-skia 月更 | ● Slint 背书 | ○ GPUI 已停社区维护(参照物) | ● |

---

## 4. 排序结论

**综合排序(对本项目)**:
vello classic(桌面主力)> vello_cpu(全平台兜底)> vello_hybrid(OHOS 首选、成熟度观察位)
≫ femtovg(OHOS 应急候补)> skia-safe(远期可选后端)> wgpu 自绘(仅吸收架构思想)
> OS 原生(不适用,OH_Drawing 记为 OHOS 最后退路)> 现状 CPU 栈(退役进行时,保留 CI 位)。

### 近期(M1 内,不换后端)
1. **立即抽 `Painter` trait**(ADR-3 已定):按现有指令集 + ⏳ 前四项(transform 矩阵、
   模糊圆角矩形、渐变画刷、clip 入栈/出栈)设计接口,对齐 Linebender `AnyRender`/`imaging`
   的形状而不自造第三套语义;tiny-skia 实现为第一个 impl,离屏 PNG 测试全部走 trait。
2. tiny-skia 栈**冻结能力开发**:⏳ 项一律不在 CPU 栈上实现(避免为迁移目标重复造轮子);
   只允许修 bug 与加脏区优化。
3. fontdue 停更事实写入风险清单,M2 文本迁移(Parley)优先级上调一档。

### 中期(M2 桌面渲染升级)
1. 主后端 **vello 0.9+**(届时跟进 wgpu 30 的新版):⏳ 高优先级四项(transform、blur/
   毛玻璃、box-shadow、渐变)一步到位;启动管线编译用 wgpu pipeline cache 压平(⚠ 需实测)。
2. 兜底后端从 tiny-skia 切 **vello_cpu**+softbuffer:三档共享 imaging model,截图测试
   基准同步切换;锁 minor、每季度跟升(0.0.x 破坏性变更常态)。
3. 文本走 Parley + Glifo/swash,vello glyph run 直通;fontdue 退役。
4. 静止帧零提交 + "变更帧全量重编码"先行,增量编码优化等上游(vello 无脏区概念,
   小场景下全量编码成本可接受,需基准确认)。

### OHOS 特化(M3 spike 时决策)
1. 首选 **vello_hybrid via wgpu-GLES**;spike 当周核实其 filter/blur 补齐度(2026-07 仍
   落后于 vello_cpu),不达标则 **vello_cpu + GLES 纹理上屏**顶替(中低复杂度 UI 可接受),
   imaging model 同族保证日后无痛升级。
2. vello classic 在 OHOS-GLES 不可用为**硬约束**(需完整 compute shader;GLES 3.1 compute
   经 wgpu-GL 后端不满足 vello 需求);若远期 wgpu-Vulkan-OHOS(VK_OHOS_surface,ash 已支持)
   打通,classic 上 OHOS 才解锁。
3. 应急序列:vello_hybrid → vello_cpu → femtovg(GLES3.0 现成但能力/文本降档)→
   OH_Drawing(系统兜底,像素与桌面不一致,仅保命)。
4. skia-safe 在 OHOS 方向明确出局(无 prebuilt、自维护 GN 工具链不可承受)。

### 触发重评的条件
- vello_hybrid 连续两季度无实质补齐 → OHOS 方案降级为 vello_cpu 常驻;
- vello classic 增量编码/局部渲染上游落地 → 固定开销顾虑消除,权重再升;
- 项目转向富文本编辑器产品化 → skia-safe 作为可选后端重新评估;
- wgpu 在 OHOS 真机驱动矩阵翻车率高(风险清单 #6)→ femtovg/OH_Drawing 候补启动。

---

## 5. 来源(2026-07-18 核实)

- vello / vello_cpu / vello_hybrid 版本与日期:crates.io API(vello 0.9.0=2026-05-15、
  sparse strips 0.0.9=2026-05-30);https://github.com/linebender/vello/releases
  (0.9.0 亮点:wgpu 29、brush_transform、image atlas 驻留;sparse strips 定性:
  vello_cpu "broadly usable"、vello_hybrid "early stages"、filter effects 0.0.7+)
- Linebender 2026 Q1 报告(hybrid beta 定性、矩形快速路径、glyph caching):
  https://linebender.org/blog/tmil-25/ ;2026-07 仓库活动(vello_hybrid filter region
  management 进行中):https://github.com/linebender/vello/pulls
- wgpu 30.0.0(2026-07-01)/29.0.4:crates.io API;OHOS-GLES 合入与 Dioxus 使用:
  https://github.com/DioxusLabs/dioxus/pull/4508
- skia-safe 0.99.0(2026-06-19,m150):crates.io API;https://github.com/rust-skia/rust-skia
- femtovg 0.25.1(2026-05-29)、GLES3.0+/wgpu 双后端、能力与限制(box gradient、无路径
  scissor):crates.io API + https://github.com/femtovg/femtovg ;Slint femtovg 渲染器现状
  与文本质量注记:https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/
- GPUI 停摆与 gpui-ce fork 现状:https://github.com/intendednull/buiy/blob/main/docs/prior-art/gpui/history.md 、
  https://news.ycombinator.com/item?id=47003569
- tiny-skia 0.12.0 / softbuffer 0.4.8 / fontdue 0.9.3(2025-02 后无版本):crates.io API
- OHOS NDK 图形绘制(OH_Drawing,blur/shadow/gradient):
  https://github.com/openharmony/docs (release-notes 与 NDK 概览)
- 本仓库既有结论:docs/DESIGN.md(ADR-3/5)、docs/research/05-rendering-stack.md、
  docs/CSS-SUPPORT.md、crates/sv-shell/src/render.rs
