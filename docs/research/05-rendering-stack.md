# 05 · 四平台统一自绘渲染栈与文本/布局/无障碍/IME 选型

> 调研日期:2026-07-17。目标平台:Windows / Linux / macOS / 鸿蒙(HarmonyOS NEXT & OpenHarmony,下文统称 OHOS)。
> 关键版本号、维护状态均已联网核实(见文末来源);个别未能核实的点在正文中标注"⚠ 仅基于训练数据"。

---

## 0. TL;DR(结论先行)

**推荐栈(自上而下):**

```
┌─────────────────────────────────────────────────────────────┐
│  编译器产物:细粒度更新指令(Svelte 式 runes → 命令式代码)      │
├─────────────────────────────────────────────────────────────┤
│  Widget/场景层(自研):retained 场景树 + 局部脏区更新           │
│    布局: taffy 0.12(flexbox/grid/block,measure 闭包接文本)   │
│    无障碍: AccessKit 0.24(Win/mac/Linux)+ 自研 OHOS adapter   │
│    文本: Parley 0.3 栈(fontique + harfrust + swash/Glifo)     │
├─────────────────────────────────────────────────────────────┤
│  渲染抽象层(自研 trait,参照 Linebender AnyRender/imaging)     │
│    主渲染器: Vello 0.9(wgpu 29 compute)── Win/mac/Linux 桌面  │
│    GLES 路径: vello_hybrid(sparse strips)── OHOS 首选         │
│    纯 CPU 兜底: vello_cpu + softbuffer ── 虚拟机/远程桌面/旧机  │
├─────────────────────────────────────────────────────────────┤
│  窗口/输入层:                                                  │
│    桌面三平台: winit 0.30.x(IME/DPI/theme)+ muda + tray-icon  │
│    OHOS: XComponent + OHNativeWindow(自研薄层或 ohos-rs        │
│          openharmony-ability;winit 上游尚未合并 OHOS 后端)     │
└─────────────────────────────────────────────────────────────┘
```

**三个关键判断:**

1. **不要选 Skia(skia-safe)作主渲染器,也不要绑死 GPUI/Makepad 的渲染层。** Skia 构建成本高、OHOS 交叉编译无官方支持;GPUI 2026 年已明确暂停面向社区的维护;Makepad 无无障碍且 DSL 生态封闭。Vello 家族(经典 compute 版 + sparse strips 的 hybrid/cpu 双形态)是 2026 年中唯一同时覆盖"高端 GPU / GLES-only / 纯 CPU"三档、纯 Rust、且与 Parley/AccessKit 同生态的选项。
2. **文本选 Parley 栈而非 cosmic-text**,决定性理由是生态协同:Parley 0.3 内置 AccessKit 文本属性集成、shaping 已切换到 HarfBuzz 官方组织维护的 HarfRust(对齐 harfbuzz 13.0.0,2026-07 仍活跃提交),且 Bevy 已迁移到 Parley,长期维护信号强。cosmic-text 依然优秀(0.19.0,2026-04),可作为备选,但其与 AccessKit/vello 的集成要自己缝合。
3. **鸿蒙是全栈风险集中地**:渲染(wgpu 上游 OHOS 后端目前走 GLES,Vulkan 面需自己接 `VK_OHOS_surface`)、窗口(winit 无上游后端)、无障碍(AccessKit 无 OHOS adapter,需基于 `OH_ArkUI_AccessibilityProvider` 自研)、IME(需直接对接 IMF native C API)。建议架构上把"平台壳"做成独立 crate,桌面三平台先行,OHOS 作为第二梯队并尽早做穿透式原型(spike)验证。

---

## 1. 调研方法与核实说明

