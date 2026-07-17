# 调研报告 03:鸿蒙(HarmonyOS NEXT / OpenHarmony)上跑 Rust 自绘 UI 的可行性与集成架构

- 调研日期:2026-07-17(关键事实已联网核实;个别标注"仅训练数据/二手来源"的除外)
- 项目上下文:svelte-rs —— Svelte 风格、编译期细粒度响应式的 Rust 跨平台桌面 UI 库,目标平台含 HarmonyOS NEXT / OpenHarmony
- 结论先行:**技术上完全可行,且 2025–2026 年生态成熟速度超预期**。Rust 官方 target 是 Tier 2 with host tools;XComponent → OHNativeWindow → EGL/GLES(现在)或 Vulkan(VK_OHOS_surface,ash 已合入)渲染路径已被 Servo、Flutter-OHOS、wgpu 等多个项目走通;winit **上游没有** ohos backend,需依赖社区层(openharmony-ability)或自建窗口抽象——对本项目而言自建反而是更稳的选择。政策上自绘引擎不被禁止(Flutter/RN 官方生态合作是最强先例),主要风险在性能基线审核、IME/无障碍等"原生体验"细节。

---

## 1. 术语与平台边界

先划清两个名字,后文所有结论都要区分:

| | OpenHarmony | HarmonyOS NEXT |
|---|---|---|
| 归属 | 开放原子开源基金会,开源 | 华为商业发行版("纯血鸿蒙") |
| SDK | OpenHarmony Public SDK(Gitee 下载) | HarmonyOS SDK(随 DevEco Studio / Command Line Tools) |
| 签名 | 自签/板卡宽松(`"default"` 型签名) | 必须 AGC(AppGallery Connect)证书链(`"hos"` 型签名) |
| 分发 | 自装 hap(hdc) | 华为应用市场审核上架 |

