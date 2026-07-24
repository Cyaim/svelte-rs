# 29 · 距离 Electron 还差哪些 + JS/DOM 事件覆盖核对

> 方法:Electron 模块面与 DOM 事件全集**联网核实**(electronjs.org/docs、MDN
> Element 事件参考,2026-07);svelte-rs 现状以**仓库代码实证**为准(main =
> 事件 12 个模板属性 + 多窗体 run_multi + disabled + 右键/指针移动派发,即本轮
> #62–#68 合入后的状态)。判定依据随行给出代码位置。

---

## TL;DR

**两个问题分别回答:**

1. **距离 Electron 还差哪些** —— 要先分清**两层**,否则会把"路线选择"误当"缺口":
   - **架构层(不是缺口,是另一条路)**:svelte-rs 是**原生场景树 UI**(编译期细粒度
     更新,winit + taffy + Parley + CPU/vello),**不是 Chromium/webview 壳**。所以它
     永远不会"追平"Electron 的 web 面——无 HTML/JS/CSS 网页内容、无 V8/Node、无
     DevTools、无 npm 前端生态、无 Web API。它的**同类**是 Tauri(去掉 webview)/
     egui / Slint / Dioxus-native,不是 Electron 的渲染器。**代价**:不能复用 Web
     前端资产;**收益**:内存/启动/包体一个量级优势(百万控件 28MB vs Electron 每窗
     数百 MB)、真原生渲染。**IPC** 在 svelte-rs 不存在也不需要(单进程共享运行时,
     跨窗联动免费),不是能力缺口而是架构红利。
   - **桌面外壳集成层(真缺口、可补)**:这才是"距离 Electron"里该谈的部分。多窗体
     本轮已补(`run_multi`);但**窗口属性控制、应用菜单、系统托盘、原生对话框、系统
     通知、全局快捷键、OS 文件拖入、深链/协议、自动更新、签名安装器——目前基本全空**。
     好消息:这些多数与渲染路线正交,引 Rust 生态成熟 crate(muda/tray-icon/rfd/
     notify-rust/global-hotkey/opener)+ 转发 winit 既有事件即可,不动内核。

2. **JS 上有的事件是否全覆盖** —— **否,但"桌面有意义的高频交互事件已覆盖大半**"。
   模板层 12 个 `on*` 属性(onclick/onkeydown/onkeyup/onfocus/onblur/onpointerenter/
   onpointerleave/oninput/onsubmit/onscroll/oncontextmenu/onpointermove)+ 内部完整的
   IME/剪贴板/文本编辑/五段键盘路由。**缺口**集中在:`dblclick`/`auxclick`(中键)、
   `pointerover/out/cancel/capture`、`change`/`beforeinput`/`select`(作为事件)、
   **拖放(drag-and-drop)**、**触屏(touch)**、`onwheel` 原始滚轮、
   `mouseover/mouseout`(vs 已有 enter/leave)。Web 独有的 `animation*`/`transition*`/
   `fullscreenchange`/`scrollsnap*` 在编译期 CSS + 原生动画模型下**无 1:1 对应也不需要**。

**一句话**:事件面对"手写桌面控件"已够用,缺的是拖放/触屏/中键这类次高频项;距 Electron
最大的一块是**原生桌面集成(菜单/托盘/对话框/通知/全局快捷键/OS 拖入)**,而非渲染或事件。

---

## §0 坐标系:svelte-rs 不是 Electron 的同类

对比前先摆正坐标,否则整份对照会失真。

| 维度 | Electron | svelte-rs |
|---|---|---|
| UI 渲染 | Chromium(HTML/CSS/JS,完整浏览器引擎) | **原生场景树**:模板编译成对 retained 树的定点更新,winit + taffy 布局 + Parley 文本 + CPU(tiny-skia)/GPU(vello)双后端。**无 VDOM/diff**(CLAUDE.md「架构速记」) |
| 逻辑运行时 | Node.js(V8)+ 渲染进程 V8 | 编译后的 **Rust**;响应式是线程内 runtime(句柄 `Copy + !Send`) |
| 进程模型 | 多进程(main ↔ N renderer),靠 **IPC** 通信 | **单进程**;多窗共享同一运行时,跨窗联动免序列化免总线(`shared_signal_drives_multiple_docs` 测试证:一个 signal 同时驱动多个 Doc) |
| "CSS" | 真浏览器 CSS 引擎 | 真实 CSS 语法的**封闭子集**在**编译期**求解成原生样式,零运行时选择器引擎(docs/CSS-SUPPORT、research/12) |
| DevTools / Web API | 有(F12、fetch、localStorage、WebGL…) | 无(不是浏览器) |
| 生态 | npm(海量前端) | Cargo(Rust);无前端包复用 |
| 内存 / 启动 / 包体 | 每窗数百 MB,启动数百 ms,包体 ~100MB+ | 百万控件 28MB 工作集(CPU 后端实测,research/16-18);原生启动;包体一个量级小 |

**结论**:凡是"能不能跑网页/复用 React 组件/用 npm"这类问题,答案是**架构性地不能**,
且**这是设计目标**(编译到原生换性能),不列为"缺口"。下面 §1 只谈**桌面外壳集成**
这个正交面——那才是 Electron 应用真正依赖、而 svelte-rs 目前欠缺的。

---

## §1 距离 Electron:桌面外壳能力对照

逐 Electron 模块核对。状态:✅ 已实现 / 🚧 部分 / ❌ 无 / 🟦 架构性不适用。

### 1.1 窗口(BrowserWindow / BaseWindow）

| 能力 | Electron | svelte-rs | 依据 / 补法 |
|---|---|---|---|
| 多窗口 | `new BrowserWindow()` ×N | ✅ **本轮已补**:`run_multi(vec![(标题, build), …])` 启动即开 N 窗;`App` 拆循环级 + 每窗 `Pane`,`windows: HashMap<WindowId, Pane>` 路由;共享运行时跨窗联动零样板 | sv-shell `run_multi`、`struct Pane`、`examples/multi-window` |
| 运行期动态开窗 | 随时 `new` | ❌ 无 | `BuildFn` 是 `!Send` 不能走 `EventLoopProxy`;需 thread_local BuildFn 队列 + 无载荷唤醒(已记 open-issues) |
| 窗口标题 | `setTitle` | 🚧 仅创建时 `run_app/run_multi(title,…)`,无运行期 `set_title` | winit `Window::set_title` 转发即可 |
| 尺寸 / 位置 | `setBounds/setPosition/setSize` | ❌ 尺寸创建时硬编码 480×400,位置无 API | winit `set_inner_size`/`set_outer_position` |
| 最小/最大尺寸约束 | `setMinimumSize` | ❌ 无 | winit `set_min_inner_size`/`set_max_inner_size` |
| 可调整大小 | `resizable` | 🚧 用 winit 默认(可调),无开关 | winit `set_resizable` |
| 无边框 / 透明 / 置顶 | `frame:false`/`transparent`/`alwaysOnTop` | ❌ 全无 | winit `with_decorations(false)`/`with_transparent`/`WindowLevel::AlwaysOnTop` |
| 全屏 / 最小化 / 最大化(程序控制) | `setFullScreen`/`minimize`/`maximize` | ❌ 无(仅被动响应 OS 尺寸变化) | winit `set_fullscreen`/`set_minimized`/`set_maximized` |
| 窗口图标 | `icon` | ❌ 无 | winit `set_window_icon` |
| 模态子窗 | `modal:true, parent` | 🚧 只有**自绘**模态弹层(overlay `Anchor::WindowCenter`),非独立 OS 窗 | overlay.rs;真原生模态需 winit 子窗 + OS modal |
| 关最后一窗才退 | 手动管 | ✅ 本轮 `windows.is_empty() → exit`(多窗语义) | sv-shell window_event CloseRequested |

**小结**:多窗体本轮补齐了"能不能开多个窗"这个门槛;但**窗口属性控制面(位置/尺寸约束/
无边框/透明/置顶/全屏/图标/最小最大化)几乎全空**——而这些几乎全是 winit **已经提供**的
API,只差在 sv-shell 暴露出来(低风险、无需动内核)。

### 1.2 原生菜单

| 能力 | Electron | svelte-rs | 依据 / 补法 |
|---|---|---|---|
| 应用菜单栏 | `Menu.setApplicationMenu` | ❌ 无 | 引 `muda`(tao/tauri 生态) |
| 系统托盘 | `new Tray()` | ❌ 无 | 引 `tray-icon`(re-export muda 做托盘菜单) |
| 右键上下文菜单 | `Menu.popup()`(原生) | 🚧 **半有**:本轮 `oncontextmenu` 右键**事件**已接(shell `MouseButton::Right` 命中派发,回调带逻辑坐标),菜单本身用**自绘 overlay**(`Anchor::Point`,a11y `Role::Menu`)——**非原生菜单**,但功能闭环(用户在回调里开 overlay 菜单) | sv-shell 右键派发 + overlay.rs;要原生外观再引 muda |

**小结**:右键**交互**本轮通了(事件 + 自绘菜单基础设施),但**原生菜单外观/系统菜单栏/
托盘**是明确空白,且是桌面应用的高频需求。

### 1.3 对话框 / 通知

| 能力 | Electron | svelte-rs | 补法 |
|---|---|---|---|
| 文件打开/保存对话框 | `dialog.showOpenDialog` | ❌ 无 | 引 `rfd`(Rust file dialog,跨平台原生) |
| 消息框 / 错误框 | `dialog.showMessageBox` | 🚧 只有自绘 `Dialog.svelte` 弹层,无原生 | `rfd::MessageDialog` |
| 系统通知 | `new Notification()` | ❌ 无 | 引 `notify-rust`(或平台 API) |

### 1.4 系统集成

| 能力 | Electron | svelte-rs | 依据 / 补法 |
|---|---|---|---|
| 剪贴板 | `clipboard`(文本+图片+HTML) | ✅ **文本**(arboard,懒建、失败静默降级;已接 Ctrl+C/X/V 编辑内核)。**无图片剪贴板**(arboard `default-features=false`) | sv-shell `ShellClipboard`、`clipboard_text()`/`set_clipboard_text()` |
| 打开外部链接/文件 | `shell.openExternal/openPath` | ❌ 无 | 引 `opener` 或 `open` crate |
| 全局快捷键(无焦点也触发) | `globalShortcut.register` | ❌ 无(现有 `shortcuts` 是**应用内**、窗口获焦时才派发) | 引 `global-hotkey` crate |
| 多显示器 / 屏幕信息 | `screen.getAllDisplays` | ❌ 无(仅读 `scale_factor` 做 DPI,不枚举显示器) | winit `available_monitors`/`primary_monitor` |
| OS 文件拖入 | HTML5 drag/drop + `webContents` | ❌ 无(`window_event` 不处理 `DroppedFile`/`HoveredFile`) | winit **已提供** `WindowEvent::DroppedFile/HoveredFile`,转发即可 |
| 系统主题(暗色) | `nativeTheme` | ❌ 无显式 API | winit theme / `dark-light` crate |
| 电源事件 | `powerMonitor` | ❌ 无 | 平台 API / crate |
| 无障碍 | Chromium a11y | ✅ **AccessKit** 已接(语义树增量推送、动作派发,research/24) | sv-shell a11y.rs |

### 1.5 分发 / 更新 / 协议

| 能力 | Electron | svelte-rs | 依据 |
|---|---|---|---|
| 打包安装器 | electron-builder/forge(nsis/dmg/deb) | 🚧 `examples/showcase` 有 cargo-packager 配置产 nsis/dmg/appimage **未签名**(手动工作流,DESIGN R4) | 无代码签名/公证 |
| 自动更新 | `autoUpdater` | ❌ 无 | 引 tauri-updater 类方案 |
| 深链 / 自定义协议 | `protocol`/`setAsDefaultProtocolClient` | ❌ 无 | 平台注册 + winit |
| 崩溃上报 | `crashReporter` | ❌ 无 | — |

---

## §2 JS/DOM 事件覆盖核对

完整 DOM 事件全集(MDN Element,2026-07 核实)× svelte-rs 现状。列:
**✅ 有模板属性** / **🟨 内部有(功能覆盖但无 `onX` 属性)** / **❌ 未做** / **🟦 Web 独有·原生无对应也不需要**。

事件流三层:**codegen**(`sv-compiler` 把 `on*` 翻成 sv-ui setter)→ **ui**(`ViewNode`/
`InputState` 存回调)→ **shell**(winit `WindowEvent` 派发)。

### 2.1 鼠标 Mouse

| DOM 事件 | svelte-rs | 说明 |
|---|---|---|
| `click` | ✅ `onclick` | 命中 + `set_on_click`;键盘 Enter/Space 也走激活段触发 |
| `contextmenu` | ✅ `oncontextmenu` | **本轮**:shell `MouseButton::Right` 命中派发,回调带逻辑 `(x,y)` |
| `mousedown`/`mouseup` | 🟨 内部 | 有 `on_pointer_down/up`(sv-ui),但**只内部接 `:active`/按压**,无 `onpointerdown/up`/`onmousedown/up` 模板属性 |
| `mouseenter`/`mouseleave` | ✅ 经 `onpointerenter`/`onpointerleave` | shell `update_hover` 派发;Pointer Events 取代 Mouse Events(现代做法) |
| `mousemove` | ✅ 经 `onpointermove` | **本轮**:shell `CursorMoved` 命中派发,逻辑坐标 |
| `mouseover`/`mouseout` | ❌ | 与 enter/leave 的区别是冒泡+子元素进出重触发;桌面场景通常 enter/leave 已够 |
| `dblclick` | ❌ 无事件 | shell 内部有 `click_streak()` 三击计数**仅用于文本选词/全选**,不作 DOM 事件暴露 |
| `auxclick`(中键) | ❌ | shell 只派发 `MouseButton::Left`/`Right`,中键落 `_ => {}` |
| `wheel` | 🟨 经 `onscroll` | winit `MouseWheel` → 滚动链 `route_wheel_with` → 滚动结果由 `onscroll`(`Fn(f32,f32)`)透出;**无原始 `onwheel`(delta)属性** |

### 2.2 指针 Pointer

| DOM 事件 | svelte-rs | 说明 |
|---|---|---|
| `pointerenter`/`pointerleave` | ✅ `onpointerenter`/`onpointerleave` | 见上 |
| `pointermove` | ✅ `onpointermove` | 本轮 |
| `pointerdown`/`pointerup` | 🟨 内部(`:active`),无模板属性 | 同 mousedown/up |
| `pointerover`/`pointerout` | ❌ | 同 mouseover/out |
| `pointercancel` | ❌ | 无触屏/笔,暂无来源 |
| `gotpointercapture`/`lostpointercapture` | ❌ | 无显式指针捕获 API(拖动靠 shell `drag_input`/`drag_scroll` 内部状态实现"跟手",不暴露捕获事件) |
| `pointerrawupdate` | ❌ | 高频原始更新,未做 |

### 2.3 键盘 Keyboard

| DOM 事件 | svelte-rs | 说明 |
|---|---|---|
| `keydown` | ✅ `onkeydown` | 自动 `set_focusable`;`KeyEvent` 带 DOM 心智 `stop_propagation()`/`prevent_default()` |
| `keyup` | ✅ `onkeyup` | 与 keydown 共用 `on_key` 槽,按 `released` 相位分派(只走捕获/冒泡段) |
| `keypress`(弃) | 🟦 | Web 已废弃;字符输入走 IME/`text` 路径 |
| — 五段路由 | ✅ 超出 DOM 单一模型 | `dispatch_key`:捕获 → 冒泡 → Tab 导航 → Enter/Space 激活 → 快捷键(research/20) |

### 2.4 焦点 Focus

| DOM 事件 | svelte-rs | 说明 |
|---|---|---|
| `focus`/`blur` | ✅ `onfocus`/`onblur` | 合成进单一 `set_on_focus_change(bool)`,与 `:focus` 伪类共线 |
| `focusin`/`focusout`(冒泡版) | ❌ | 焦点变化回调不冒泡;桌面单焦点模型,通常不需要 |

### 2.5 表单 / 输入 Form

| DOM 事件 | svelte-rs | 说明 |
|---|---|---|
| `input` | ✅ `oninput`(`<input>`,签名 `\|&str\|`) | 挂在 `InputState`;`bind:value` 内部也用它 |
| `submit` | ✅ `onsubmit`(单行 Enter) | 文本输入回车触发 |
| `change` | 🟨 由 `bind:value`/`bind:checked` 覆盖语义 | 无独立 `onchange` 事件,但双向绑定达成同效 |
| `beforeinput` | ❌ | 未做(可用于输入拦截/改写) |
| `select`(选区变化) | 🟨 内部 | 文本选区(双击选词/三击全选/拖选)在 `InputState`,不作 `onselect` 事件暴露 |
| `reset` | 🟦 | 无 `<form>` 宿主概念 |

### 2.6 剪贴板 / IME

| DOM 事件 | svelte-rs | 说明 |
|---|---|---|
| `copy`/`cut`/`paste` | 🟨 内部(Ctrl+C/X/V) | 编辑内核直接处理,**无 `oncopy/oncut/onpaste` 事件属性**(不能拦截/改写剪贴板内容) |
| `compositionstart`/`update`/`end` | 🟨 内部(winit `Ime` → `handle_ime`) | IME 预编辑 over-the-spot、候选窗跟随光标(research/21);功能完整,但**无 `oncompositionX` 事件属性** |

### 2.7 滚动 / 滚轮 Scroll·Wheel

| DOM 事件 | svelte-rs | 说明 |
|---|---|---|
| `scroll` | ✅ `onscroll`(`Fn(f32,f32)` 新 (x,y)) | + `bind:scrolly` 双向桥、虚拟化(research/22) |
| `wheel` | 🟨 见 §2.1 | 无原始 delta 事件 |
| `scrollend`/`scrollsnapchange` | ❌ / 🟦 | scroll-snap 是 CSS 特性,未做 |

### 2.8 拖放 Drag & Drop

| DOM 事件 | svelte-rs | 说明 |
|---|---|---|
| `dragstart`/`drag`/`dragend`/`dragenter`/`dragover`/`dragleave`/`drop` | ❌ **全无** | HTML5 拖放协议未做。**注意区分**:①元素间拖放(reorder/拖拽上传)——`onpointermove` + 手写状态可搭出，但无 dataTransfer 协议;② **OS 文件拖入**——winit 有 `DroppedFile/HoveredFile`,shell 未转发（§1.4）。**这是与 Electron 差距较明显的一类** |

### 2.9 触屏 Touch

| DOM 事件 | svelte-rs | 说明 |
|---|---|---|
| `touchstart`/`move`/`end`/`cancel` | ❌ 全无 | winit 有 `Touch` 事件，shell 落 `_ => {}`；桌面优先，触屏（含鸿蒙）是后续 |

### 2.10 Web 独有 · 原生无对应也不需要(🟦)

| DOM 事件 | 为何不需要 |
|---|---|
| `animationstart/iteration/end/cancel`、`transitionrun/start/end/cancel` | svelte-rs 动画是**原生模型**(`in:fade` 编译期 + anim 载荷 + 帧调度 ADR-6),非 CSS 动画的 DOM 事件回调 |
| `fullscreenchange/error` | 全屏是窗口 API（§1.1）非文档事件 |
| `scrollsnapchange/changing` | CSS scroll-snap 未做 |
| `securitypolicyviolation`、`beforematch`、`contentvisibility*`、`beforexrselect`、`DOMActivate`(弃) | 浏览器/CSP/WebXR 语境，无对应物 |

### 2.11 事件系统语义差异(非缺口，刻意)

- **事件修饰符 `|once`/`|preventDefault` 等**:❌ 与 Svelte 5 同步不做（5 已删）；键盘侧
  给了 `KeyEvent::stop_propagation()`/`prevent_default()` 方法（DOM 心智）。
- **`on:` 遗留指令**:硬错误 + 指路属性形态（对齐 Svelte 5）。
- **冒泡**:键盘有真冒泡（沿焦点父链）；指针/点击是**命中最上层单点派发**，不做 DOM 式
  指针冒泡（桌面惯例，避免全树冒泡开销）。

---

## §3 结论与优先级建议

### 事件面:够手写控件,缺次高频与拖放/触屏

**已覆盖**(模板属性 12 + 内部完整 IME/剪贴板/编辑/五段键盘):足以手写按钮、表单、
可点击/悬停/获焦/滚动的交互控件，右键与自定义拖拽（pointermove）本轮补上。

**建议补的顺序**(按"手写桌面应用"痛感):
1. **OS 文件拖入**(`DroppedFile/HoveredFile`,winit 已给,转发 + 一个 `ondrop`-类回调)——桌面刚需，成本低。
2. **`onpointerdown`/`onpointerup` 模板属性**（内部已有回调，只差暴露）——手写拖拽/长按的基础。
3. **`onwheel` 原始滚轮**（缩放/自定义滚动手势）+ **`ondblclick`**（双击语义已有计数，暴露即可）。
4. 元素间拖放协议（dataTransfer）、触屏——有前置（拖放设计 / 触屏设备），排后。

### Electron 面:最大缺口是原生桌面集成

**架构层不追**（web 内容/npm/DevTools 是路线选择）。**该补的桌面外壳集成**，按性价比:
1. **窗口属性暴露**（位置/尺寸/尺寸约束/无边框/透明/置顶/全屏/图标/最小最大化）——winit **已提供**，纯 sv-shell 暴露，**最低成本最高回报**。
2. **原生文件/消息对话框**（`rfd`)——桌面刚需，一个 crate。
3. **系统托盘 + 应用菜单 + 原生右键菜单**（`tray-icon`/`muda`)——常驻类应用刚需。
4. **系统通知**（`notify-rust`)、**打开外部链接**（`opener`)——轻量高频。
5. **全局快捷键**（`global-hotkey`)、**多显示器信息**（winit `available_monitors`)。
6. 运行期动态开窗、深链/协议、自动更新、代码签名——排后（各有前置）。