- 已联网核实:Vello/vello_cpu/vello_hybrid 版本与状态、HarfRust 现状、Parley/fontique、cosmic-text、taffy、AccessKit、winit IME、skia-safe、GPUI、Makepad、Rust OHOS target tier、wgpu OHOS 支持、OHOS Vulkan/XComponent/无障碍/IME native API、muda/tray-icon。
- ⚠ 仅基于训练数据(未逐条联网核实,置信度中等):tiny-skia/softbuffer 的 API 细节、winit 各平台 theme/深色模式行为细节、OHOS `/system/fonts` 具体文件名、GPUI 各平台后端(Metal/blade/DirectX)细节。
- 检索到的 Vello 发布日期在不同来源间有年份歧义(GitHub 页面摘要与 crates.io API),以 **crates.io API 数据为准**:vello 0.9.0 = 2026-05-15。

---

## 2. 2D 渲染引擎对比

### 2.1 Vello 家族(Linebender)——推荐

联网核实的现状(2026-07):

| 组件 | 版本 | 状态 | 说明 |
|---|---|---|---|
| vello(经典,wgpu compute) | **0.9.0**(2026-05-15,依赖 **wgpu 29**;0.8.0 = 2026-03-20) | 可用,持续快速迭代 | GPU compute-centric,性能最强,但要求较完整的 compute shader 支持 |
| vello_cpu(sparse strips) | 0.0.9(2026-05-30) | **alpha** | 纯 CPU 渲染器,面向无 GPU/弱 GPU 设备;已支持 non-isolated blending、clip/mask、实验性 image filters;基于 fearless_simd(0.4,AVX2)做跨平台 SIMD,官方称"competitive performance" |
| vello_hybrid(sparse strips) | 0.0.9 | **约 beta 质量** | CPU 做粗光栅化预处理 + GPU 做最终光栅化,显式面向 WebGL2/GLES 级别的 GPU;特性尚未与 vello_cpu 完全对齐 |
| Glifo | 0.1.1(2026-05-30) | 实验性 | 新拆出的字形渲染 crate:outline 提取、彩色 emoji、hinting,供各 Vello 变体共用 |

Linebender 2026 Q1 报告还有两个对我们架构极有参考价值的信号:

- **统一 API 尝试受挫后,官方转向两层抽象**:`AnyRender`(易用优先)与 `imaging`(性能优先),允许在多个渲染器间切换;Masonry 已迁移到 `imaging`,实现"GPU-optional"渲染。这正是我们需要的"主渲染器 + 兜底"的官方版本——**我们的渲染抽象层应当对齐或直接复用这套 trait**,而不是自己发明第三套。
- 性能优化持续落地:glyph caching 初版、矩形快速路径、gradient 修复、opaque full-tile image 优化等,说明 sparse strips 路线是当前投入重心。

**风险**:vello_cpu/vello_hybrid 无 API 稳定性承诺(pre-release);经典 vello 与 sparse strips 两条线的 imaging model 相近但 API 不同,短期内需要靠抽象层隔离。

### 2.2 Skia(skia-safe)——不推荐作主渲染器

- 联网核实:skia-safe **0.99.0**,跟踪 Skia **chrome/m150**,rust-skia 项目维护正常(2026-05 仍在更新 milestone 流程)。
- 能力无可挑剔(路径、文本、滤镜、GPU 后端 gl/vulkan/metal/d3d),但代价:
  - **构建成本高**:C++ 大件,冷构建数十分钟级;prebuilt 二进制仅覆盖官方 host 平台组合,**没有 `*-linux-ohos` 目标的官方 prebuilt**,OHOS 交叉编译需自己维护 Skia 的 GN 工具链配置(Flutter OHOS 移植证明 Skia 能跑在 OHOS 上,但那是华为/社区维护的引擎级工程量)。
  - 二进制体积大(数十 MB),与"Svelte 式极小运行时"的定位相悖。
- **保留价值**:若后期需要像素级对标浏览器渲染(如富文本编辑器产品化),可把 Skia 做成渲染抽象层的一个可选后端,而不是地基。

### 2.3 tiny-skia + softbuffer——兜底方案的备选,已被 vello_cpu 部分取代