Native 层(NDK、XComponent、EGL、NAPI)两者高度一致,Rust 侧代码基本可以共用;差异集中在签名、上架、部分闭源 Kit。Servo 的构建系统就是同一套代码用 `--flavor=harmonyos` 切换([Servo Book: OpenHarmony](https://book.servo.org/building/openharmony.html))。

---

## 2. Rust 官方 target 现状(已核实,rustc book)

来源:[rustc book — *-linux-ohos](https://doc.rust-lang.org/rustc/platform-support/openharmony.html)

- `aarch64-unknown-linux-ohos`、`armv7-unknown-linux-ohos`、`x86_64-unknown-linux-ohos`:**Tier 2 with host tools**——rustup 直接分发 std 与工具链,`rustup target add aarch64-unknown-linux-ohos` 即可;cargo/rustfmt/clippy 全套可用(cargo 在 ohos 上自举需 [ohos-openssl](https://github.com/ohos-rs/ohos-openssl) 预编译库)。`loongarch64-unknown-linux-ohos` 为 Tier 3。miri 不可用(libffi、tikv-jemalloc-sys 不支持 ohos)。
- libc 为 OHOS 自己的 musl 变体,编译时需 `-D__MUSL__`。
- 工具链配置:OHOS SDK 自带 clang/llvm,但**不直接认识 Rust 的 target 名**,需要写 wrapper 脚本:

```sh
#!/bin/sh
exec /path/to/ohos-sdk/linux/native/llvm/bin/clang \
  -target aarch64-linux-ohos \
  --sysroot=/path/to/ohos-sdk/linux/native/sysroot \
  -D__MUSL__ "$@"
```

再在 `.cargo/config.toml` 里给三个 target 分别指定 `linker`(上述脚本)与 `ar`(SDK 的 llvm-ar)。**实践中不必手工做这些**:ohos-rs 的 `ohrs` CLI(v1.4.2,2026-06 更新)封装了 `ohrs build --arch arm64`,自动处理 wrapper/sysroot;Servo 的 mach 也是自动生成。

判断:工具链层面**零风险**。Tier 2 with host tools 意味着 Rust 官方 CI 每次发布都构建这些 target,不会悄悄烂掉。目标设备侧只需关注 aarch64(NEXT 真机全是 arm64)+ x86_64(模拟器),armv7 可以不管。

---

## 3. 绑定层与 ArkTS ↔ Rust 互操作

鸿蒙应用的进程入口**必须**是 ArkTS(UIAbility),这一点和 Android 的 Java Activity 类似但更严格——没有纯 native 入口(NativeActivity 等价物不存在)。所以任何 Rust 方案都是"ArkTS 壳 + Rust cdylib(.so)经 NAPI 加载"的结构。

生态现状(2026-07 核实,均活跃):

- **[ohos-rs](https://github.com/ohos-rs/ohos-rs)**(napi-rs 的 fork,229 stars,2026-07 仍在更新):`napi-ohos` v1.2.0(2026-05-12,累计下载 26 万+),用 `#[napi]` 宏导出 Rust 函数给 ArkTS,体验与 napi-rs 一致。配套 `ohrs` CLI 负责交叉编译与产物摆放。这是 ArkTS↔Rust 桥的事实标准。
- **[ohos-sys](https://crates.io/crates/ohos-sys)** v0.9.0(2026-05-26,[openharmony-rs](https://github.com/openharmony-rs) 组织,维护者 jschwe 即 Servo OHOS 移植的华为工程师):NDK 原始 FFI 绑定,按 feature 对应各 OpenHarmony API 模块。
- **[ohos-native-bindings](https://github.com/ohos-rs/ohos-native-bindings)**(2026-07 更新):高层安全绑定,拆成 `ohos-xcomponent-binding`(v0.3.1)、`ohos-vsync-binding`、`ohos-ime-binding`(均 2026-07-14 更新)等 crate——正好覆盖自绘 UI 需要的三大件。
- **[uniffi-bindgen-arkts](https://github.com/ohos-rs)**(2026-07 更新):若业务层想用 UniFFI 统一生成多语言绑定,ArkTS 也有后端了。
- 互操作性能注意:NAPI 走 JS 引擎(ArkTS 运行时),高频调用有开销。RN-OHOS 的经验是绕开 NAPI、用 C-API 直连 ArkUI 后端提效([鸿蒙版 RN 架构浅析](https://zhuanlan.zhihu.com/p/719918885))。对我们这种自绘方案,**渲染热路径完全在 native 侧,NAPI 只承担生命周期/低频系统调用转发**,开销无关紧要——这是自绘架构在鸿蒙上的一个天然优势。

---

## 4. 渲染路径:XComponent → OHNativeWindow → GPU

核心链路(多来源交叉验证,含华为官方文档与 Flutter/Servo 实现):

1. ArkTS 页面放一个 `XComponent`(type = `surface`,`libraryname` 指向 Rust 编译出的 .so);
2. Native 侧通过 `OH_NativeXComponent_RegisterCallback` 注册 surface created/changed/destroyed 与触摸/鼠标/按键回调;`OnSurfaceCreated` 回调参数直接就是 `OHNativeWindow*`(另一条路:ArkTS 拿 SurfaceId 传下来,native 用 `OH_NativeWindow_CreateNativeWindowFromSurfaceId` 换取);
3. GPU API 接入:
   - **EGL/GLES3**:`OHNativeWindow*` 直接作为 `EGLNativeWindowType` 创建 EGLSurface;链接 `libEGL.so`、`libGLESv3.so`、`libace_ndk.z.so`、`libace_napi.z.so`。这是当前最成熟路径(华为官方 XComponent 文档示例即此)。
   - **Vulkan**:Khronos 已收录华为提交的 **`VK_OHOS_surface`** 扩展(`vkCreateSurfaceOHOS` 接 `OHNativeWindow`)。Rust 侧 [ash PR #1016](https://github.com/ash-rs/ash/pull/1016) 已于 **2026-05-05 合入**(ash-window 同步支持 ohos handle),排期在 ash 0.39。Flutter-OHOS 的 Impeller 后端就是走 Vulkan + VK_OHOS_surface([Impeller 鸿蒙化](https://blog.csdn.net/zackslee/article/details/144914441))。
4. **wgpu**:[PR #7085](https://github.com/gfx-rs/wgpu/pull/7085)(ohos-rs 维护者 richerfu 提交)**2025-02-13 合入 GLES/EGL 后端的 OHOS 支持**,并把 ohos 加入 wgpu CI 的 tier-3 build-only 矩阵。Vulkan 后端要等 wgpu 升级到含 VK_OHOS_surface 的 ash(0.39)——已在路上但**截至调研日尚未确认发布**。个别 example(shadow/water)在合入时有 crash 记录,成熟度打八折。
5. **raw-window-handle 0.6** 已内置 `OhosNdkWindowHandle` / `OhosDisplayHandle`,意味着整个 rust-windowing 生态的握手协议已认识鸿蒙窗口。
6. **vsync**:NDK 提供 `OH_NativeVSync_Create` / `OH_NativeVSync_RequestFrame`(链接 `libnative_vsync`),回调发生在 vsync 实例自己的 EventRunner 线程而非请求线程——帧循环要做好线程切换设计([华为官方 NativeVsync 文档](https://developer.huawei.com/consumer/cn/doc/harmonyos-guides/native-vsync-guidelines))。Rust 侧有现成 `ohos-vsync-binding`。

判断:**GLES 路径今天就能用且被 wgpu 官方 CI 保护;Vulkan 路径 2026 下半年内会在 Rust 生态完全打通**。对 svelte-rs 的渲染后端,建议第一阶段锁 wgpu GLES(或裸 EGL+GLES),把 Vulkan 当免费升级。注意若渲染器想用 vello(依赖 compute shader),GLES 后端的 compute 支持受限,需实测;稳妥起见 demo 阶段用传统光栅化(femtovg / skia-safe / 自研)。

---

## 5. 窗口抽象:winit 有没有 ohos backend?(重点核实)

**结论:上游 winit 没有 OHOS backend。**

- [winit issue #4081](https://github.com/rust-windowing/winit/issues/4081)(2025-01 提出"能否支持 OpenHarmony")截至调研日**仍 open**;winit 当前 0.31.0-beta.2 的 README/平台列表无任何 ohos 字样(已直接核实 master README)。
- 社区替代品(三条线,都不是官方):
  1. **[harmony-contrib/openharmony-ability](https://github.com/harmony-contrib/openharmony-ability)**(2026-07-14 仍在 push):定位相当于 Android 生态的 `android-activity`——提供 ArkTS 壳模板 + `#[ability]` 宏 + `app.run_loop(|event| …)`,把 UIAbility 生命周期、输入事件(含 `InputEvent::TextInputEvent`)转发进 Rust。ohos-rs 官方博客([2025-01-24](https://ohos.rs/blog/2025-01-24))宣布了基于它的 **winit fork 适配**(维护者 richerfu,另有 winit+softbuffer 渲染示例 [richerfu/winit-softbuffer](https://github.com/richerfu/winit-softbuffer))。README 自述 "in progress / API unstable"。
  2. **tgui-winit-ohos** v0.4.11(2026-04 发布,个人作者,下载量 260):面向 winit 0.31 模块化架构(winit-core)的 OHOS backend,基于 ArkUI NativeXComponent。太新太小,只当信号看:winit 0.31 的 backend 插件化让第三方 ohos backend 成为可能,长期或有转机。
  3. **Servo 的做法**:不用 winit,ArkTS glue([jschwe/ServoDemo](https://github.com/jschwe/ServoDemo))+ 自己的窗口抽象。IME/键盘支持单独做([servo PR #34188](https://github.com/servo/servo/pull/34188))。

**自建窗口抽象要做的事(如果不赌社区层)**——这份清单同时就是 svelte-rs 平台抽象层的 OHOS 需求说明:

| 能力 | OHOS 对应物 | 现成 Rust 绑定 |
|---|---|---|
| 生命周期 | UIAbility(onCreate/onForeground/onBackground/onDestroy),经 ArkTS 壳 NAPI 转发;surface 级用 XComponent 回调 | openharmony-ability;napi-ohos |
| 窗口/surface | XComponent surface created/changed/destroyed(注意后台可能销毁 surface,GPU 资源需重建) | ohos-xcomponent-binding |
| 输入 | XComponent 回调:touch/mouse/axis/key;更细粒度走 multimodalinput | ohos-sys(feature 门控) |
| vsync | OH_NativeVSync(注意回调线程) | ohos-vsync-binding |
| IME | IME Kit(InputMethod):自绘 UI 必须自己 attach 输入法、处理组合文本与候选窗定位 | [openharmony-rs/ohos-ime](https://github.com/openharmony-rs/ohos-ime)、ohos-ime-binding |
| DPI/density、旋转 | XComponent size change + display 信息(ArkTS 侧查询转发) | 需自己拼 |
| 剪贴板/光标/拖拽等桌面能力 | Pasteboard Kit 等,多数只有 ArkTS API | 经 NAPI 桥 |

判断:**不要把 winit 作为架构前提**。svelte-rs 本来就要跨 Win/Linux/macOS/OHOS 四平台,winit 对 OHOS 的空缺+社区层的不稳定,意味着正确做法是定义自己的窄窗口抽象 trait(create surface、事件流、vsync、IME 六七个接口),桌面端可以拿 winit 当其中一个实现,OHOS 端用 openharmony-ability 起步、保留换成手写 XComponent glue 的退路。

---

## 6. 先例:别人是怎么上鸿蒙的

| 项目 | 路线 | 对我们的参考价值 |
|---|---|---|
| **Flutter-OHOS**([OpenHarmony-SIG/flutter_flutter](https://gitee.com/openharmony-sig/flutter_flutter),现迁 atomgit openharmony-tpc) | **完全自绘**:XComponent 拿 OHNativeWindow,Skia(GLES/Vulkan)或 Impeller(Vulkan/ArkGraphics)直接绘制,绕开 ArkUI 渲染管线;PlatformView 混合原生组件 | 最强先例:证明自绘引擎被鸿蒙生态**官方接纳**(2026 路线图:SIG 主导、季度跟进上游 3.41+、年内适配 200+ 三方库)。我们的渲染架构与它同构 |
| **Servo OHOS**(jschwe/华为) | Rust 自绘引擎完整跑在 OHOS:ArkTS glue + XComponent + EGL,IME/键盘已接,构建签名 CI 全流程文档化([Servo Book](https://book.servo.org/building/openharmony.html)) | **最贴近我们的技术栈**(Rust+EGL+ArkTS 壳),它踩过的坑(签名、hvigor、IME)可直接抄作业 |
| **React Native OHOS**([ohos_react_native](https://gitee.com/openharmony-sig/ohos_react_native),华为×Software Mansion,支持 RN 0.72/0.77,0.82 适配中) | **非自绘**:Fabric 经 C-API 直连 ArkUI 组件渲染 | 反例参照:说明"映射到 ArkUI 原生组件"是另一条路,但对细粒度响应式+自绘的我们不适用;其 NAPI→C-API 演进印证了 NAPI 热路径开销问题 |
| **uni-app x** | 非自绘:编译到 ArkTS/ArkUI | 同上,证明"编译到原生 UI"路线存在但与 svelte-rs 的渲染模型冲突 |
| **Tauri/wry** | [wry PR #1607](https://github.com/tauri-apps/wry/pull/1607)(richerfu)2026-06-08 合入 feat/open-harmony 分支,基于系统 **ArkWeb** webview;[Dioxus PR #4508](https://github.com/DioxusLabs/dioxus/pull/4508) 仍是 draft(2026-03 最后活动) | webview 路线,与我们无直接关系;但 Tauri 维护者确认 cargo-mobile2 链路已打通,说明 Rust 移动打包工具在跟进 ohos |
| 其他信号 | ohos-rs org 有 **gpui-template**(Zed 的 GPUI 在 ohos 的模板,2026-02);Godot 有 OHOS 官方支持提案 | 自绘 Rust/native UI 上鸿蒙已是多方并行的趋势 |

---

## 7. 上架审核与政策风险(联网核实+标注)

- **没有任何公开政策禁止自绘渲染引擎**。Flutter-OHOS 由 OpenHarmony SIG 官方运营、华为深度参与;RN 移植是华为出资与 Software Mansion 合作([官方博客](https://swmansion.com/blog/huawei-x-software-mansion-bringing-react-native-support-to-harmonyos-next-82e02bd75549/));转转等商业 App 已用 Flutter 版本过审上架。**"自绘引擎过不了鸿蒙审核"是谣言**,风险定级:低。
- **真实存在的硬约束:W^X / JIT 禁令**。自 HarmonyOS 5.0(API 12)起,应用申请可执行匿名内存被禁止,除系统内置 JS 引擎外的 VM 不能 JIT(这也是 .NET CoreCLR 无法上鸿蒙的原因,[来源:cnblogs .NET 适配进展](https://www.cnblogs.com/CeSun/p/18706813),二手来源但与多方信息一致)。**对 svelte-rs 完全无影响甚至是利好**:我们是 AOT 编译的 Rust,无 JIT;而竞品中任何依赖动态代码生成的方案在鸿蒙都会被卡死。
- **性能基线审核**(二手来源,腾讯云/CSDN 社区文章,未在华为官方公开文档中逐条核实):冷启动 ≤800ms、内存峰值、帧率稳定性(连续 60 帧波动 ≤5%)等指标据称纳入审核。自绘引擎首帧(shader 编译、字体加载)是重点风险,Flutter-OHOS 为此做过专项优化(Dart VM 创建与 pipeline 加载并行,首帧 -50ms)。应对:预编译 shader、字体懒加载、首屏骨架。
- **隐性成本**:自绘 UI 的**无障碍**(accessibility 树)、**多语言/镜像布局**、**IME 完整度**在审核与商店评级中可能被人工检查。Flutter-OHOS 接了 ArkUI accessibility 桥,我们长期也躲不掉。
- 元服务(卡片、服务小组件)必须 ArkUI——若未来要做桌面卡片,那部分只能 ArkTS 写,架构上要预留。

---

## 8. 打包发布与 CI(已核实)

- 产物形态:HAP(模块包)→ APP(上架包);构建工具 **hvigor**(`hvigorw` wrapper,类 gradlew),官方支持[纯命令行流水线](https://developer.huawei.com/consumer/en/doc/harmonyos-guides/ide-command-line-building-app),**不需要 DevEco Studio GUI**。HarmonyOS NEXT 需要 Command Line Tools 包(含 hvigor/ohpm/codelinter/hstack),有 Linux 版。
- 签名:`build-profile.json5` 的 `signingConfigs`(证书 + keystore + 加密口令)。坑(Servo 实测):**密钥材料只能由 DevEco Studio 生成**(存于 `~/.ohos/config`),Linux CI 需要先在 Win/mac 机器上生成一次再拷入 CI secret。OpenHarmony 板卡用 `default` 签名,HarmonyOS NEXT 真机/商店必须 AGC 证书(debug 证书可自动化,release 证书走 AGC 后台)。
- 设备侧:`hdc`(等价 adb)安装调试;真机需开发者模式;模拟器为 x86_64(DevEco 提供,NEXT 模拟器需申请),另有社区 [ohos-qemu](https://github.com/harmony-contrib) 标准系统镜像。
- CI 可行性:**已被证实**——Servo 在 GitHub CI 上构建 OHOS 产物;社区有 GitHub Actions 教程([Medium/Snapp Mobile](https://medium.com/snapp-mobile/ci-cd-for-openharmony-project-github-action-8ba7940a3d2d))。流程 = 缓存 SDK/Command Line Tools → `rustup target add` → cargo build(ohrs)→ 把 .so 放进 ArkTS 工程 libs/arm64-v8a → `hvigorw assembleHap` → 签名。全程 Linux 可跑(除首次生成签名材料)。

---

## 9. 推荐集成分层与最小可行 Demo

### 9.1 分层架构(结论)

```
┌───────────────────────────────────────────────┐
│ ArkTS 壳(尽量薄,<500 行,模板化生成)          │
│  EntryAbility 生命周期 → NAPI 转发             │
│  单个全屏 XComponent(surface)                 │
│  低频系统服务桥:权限/剪贴板/通知/分享/IME面板    │
├───────────────────────────────────────────────┤
│ Rust 平台适配层(svelte-rs 自己的窗口抽象 trait) │
│  OHOS backend:openharmony-ability 起步         │
│  (退路:直接 ohos-xcomponent-binding 手写)      │
│  vsync(ohos-vsync-binding)/ IME(ohos-ime)   │
├───────────────────────────────────────────────┤
│ Rust 核心(全平台共享,与 OHOS 无关)            │
│  svelte-rs 编译产物(细粒度更新指令)            │
│  渲染器:wgpu(GLES 后端→将来 Vulkan)          │
│  文本:rustybuzz + HarmonyOS Sans 系统字体      │
└───────────────────────────────────────────────┘
```

原则:**ArkTS 只做"进程入口 + 系统服务代理",一切 UI 与状态都在 native**。这与 Flutter-OHOS/Servo 的分层一致,是被验证过的形态。桥接用 napi-ohos,渲染热路径零 NAPI。

### 9.2 最小可行 Demo 步骤清单

1. Win/mac 装 DevEco Studio(生成签名材料 + 模拟器);CI/日常构建装 Linux Command Line Tools。
2. `rustup target add aarch64-unknown-linux-ohos x86_64-unknown-linux-ohos`;`cargo install ohrs`。
3. DevEco 建空 ArkTS 工程:EntryAbility + 全屏 `XComponent(type: surface, libraryname: "svelte_demo")`。
4. Rust cdylib:`napi-ohos` 注册模块,`OH_NativeXComponent_RegisterCallback` 拿 `OHNativeWindow*`(或直接套 openharmony-ability 模板)。
5. EGL context + GLES3 画清屏色/三角形(第一里程碑:像素上屏)。
6. 接 `OH_NativeVSync` 帧循环,打印帧间隔验证 vsync 驱动(第二里程碑:稳定 60/120fps 动画)。
7. 触摸回调改颜色/拖方块(第三里程碑:输入闭环)。
8. 换 wgpu(GLES 后端 + raw-window-handle OhosNdk)重画同场景,评估 wgpu 在真机的兼容性。
9. `ohrs build --arch arm64` + `hvigorw assembleHap` + 自动签名 + `hdc install` 真机(第四里程碑:全链路打包)。
10. GitHub Actions 复刻步骤 9(Linux,签名材料入 secret),产出可安装 hap(第五里程碑:CI)。
11. (进阶)ohos-ime 接文本输入框,验证中文 IME 组合文本——这是自绘 UI 最大的工程暗礁,越早碰越好。

预估:熟悉 Rust 但不熟鸿蒙的工程师,里程碑 1–4 约 1–2 周,IME 单独再留 1–2 周。

---

## 10. 未决问题(需原型验证)

见文末结构化输出;核心是 wgpu-Vulkan 时间表、模拟器 GPU 能力、IME 完成度、无障碍审核尺度、openharmony-ability 的 API 稳定性。

---

## 来源链接

**官方文档/仓库(一手)**
- https://doc.rust-lang.org/rustc/platform-support/openharmony.html — Rust ohos target(Tier 2 host tools、工具链配置)
- https://book.servo.org/building/openharmony.html — Servo OHOS 构建/签名/hvigor 全流程
- https://developer.huawei.com/consumer/cn/doc/harmonyos-guides/native-vsync-guidelines — OH_NativeVSync
- https://developer.huawei.com/consumer/en/doc/harmonyos-guides/ide-command-line-building-app — 命令行流水线
- https://github.com/ohos-rs/ohos-rs 、https://ohos.rs — ohos-rs / napi-ohos / ohrs
- https://github.com/ohos-rs/ohos-native-bindings — xcomponent/vsync/ime 高层绑定
- https://github.com/openharmony-rs — ohos-sys、ohos-ime(jschwe)
- https://github.com/harmony-contrib/openharmony-ability — Rust ability 壳(winit 适配基础)
- https://github.com/rust-windowing/winit/issues/4081 — winit ohos 请求(仍 open)
- https://github.com/gfx-rs/wgpu/pull/7085 — wgpu GLES OHOS(2025-02 合入)
- https://github.com/ash-rs/ash/pull/1016 — ash VK_OHOS_surface(2026-05 合入,ash 0.39)
- https://github.com/servo/servo/pull/32594 、https://github.com/servo/servo/pull/34188 — servoshell OHOS、IME
- https://github.com/tauri-apps/wry/pull/1607 — wry ArkWeb OHOS(2026-06 合入分支)
- https://github.com/DioxusLabs/dioxus/pull/4508 — Dioxus OHOS(draft)
- https://gitee.com/openharmony-sig/flutter_flutter — Flutter-OHOS
- https://gitee.com/openharmony-sig/ohos_react_native — RN-OHOS
- https://swmansion.com/blog/huawei-x-software-mansion-bringing-react-native-support-to-harmonyos-next-82e02bd75549/ — 华为×SWM RN 合作

**社区/二手(观点与政策类,已标注)**
- https://blog.csdn.net/zackslee/article/details/144914441 — Impeller 鸿蒙化(Vulkan/VK_OHOS_surface)
- https://zhuanlan.zhihu.com/p/719918885 — RN-OHOS 架构(NAPI→C-API)
- https://zhuanlan.zhihu.com/p/1915289056989917432 — 跨平台框架适配鸿蒙原理综述
- https://www.cnblogs.com/CeSun/p/18706813 — .NET 适配鸿蒙(JIT/W^X 禁令)
- https://cloud.tencent.com/developer/article/2480057 等 — 上架流程与性能基线(未经华为官方逐条核实)
- https://medium.com/snapp-mobile/ci-cd-for-openharmony-project-github-action-8ba7940a3d2d — GitHub Actions CI
- https://harmonyosdev.csdn.net/69b7651754b52172bc61b1ab.html — Flutter 鸿蒙 2026 路线