**判断**:svelte-rs 距"能替代 Electron 写一个典型桌面工具应用"，**渲染和事件已不是主要
障碍**（本轮补完多窗体 + 右键 + 指针移动后尤其如此）；**主要障碍是原生桌面集成层的
广度**——而这层多数是"引 crate + 转发 winit 事件"的工程活，不涉及内核重构。Rust 桌面
生态（tao/tauri 沉淀的 muda/tray-icon/rfd/notify-rust/global-hotkey）已把这些 crate 备好。

---

## 附录 A · Electron 主进程模块全集(联网核实 2026-07)

窗口/视图:app、BaseWindow、BrowserWindow、View、WebContentsView、ImageView、
webContents、webFrameMain ·
原生 UI:Menu、MenuItem、Tray、TouchBar、ShareMenu ·
对话/通知:dialog、Notification、pushNotifications ·
系统:globalShortcut、shell、clipboard、nativeImage、nativeTheme、screen、
powerMonitor、powerSaveBlocker、systemPreferences、desktopCapturer、safeStorage ·
进程/IPC:ipcMain、MessageChannelMain、MessagePortMain、utilityProcess、crashReporter ·
网络:net、session、protocol、inAppPurchase ·
分发:autoUpdater。

## 附录 B · DOM 事件全集(MDN Element,联网核实 2026-07)