- tiny-skia:纯 Rust CPU 光栅化,质量高但只有低层 API(无场景图、文本要自己拼);softbuffer:纯 CPU 帧缓冲展示,winit 生态标配。⚠ 细节仅基于训练数据。
- 2026 年的新形势:**vello_cpu 就是 Linebender 官方给出的"tiny-skia 继任者"**,imaging model 与 GPU 版一致——用它做兜底可保证三档渲染路径视觉一致。建议:兜底路径 = vello_cpu 渲染 → softbuffer 上屏;tiny-skia 仅在 vello_cpu alpha 期出现阻断性 bug 时作为应急替换。

### 2.4 Makepad 自研渲染 / GPUI 渲染层——仅作参考,不建议依赖

- **Makepad**:1.0 于 2025-05 发布,shader-DSL 驱动的自绘渲染(metal/dx11/opengl/webGL)。硬伤:**无无障碍支持**、live DSL 文档薄弱、版本发布不打 git tag,生态封闭。作为"如何用 GPU 画 UI"的参考实现有价值,作为依赖不可取。
- **GPUI**(Zed):已发布到 crates.io(月下载 ~6 万),但 **2026 年 Zed 官方明确"community-facing GPUI work paused"**,只为 Zed 自身服务;平台后端(macOS Metal / Linux blade / Windows DirectX,⚠ 后端细节仅基于训练数据)不含任何 OHOS 路径。作为架构参考(hybrid immediate/retained、taffy fork 做布局)可读,不可作依赖。

### 2.5 鸿蒙的 GPU API 可用性(决定 OHOS 渲染路径)

联网核实:

- **Vulkan**:HarmonyOS 4.0(API 10)起提供 native Vulkan,OHOS 有专属 surface 扩展 **`VK_OHOS_surface`**;Rust 侧 **ash 已于 2025-11 合并 `VK_OHOS_surface` 支持**。Flutter OHOS 移植的 Impeller 后端即为:XComponent → `OHNativeWindow` → `VK_OHOS_surface` → VkSurface/Swapchain,证明该路径生产可用。
- **GLES/EGL**:XComponent + EGL + GLES 3.x 是 OHOS NDK 的标准自绘路径,最成熟。
- **wgpu**:上游已合并 OHOS 支持,但**当前走 GLES 后端**(社区 PR,Dioxus 4508 等已在用);Vulkan 后端理论上随 ash 的 `VK_OHOS_surface` 可通,但 wgpu 上游是否已接通 OHOS Vulkan surface 创建**未能确认**,需原型验证。
- Rust target:`aarch64/armv7/x86_64-unknown-linux-ohos` 均为 **Tier 2**,rustup 直接安装,工具链风险低。

**OHOS 渲染结论**:首选 **vello_hybrid(为 GLES 级 GPU 设计)走 wgpu-GLES**;若 wgpu-vulkan-OHOS 打通则经典 vello 也可上;vello_cpu 兜底始终可用(OHOS target 就是 Linux-like + LLVM,纯 CPU 代码无平台风险)。

### 2.6 渲染选型总结

| 场景 | 渲染器 |
|---|---|
| Win/mac/Linux,正常 GPU | vello 0.9(wgpu 29:D3D12/Metal/Vulkan) |
| OHOS(GPU) | vello_hybrid via wgpu-GLES(近期)→ vello/Vulkan(远期) |
| 无 GPU / 驱动黑名单 / CI 截图测试 | vello_cpu + softbuffer |

抽象层设计:自研 `trait Renderer`(或直接采用 Linebender `AnyRender`/`imaging`),场景层只产出与渲染器无关的 display list;**编译器生成的细粒度更新代码落在场景层,不感知渲染器**——这保证兜底切换零成本,也隔离 sparse strips 的 API 不稳定期。

---

## 3. 文本栈:Parley 系 vs cosmic-text

### 3.1 两大候选现状(联网核实)

**Parley 栈(Linebender)——推荐**

