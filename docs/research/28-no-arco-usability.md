# 28 · 不用组件库,今天能用这个框架开发什么(可用性实证)

> 2026-07-24。方法:对 main(f4e0091,B 批合入后)做四视角并行审计(控件面/样式布局/
> 应用外壳/开发体验)+ 一轮交叉批评补漏 + **仓库外最小工程实测**;所有论断带
> 文件:行号 或 示例/测试名。与其它调研不同,本篇不联网——全部证据来自仓库自身。
> 触发问题:sv-arco(调研 26,PR #56 刚起步)还没成形,**不用它,开发者现在能
> 正常开发吗?** PR #56 只新增 sv-arco 两 crate 与橱窗示例,不影响本篇结论。

## 0. 一句话判决

**能——限定在"内容区内的工具型单窗应用"**(表单/列表/设置页/CRUD)。组件缺失
不是瓶颈:手搓对话框 21 行、下拉菜单 12 行(实测,难点由 overlay/焦点原语代付)。
真正的墙在框架本体而非组件库层:**壳层指针事件面、视觉基建(字重/图标/网格)、
表单语义(disabled)、a11y 角色层、i18n**——这些 sv-arco 也补不了,arco 的富交互
组件(Select/Slider/Table)反而会先撞上它们。

## 1. 能开发什么:证据

### 1.1 组件自建成本(实测,全部来自可运行示例)

| 自建组件 | 行数 | 难点由谁代付 |
|---|---|---|
| 模态对话框 | 21(examples/overlay-demo/src/Dialog.svelte) | 焦点陷阱/Esc LIFO/关闭恢复焦点 = `<overlay modal>` 原语(sv-ui/src/overlay.rs 13 个测试) |
| 下拉菜单 | 12(OverlayDemo.svelte:14-27) | click-outside/锚定翻转/打开落焦第一项/方向键导航 = 框架行为 |
| Tooltip | 12(OverlayDemo.svelte:29-41) | 400ms 延时/防抖/不可命中/a11y 描述 = `sv_ui::tooltip` 原语 |
| 卡片(带插槽) | 13(showcase/src/Card.svelte) | `$props` + `children: sv_ui::Snippet` + `{@render}` |
| 步进器 | 14(showcase/src/Stepper.svelte) | `$bindable(i32)` 双向 + 默认值 |
| 可勾选行 | 18(showcase/src/TaskRow.svelte) | `Rc<dyn Fn()>` 回调 prop + 行内 `$state` |

照此模式可搭:Select/Tabs/Toast/分页/树/Radio 组,估 30-60 行/个(Radio 需手写
互斥,无 `bind:group`;Toast 走 overlay + `tasks::spawn` 定时)。

### 1.2 地基清单(有测试背书)

- **文本输入**:1088 行编辑内核(sv-ui/src/input.rs)——IME 预编辑/剪贴板/撤销
  重做/词跳/双击选词/多行 textarea;IME 中文**真机通过**(商用循环 C 项记录)。
- **弹层体系**:modal 焦点陷阱、Esc LIFO、锚定翻转、a11y 挂载(overlay.rs:184-320)。
- **列表**:keyed each 状态保留(keyed_each_preserves_row_state);`virtual_list`
  百万行 ≤34 节点(sv-ui/src/lib.rs:2062-2106),membench 实测 100k 行 3.2ms/帧。
- **滚动**:双轴 overflow + 滚轮路由 + 滚动条拖拽 + scroll-blit 后滚动帧 2.2ms
  (membench README:227-231)。
- **异步**:`{#await}{:then}{:catch}` + `tasks::spawn`(每任务一线程,完成拍醒事件
  循环,竞态取消有测试,tasks.rs:70-490);showcase 实用。
- **CSS 子集**"小而真":盒模型简写/伪类三件套(:hover/:active/:focus)/:root
  var()/嵌套/继承,报错指路质量高(style.rs:675-683 未知键会列全支持面)。
- **语法面**:Svelte 5 特性 44/77 ✅、计划内覆盖 84%(docs/SVELTE-SUPPORT.md)。
- 离屏 PNG + headless `dump()` 使无窗测试/CI 可行(三平台 CI 真跑)。

### 1.3 仓库外引入实测(本篇独家)

在仓库外建最小 crate,path 依赖(git dep 同机制)走 SFC 路线:**编译、无头运行、
点击断言全部通过**。但踩出两个事实:

1. **伞 crate 单依赖是期票**:codegen 发射 `::sv_ui`/`::sv_reactive` 绝对路径
   (codegen.rs:124-125 等 9 处),`svelte-rs` 的 re-export 救不了 extern prelude,
   报 E0433 ×9。实际依赖集 = `svelte-rs`(或不要)+ **`sv-ui` + `sv-reactive` 直接
   依赖** + `sv-compiler` 作 build-dep。`crates/svelte-rs/src/lib.rs:6-7` 的
   "cargo add svelte-rs 一条依赖拿全套"今天不成立,文档要改。
2. **依赖声明形态零文档**:7 篇指南只有"克隆仓库"形态,git/path 依赖示例缺失。

## 2. 要自己搭的(能绕,标成本)

- **主题**:无运行时 var()/@media,亮暗切换 = 逐属性接 signal;SFC `style:` 指令
  只认 11 字段(codegen.rs:1926-1941,border 色换肤只能条件类绕);宏前端
  `style(闭包)` 反而全字段——两前端能力不对等。成本线性于 UI 规模。
- **静态图片/图标**:无 `<image>`/SVG/icon-font;绕行 = 自带解码 crate →
  `PixelImage` → `register_frames(单帧)` → Animation 节点(vap-gift main.rs:70-84
  同款 hack)。emoji/Lottie 动画图标是权宜。
- **HTTP**:阻塞客户端(ureq/reqwest::blocking)包进 `spawn` 可用;**tokio 系
  async 客户端在 tasks 的 block_on 里会 panic(无 reactor)**,文档未提示。
- **大列表虚拟化**:`virtual_list` 纯 Rust API,模板无对应标签——列表页下沉 Rust 层。
- **组件规模化纪律**:全局平面命名空间,两个子目录同名 `Button.svelte` **静默互覆**
  (sv-compiler/lib.rs:242-282 无重名检查);每组件手动 `include!` 一行。
- 文件对话框/桌面通知:rfd/notify-rust 应用侧自接(调研 02 选型未落地)。
- 日志后端:壳层 16 处 `log::warn`(GPU 回退/丢帧)默认静默,不接 env_logger 即盲飞。

## 3. 绕不过的硬缺口(按杀伤力;组件库补不了)

### 3.1 壳层指针/键盘事件面
- **无节点级 pointer-move**(ViewNode 事件字段全集 sv-ui/src/lib.rs:473-479);
  down/up 未暴露给 .svelte → **Slider/拖拽排序/分栏器/列宽调整做不了**
  (settings-sfc 音量用 ±10 按钮就是旁证)。
- **壳层只处理左键**(sv-shell/lib.rs:906-1002 仅 MouseButton::Left)→ 右键菜单
  无事件源(overlay Anchor::Point 备好了没人喂);触屏 `WindowEvent::Touch` 零处理。
- **无文件拖放**(DroppedFile/HoveredFile 全仓零命中)。
- **键盘滚动整环缺失**:PageUp/PageDown 映射进来后无消费者(focus.rs:209-266
  导航段无滚动);Tab 落焦视口外无 scroll-into-view(grep 零命中)→ **超一屏的
  表单,键盘用户会把焦点按出视野且滚不回来**。
- click 在**按下**触发(lib.rs:988 Pressed 分支),不合桌面惯例(应抬起)。

### 3.2 窗口外壳
单窗口(App 单 win 槽,lib.rs:194/650);尺寸硬编码 480×400、无图标/最小尺寸
(lib.rs:654-657);`CloseRequested` 无条件退出(lib.rs:778,"未保存提示"做不了);
无原生菜单栏/托盘(muda/tray-icon 仅调研选型);快捷键注册表 thread-local 自认
多窗口需重构(shortcuts.rs:9)。

### 3.3 视觉基建
**无 font-weight/font-family 样式键**(text.rs:124-131 恒默认族)——全应用一个
字重、无等宽字体,排版层级只剩字号;无 %/vw/vh(编译错,style.rs:717-728)、无
grid、无 position:absolute/z-index(角标/徽标无出路);无 box-shadow/渐变/
transform(排 vello M2);`out:` 出场过渡是编译错(codegen.rs:1241,等 INERT);
无 text-overflow ellipsis。**这是"demo 能看、产品级 UI 出不来"的分界线。**

### 3.4 表单语义
无 disabled(`:disabled` 编译错,style.rs:237;focusable 只能 Rust API 关);
无密码掩码/readonly/maxlength(input.rs 零命中)。

### 3.5 a11y 深水区("能自建组件"在此维度不成立)
无角色覆盖 API(仅 label/description)→ 自建 Select/Tabs/Radio 在读屏里永远是
GenericContainer/Button,拼不出 listbox/tab/radiogroup;TextInput 无光标/选区上报
(a11y.rs:132-141 只 set_value,读屏用户盲打);无 live region(toast/校验错误
无声);a11y.rs:175 广告 SetScrollOffset 动作但 dispatch_action 无对应分支(虚广告)。

### 3.6 i18n
排版 locale 硬编码 zh-Hans(text.rs:106-113,日文应用汉字按简中字形渲染——ja
市场错字级问题);RTL 无 direction 键、零测试;叠加无 font-family = 等宽/图标
字体都进不来。

### 3.7 工程与稳定性
`.svelte` 内无 rust-analyzer(风险清单第 1 位;sv-lsp 只有诊断);无热重载(改 UI
全量 rustc);**用户回调 panic 无护栏**(事件派发无 catch_unwind)→ 一个 onclick
的 bug = 进程消失,从 Web 迁移者最易踩;crates.io 未发布(git dep 可用但要钉
rev);`$state` 表面语法未冻结(还会 breaking);多窗口布局缓存单槽 thread-local。

## 4. 两处订正(对既有叙述)

- **"CPU 后端撑不住文本密集界面"过重**:47ms 是 membench 强制全量的静止对照档;
  真实路径有整帧跳过(RenderOutcome::Skip)+ 脏矩形 + scroll-blit。正确结论:
  **resize/首帧会卡(shape 无缓存,text.rs:347-397),持续运行不卡**。
- onpointerenter/leave 已暴露给 .svelte(codegen.rs:768-769)——hover 回调可用,
  只是不带坐标,不改变"无 pointer-move → 拖拽类做不了"的结论。

## 5. 顺带核实的登记项(建议入 open-issues / 修复)

1. `class:` 条件类只搬 `:hover` 变体,`:active`/`:focus` 静默丢弃(codegen.rs:739-741,
   ClassEntry 三字段俱在)——按 bug 记。
2. a11y SetScrollOffset 虚广告(见 §3.5)。
3. click 应改抬起触发(§3.1)。
4. CSS-SUPPORT.md 至少 5 处状态列落后于代码(:focus/滚动条拖拽/overflow 按轴/
   底部统计 12 vs 头部 24/命名色 8 vs 66)。
5. 伞 crate "cargo add 一条依赖"期票文案(§1.3)+ git/path 依赖形态零文档。
6. tasks 的 block_on 不兼容 tokio async 客户端,文档未提示(§2)。
7. SVELTE-SUPPORT.md:102 残留 {#await} 过期 ⏳ 旧行(:280 已 ✅)。

## 6. 对 sv-arco(D 项)排期的启示

arco 组件清单里的富交互件会**先撞框架缺口**:Select/Dropdown(还好,overlay 够)、
**Slider(缺 pointer-move,做不了)**、Table(列宽拖拽做不了)、任何 disabled 态
(缺语义)、Tooltip 无障碍(缺角色)。PR #56 的 Button 已踩到 disabled/focus-visible
欠账,印证该判断。**建议**:A1 批继续做静态件(Tag/Badge/Divider/Alert),重交互件
排到壳层事件面(pointer-move/右键/disabled)补齐之后;或把"壳层事件面小补"插进
D 与首发之间。

## 7. 档位对照(接调研 19/27)

- **档 A(内部工具)今天成立**,且 IME/虚拟化/弹层 a11y 在同类早期框架里超配;
  适用画像 = 愿意 git dep + 读示例源码的早期采用者,中文场景,单窗工具。
- **档 B(单桌面商用)** 差的就是 §3 清单——其中 3.1/3.3/3.4 是明确的工程件
  (各自有界),3.5/3.6 是慢功夫,3.7 的 IDE/热重载是先例里最贵的长尾。