Mouse:click、dblclick、mousedown/up/move/enter/leave/over/out、contextmenu、auxclick ·
Pointer:pointerdown/up/move/enter/leave/over/out/cancel/rawupdate、got/lostpointercapture ·
Keyboard:keydown、keyup ·
Focus:focus、blur、focusin、focusout ·
Form:input、beforeinput、change、select、submit、reset ·
Drag:dragstart、drag、dragend、dragenter、dragover、dragleave、drop ·
Touch:touchstart/move/end/cancel ·
Wheel:wheel · Scroll:scroll、scrollend、scrollsnapchange/changing ·
Clipboard:copy、cut、paste · Composition:compositionstart/update/end ·
Animation:animationstart/iteration/end/cancel · Transition:transitionrun/start/end/cancel ·
Fullscreen:fullscreenchange/error。

## 数据来源

- Electron API 模块:<https://www.electronjs.org/docs/latest/api/app>(sidebar 全模块)
- DOM 事件参考:<https://developer.mozilla.org/en-US/docs/Web/API/Element>、
  <https://developer.mozilla.org/en-US/docs/Web/Events>、<https://www.w3.org/TR/uievents/>
- Rust 桌面生态 crate:tray-icon / muda / rfd / notify-rust / global-hotkey / opener
  (tao·tauri 生态,见 <https://crates.io/crates/tray-icon>、<https://lib.rs/gui>)
- svelte-rs 现状:仓库代码实证(sv-compiler/codegen.rs 事件白名单、sv-ui/lib.rs
  ViewNode·InputState·RareHandlers、sv-shell/lib.rs window_event·run_multi、
  docs/SVELTE-SUPPORT.md、docs/plans/open-issues.md)