- parley **0.3.0**:rich text layout,**shaping 已切换到 HarfRust**;0.3 亮点:AccessKit 集成(文本属性直通无障碍树)、Cursor 重设计、PlainEditor 大改(现成的单行/多行编辑器逻辑,含 IME preedit 处理)。近期新增 macOS 系统字体枚举、按路径加载字体、CSS text-indent。**Bevy 已切换到 Parley**,另有 CuTTY、Gosub 等采用——维护动能强。
- **HarfRust**:HarfBuzz 官方组织(harfbuzz/harfrust)维护的 Rust port,**对齐 HarfBuzz v13.0.0**,最近提交 2026-07-14;性能差距 <25%;已被上游 HarfBuzz 作为可选 shaper 用于交叉验证。基于 read-fonts(与 skrifa 同源解析),无 unsafe。注意:**不含平台集成**(无 CoreText/DirectWrite/graphite2),复杂脚本主流(阿拉伯文、天城文等)覆盖,但 graphite 字体(极少数)不支持。
- **fontique**(parley 仓库内):字体枚举 + fallback。Linux 后端已重写为调用系统 fontconfig 库(而非自己解析配置),macOS 走 CoreText,Windows 走 DirectWrite 枚举(⚠ Windows 细节仅基于训练数据);mmap 懒加载。
- **swash / Glifo**:字形光栅化。swash 0.2.x 仍维护;Linebender 正把字形渲染整合进 Glifo(outline + 彩色 emoji + hinting),与 Vello 缝合最紧。

**cosmic-text(System76)——合格备选**

- **0.19.0**(2026-04-22):变量字体 wght 匹配、大文档 shape_until_scroll 优化等;支撑 COSMIC 桌面(Pop!_OS 24.04 LTS 已于 2025-12 以 COSMIC 为默认桌面发货)——**生产验证充分**。
- 一体化(fontdb + rustybuzz + swash),上手快;但 shaping 用 rustybuzz(社区维护,HarfBuzz 官方重心已转向 harfrust)、无 AccessKit 文本集成、rich text 能力弱于 parley。

**判断**:选 **Parley 栈**。理由按权重:① AccessKit/vello/taffy 同生态,缝合成本最低;② HarfRust 由 HarfBuzz 官方维护,是 Rust shaping 的长期正统;③ PlainEditor 提供了文本编辑(含 IME)的参考实现。风险:parley 0.x API 仍会破坏性变更——用内部 `TextEngine` 门面隔离。

### 3.2 中文 fallback / emoji / BiDi

- **中文 fallback**:fontique 的 fallback 链在桌面三平台走系统机制(fontconfig / CoreText / DirectWrite),简体/繁体/日文汉字消歧需锁定 locale(zh-Hans/zh-Hant/ja)传入 fallback 查询,否则会出现"日式汉字"问题——需要在引擎层默认注入应用 locale。
- **emoji**:彩色 emoji 需支持 COLRv0/v1(Windows Segoe UI Emoji、OHOS HMOS Color Emoji)与 sbix/CBDT(Apple/Noto 位图);Glifo 目标即包含彩色字形;swash 支持 COLR/位图(⚠ 覆盖度细节仅基于训练数据)。建议 v1 支持 COLR + 位图 emoji,CBDT 优先级可放低。
- **BiDi**:parley 内置 unicode-bidi 段落级 BiDi + 双向光标逻辑;这是 cosmic-text 与 parley 都过关、但自研栈几乎必翻车的点——不要自己写。

### 3.3 各平台系统字体加载

| 平台 | 机制 | 我们要做的 |
|---|---|---|
| Windows | DirectWrite 枚举(fontique) | 基本免费 |
| macOS | CoreText(fontique 近期补齐系统字体枚举) | 基本免费 |
| Linux | 系统 fontconfig 库(fontique 新后端) | 基本免费;注意 Flatpak 沙箱字体路径 |
| OHOS | **fontique 无后端** | 自写:扫描 `/system/fonts`(HarmonyOS Sans 为默认系统字体,变量字体,覆盖简繁中文/拉丁/西里尔/希腊/阿拉伯 105 种语言;另有 Noto 系补充与 HMOS 彩色 emoji,⚠ 具体文件清单仅基于训练数据),构造静态 fallback 链注入 fontique 的 custom collection;OHOS 亦有 `@ohos.font` / native drawing 的字体管理接口可查询配置 |

OHOS 字体后端是**小而确定**的工作量(枚举目录 + 解析 name 表 + 写死 fallback 顺序),建议直接列入排期而非等上游。

---

## 4. 布局:taffy 够用吗?

- 联网核实:taffy **0.12.2(2026-07-15)**,2026 年内 0.10→0.11→0.12 三个 minor,维护非常活跃(Nico Burns);实现 **Block + Flexbox + CSS Grid**,对齐 CSS 规范;Dioxus/Bevy 采用,Zed 维护自己的 fork。
- **判断:够用**。桌面 UI 的布局诉求(面板、栅格、滚动区)flexbox+grid 全覆盖;缺的是 CSS float/table/inline 流式布局——本项目不需要。
- **文本测量集成模式**(taffy 标准做法):文本节点作为 leaf node,挂 `measure` 闭包;闭包内调 parley 以 `AvailableSpace` 为宽度约束做 layout,返回尺寸;**必须缓存**(key = 文本内容 + 样式 + 约束宽度桶),否则 flexbox 的多轮测量会导致重复 shaping。Svelte 式细粒度更新在这里的红利:编译期已知哪个绑定影响哪个文本节点,可精准失效测量缓存 + 标记该子树 relayout,不需要全树 diff。
- 注意 taffy 0.x 也有破坏性变更节奏(2026 年 3 个 minor),同样用薄门面隔离。

---

## 5. 无障碍:AccessKit 与鸿蒙缺口

### 5.1 AccessKit 现状(联网核实)

- accesskit **0.24.1**(2026-06-12)。平台 adapter:**Windows(UI Automation)、macOS(NSAccessibility)、Unix(AT-SPI over D-Bus)、Android(Java accessibility API)**;官方称各 adapter 已"大致功能对齐",足以让非平凡应用可访问,但未覆盖全部控件类型/属性。iOS 与 web(canvas)在路线图上。
- 与 winit(`accesskit_winit`)、parley(0.3 起文本属性)、egui 等集成成熟。**结论:桌面三平台无障碍直接采用 AccessKit,没有竞争方案。**

### 5.2 鸿蒙没有 AccessKit 后端怎么办

- 联网核实:AccessKit **无 OHOS adapter,也未见路线图提及**。
- 但 OHOS NDK 提供了自绘框架接入无障碍的正规通道:**从 `OH_NativeXComponent` 获取 `ArkUI_AccessibilityProvider`,注册一组 C 回调**(元素树查询、按 id 找节点、动作执行、焦点移动等),由系统无障碍服务(ScreenReader)拉取虚拟节点树——语义上与 AccessKit 的 tree/update/action 模型同构(Flutter/Qt 的 OHOS 移植都走这条路)。
- **建议方案**:自研 `accesskit-ohos` 桥接 crate——把 AccessKit 的 `TreeUpdate`/`Node` schema 映射到 `ArkUI_AccessibilityProvider` 回调(role→ArkUI accessibility 类型,action 双向翻译)。因为我们的 widget 层只产出 AccessKit 语义树,这层桥是**纯适配器**,估计 2~4 人周出可用原型;做好后可考虑回馈 AccessKit 上游(其 schema 明确欢迎新平台 adapter)。
- 降级策略:OHOS v1 允许"无障碍树仅只读朗读、不支持复杂 action",符合先上架后完善的节奏;但**架构上必须从第一天保留语义树输出**,否则后补成本巨大(Makepad 之鉴)。

---

## 6. IME / 输入法

### 6.1 桌面(winit)

- 联网核实:winit 0.30.x(最新 0.30.13)提供跨平台 IME API:`Ime::{Enabled, Preedit, Commit, Disabled}` 事件、`Window::set_ime_allowed`、**`Window::set_ime_cursor_area`(候选窗定位)**;preedit 期间不派发 KeyboardInput。近期修复:X11 的 hotspot 取 ime cursor area 右下角、Windows 的 Preedit 光标偏移计算。
- 工程要点:
  - **候选窗定位责任在应用**:每次光标/滚动变化后用文本光标的窗口坐标调 `set_ime_cursor_area`,否则中文输入候选窗漂移(Rust 生态 GUI 的高频 bug,iced 2025 年才修好初始位置)。
  - preedit 渲染(下划线、选中段)要在文本引擎里做:parley `PlainEditor` 已含 preedit 概念,直接复用其状态机。
  - Wayland 走 text-input-v3,zwp 协议下 preedit 样式信息较少;X11 XIM 質量参差——Linux 上以"能用"为验收标准,别追求与 Windows 完全一致。
  - winit 0.31 的 event 模型重构临近(⚠ 时间点仅基于训练数据),窗口层也要门面隔离。

### 6.2 鸿蒙 IME

- 联网核实:OHOS IMF(输入法框架)对自绘/native 应用提供两条路:
  1. **ArkTS 侧 `InputMethodController.attach()`** 绑定自定义编辑框,监听 insertText/deleteLeft 等回调——可在 ArkTS 壳里做,再经 NAPI 转发进 Rust;
  2. **NDK C API**:`OH_InputMethodController_Attach` + **`OH_TextEditorProxy`**(一组 C 函数指针:InsertText、DeleteBackward、光标区域上报等)——Servo 的 OHOS 移植即用此路径,证明纯 native 可行。
- 建议直接走 NDK C API(与我们的 Rust 主体同层),经 ohos-rs 生成绑定;候选窗定位对应 `OH_TextConfig` 中的光标区域上报(⚠ 具体字段名仅基于训练数据,需对 SDK 头文件确认)。
- 注意:OHOS 上"IME 常驻软键盘"的交互(升起时窗口 resize/avoid area)需要处理 `keyboardHeightChange` 类事件,桌面无此概念——窗口抽象层要给"内容安全区"留接口。

---

## 7. 窗口系统集成要点(每平台)

| 关注点 | Windows | macOS | Linux (Wayland/X11) | OHOS |
|---|---|---|---|---|
| 窗口库 | winit | winit | winit | XComponent + OHNativeWindow(winit 上游无后端;ohos-rs 的 openharmony-ability 有预览级 winit 适配,issue #4081 仍开放) |
| DPI | Per-Monitor v2,ScaleFactorChanged | Retina backing scale | Wayland fractional-scale-v1;X11 需自算 | vp/density 体系,由 XComponent 报告 densityPixels |
| 多窗口 | 完整 | 完整 | Wayland 无全局坐标,弹窗用 xdg_popup | 受限:手机单窗口;平板/2in1 有自由窗口;桌面形态多窗口 API 尚在演进——**架构假设"1 Ability = 1 根窗口"最稳** |
| 菜单 | muda(Win32 菜单) | muda(NSMenu,全局菜单栏必须做) | muda(gtk);Wayland 无全局菜单标准 | 无桌面式菜单概念,用应用内组件替代 |
| 系统托盘 | tray-icon | tray-icon(NSStatusItem) | tray-icon(StatusNotifier/appindicator,依赖 libappindicator;GNOME 需扩展) | 无托盘概念,不做 |
| 深色模式 | winit `Window::theme()` + ThemeChanged(⚠ 细节基于训练数据) | 同左 | 依赖 xdg-desktop-portal settings | ArkTS ConfigurationCallback(colorMode)转发进 Rust |
| 生命周期 | 常规 | 常规 | 常规 | **Ability 生命周期**(onForeground/onBackground/低内存回收),必须与响应式系统的 effect 调度打通(后台停帧) |

muda/tray-icon(tauri-apps,联网核实)覆盖桌面三平台,Linux 依赖 gtk + libappindicator,是当前 Rust 生态唯一维护良好的组合。

**OHOS 壳的形态建议**:ArkTS 最小壳(Ability + XComponent + 生命周期/colorMode/键盘高度转发)+ Rust 主体(渲染、事件循环)。不要试图把 winit 语义硬套 OHOS——自定义 `PlatformWindow` trait,桌面实现包 winit,OHOS 实现直接包 XComponent 回调。

---

## 8. 风险清单(按严重度排序)

| # | 风险 | 等级 | 缓解 |
|---|---|---|---|
| 1 | **OHOS 平台壳整体成熟度**:winit 无上游后端、wgpu 仅 GLES、AccessKit/IME 全需自研桥接;且 HarmonyOS NEXT(商用)与 OpenHarmony(开源)的 SDK/API 存在版本错位 | 高 | 桌面先行;OHOS 做 2~3 周穿透式 spike(XComponent→wgpu-GLES→vello_hybrid→上屏 + IME attach)验证全链路;锁定 API 版本(建议 API 12+) |
| 2 | **sparse strips(vello_cpu/hybrid)API 不稳定**(0.0.x,无稳定性承诺) | 高 | 渲染抽象层隔离;跟随 Linebender AnyRender/imaging 而非自造;锁 minor 版本、每季度跟进 |
| 3 | **文本编辑 + IME 的长尾**(候选窗定位、preedit 渲染、Wayland text-input 差异、OHOS 软键盘避让) | 中高 | 复用 parley PlainEditor;IME 行为做成平台集成测试矩阵;早期就在中文/日文输入下自测 |
| 4 | parley/taffy/winit 均 0.x,破坏性变更叠加 | 中 | 每个外部依赖一层薄门面(TextEngine/LayoutEngine/PlatformWindow);升级集中在门面 crate |
| 5 | AccessKit-OHOS 桥无先例,ScreenReader 实测行为未知 | 中 | 原型期即用真机 + 华为屏幕朗读验证;先覆盖只读朗读与焦点导航 |
| 6 | wgpu-GLES 在 OHOS 真机驱动上的兼容性(各 SoC GPU 驱动质量参差) | 中 | vello_cpu 兜底常备;建立真机测试池(麒麟/骁龙各一) |
| 7 | 中文 fallback/emoji 细节(locale 消歧、COLRv1 覆盖) | 低中 | 引擎层强制注入 locale;emoji 用 COLR+位图双路径 |
| 8 | 团队被"顺便支持移动端/Web"诱惑导致范围失控 | 低 | 明确 v1 = 四平台桌面形态;Web/移动只保留架构可能性 |

---

## 9. 落地建议(排期视角)

1. **第 1 里程碑(桌面纵切)**:winit + wgpu + vello 0.9 + parley 0.3 + taffy 0.12 + AccessKit,跑通"编译器输出 → 场景树 → 细粒度更新 → 渲染"最小闭环;同时接 `set_ime_cursor_area` 与中文输入。
2. **并行 spike(OHOS 穿透)**:ohos target(Tier 2)+ XComponent + wgpu-GLES + vello_hybrid 上屏 + `OH_InputMethodController_Attach` 输入;产出 go/no-go 报告。
3. **第 2 里程碑**:vello_cpu+softbuffer 兜底路径接入渲染抽象层;fontique OHOS 字体后端;accesskit-ohos 桥原型。
4. 所有外部 0.x 依赖锁版本 + 门面隔离,每季度统一升级一次。

---

## 10. 来源

- Vello releases / 状态:https://github.com/linebender/vello/releases 、https://crates.io/api/v1/crates/vello 、vello_cpu README:https://github.com/linebender/vello/tree/main/sparse_strips/vello_cpu
- Linebender 2026 Q1(Vello 0.8/sparse strips 0.0.7、AnyRender/imaging、Glifo、Parley 动态、Masonry IME):https://linebender.org/blog/tmil-25/
- HarfRust(HarfBuzz 官方 Rust port,对齐 13.0.0):https://github.com/harfbuzz/harfrust 、CHANGELOG:https://github.com/harfbuzz/harfrust/blob/main/CHANGELOG.md
- Parley 0.3.0(HarfRust 后端、AccessKit 集成、PlainEditor):https://github.com/linebender/parley/releases 、https://docs.rs/parley/latest/parley/
- fontique(枚举/fallback、fontconfig 重写):https://github.com/linebender/parley/tree/main/fontique 、https://docs.rs/fontique/latest/fontique/
- cosmic-text 0.19.0:https://github.com/pop-os/cosmic-text/releases ;COSMIC/Pop!_OS 24.04:https://system76.com/blog/post/cosmic-1-0-8-released
- swash:https://github.com/dfrg/swash
- taffy 0.12.2:https://crates.io/api/v1/crates/taffy 、https://github.com/dioxusLabs/taffy
- AccessKit 0.24.1 与平台 adapter:https://accesskit.dev/ 、https://github.com/AccessKit/accesskit 、https://crates.io/api/v1/crates/accesskit
- winit IME API 与修复:https://docs.rs/winit/latest/winit/event/enum.Ime.html 、https://github.com/rust-windowing/winit/releases ;iced 候选窗定位修复:https://github.com/iced-rs/iced/pull/2793
- winit OHOS 讨论与 ohos-rs 适配:https://github.com/rust-windowing/winit/issues/4081 、https://ohos.rs/blog/2025-01-24
- Rust OHOS Tier 2 target:https://doc.rust-lang.org/rustc/platform-support/openharmony.html 、https://github.com/rust-lang/compiler-team/issues/719
- wgpu OHOS(GLES)与 Dioxus OHOS:https://github.com/DioxusLabs/dioxus/pull/4508 、https://github.com/gfx-rs/wgpu
- OHOS Vulkan / VK_OHOS_surface / Flutter Impeller OHOS:https://dev.to/shaohushuo/harmonyos-flutter-practice-15-flutter-engine-impeller-harmonyization-performance-optimization-and-jo1 、ash PR 列表:https://github.com/ash-rs/ash/pulls 、https://en.wikipedia.org/wiki/Vulkan
- XComponent native 指南与 Rust 绑定:https://developer.huawei.com/consumer/en/doc/harmonyos-guides/napi-xcomponent-guidelines 、https://github.com/jschwe/xcomponent
- OHOS native 无障碍(ArkUI_AccessibilityProvider):https://docs.openharmony.cn/pages/v5.0/en/application-dev/ui/napi-xcomponent-guidelines.md 、https://github.com/openharmony/arkui_ace_engine 、OpenHarmony 6.0 release notes:https://github.com/openharmony/docs/blob/master/en/release-notes/OpenHarmony-v6.0-release.md
- OHOS IME(InputMethodController.attach / OH_TextEditorProxy;Servo 用例):https://www.cnblogs.com/samex/p/18515142 、https://www.slideshare.net/slideshow/gosim-2024-porting-servo-to-openharmony/273076796 、https://github.com/openharmony/miscservices_inputmethod
- HarmonyOS Sans:https://github.com/huawei-fonts/HarmonyOS-Sans 、https://github.com/openharmony/resources
- skia-safe 0.99 / m150:https://github.com/rust-skia/rust-skia 、https://docs.rs/crate/skia-safe/latest
- GPUI 状态(crates.io 发布、社区维护暂停):https://github.com/zed-industries/zed/tree/main/crates/gpui 、https://github.com/intendednull/buiy/blob/main/docs/prior-art/gpui/history.md
- Makepad 1.0 与局限:https://github.com/makepad/makepad 、https://news.ycombinator.com/item?id=43971829 、https://www.boringcactus.com/2025/04/13/2025-survey-of-rust-gui-libraries.html
- muda/tray-icon:https://github.com/tauri-apps/tray-icon
