# 多窗体(multi-window)落地规划

**状态**:P1(结构拆分)+ P2(frame_scheduler 广播)+ `run_multi`(启动即开 N 窗)
**已实现**(编译通过 + 110 项 sv-shell 测试全绿 + 离屏渲染通过);运行期动态开窗
(WindowHandle)与真窗口行为的现场手验待有显示器的环境。
**目标**:首发能开 N 个真窗口、各自一棵 UI 树,共享同一响应式运行时;跨窗联动零样板。

## 实现进度(2026-07-24)

- ✅ **P1 结构拆分**:`App` 拆成「事件循环级 `App`(backend/epoch/show_fps/proxy/
  windows/pending)+ 每窗 `Pane`(window/presenter/access/doc/layout/交互态/ime/
  damage/a11y/fps)」。7 个每窗辅助方法(paint/update_hover/sync_ime/click/
  push_access_tree/route_line_move/click_streak)迁入 `impl Pane`,4 个
  `ApplicationHandler` 方法(resumed/user_event/about_to_wait/window_event)成为
  `windows: HashMap<WindowId, Pane>` 的路由。**行为保持**:110 项 sv-shell 测试全绿。
- ✅ **P2 frame_scheduler 广播**:`set_frame_scheduler` 从「绑单窗 `request_redraw`」
  改为「`proxy.send_event(RedrawAll)` → 请求所有窗重绘」;各窗 `last_frame_key` 静止帧
  短路吸收开销。`Doc::set_on_mutate` 仍 per-Doc 精确重绘本窗(P4 精度优化已天然具备)。
- ✅ **`run_multi(Vec<(String, BuildFn)>)`**:启动即开 N 窗;`examples/multi-window`
  演示两窗共享一个计数器(`--png` 离屏渲一帧,窗口化需显示器)。关最后一个窗才退出。
- ⏳ **运行期动态开窗**(`WindowHandle::open`):`BuildFn` 是 `!Send`,不能走
  `EventLoopProxy`;需 thread_local BuildFn 队列 + 无载荷 `OpenWindow` 唤醒。首发不必需
  (`run_multi` 已够),留作后续。
- ⚠️ **现场手验待显示器**:P1/P2 是**按构造保持行为**的重构(逻辑逐段搬迁,非改写),
  编译 + 全测 + 离屏渲染证明了核心逻辑;但「真开两个窗、点一窗按钮看另一窗跟随」这类
  **窗口化行为**在 headless 环境验不了(`--png` 不走事件循环),须有显示器时手验。

## 0. 一句话结论

多窗体的难点**不在窗口管理**(winit 0.30 一个 `EventLoop` 天然支持多 `Window`),
而在**全局响应式运行时的重绘归属**:一次 `signal` 写该触发哪个/哪些窗口重绘。
好消息是**核心价值(跨窗响应式)此刻就成立**——运行时是线程内单例,一个 `state`
可被多个 `create_root` 里的 effect 订阅,写一次全部窗口精准更新,**不需要任何窗口间
通信代码**。这条已被 `sv-shell` 的 `shared_signal_drives_multiple_docs` 测试钉死
(两个 Doc 共享一个计数器,一处 +1 两处同步,离屏可验)。剩下的是**把单窗 `App`
拆成"循环级 + 每窗 Pane",并把 frame_scheduler 从"绑定唯一窗口"改成"广播到所有窗"**。

## 1. 现状:单窗 `App` 的耦合点(crates/sv-shell/src/lib.rs)

`App` 把三类状态揉在一起:

| 类别 | 字段 | 多窗后归属 |
|---|---|---|
| **循环级**(全应用一份) | `proxy` `backend` `epoch` `show_fps` `fps_*` | 留在 `App` |
| **每窗一份** | `win(WinState)` `doc` `_scope` `layout` `cursor` `hovered` `pressed` `drag_input` `drag_scroll` `last_click` `mods` `ime_allowed` `caret_blink_reset` `last_frame_key` `frame_buf` `damage` `a11y` | 迁入 `Pane` |
| **全局单例(线程内,非 App 字段)** | `sv_reactive::set_frame_scheduler` `sv_ui::tasks::set_waker` `sv_reactive::tick` `sv_ui::anim::pump` `sv_ui::set_clipboard` | 见 §3 |

**关键耦合**:`resumed` 里 `set_frame_scheduler(move || w.request_redraw())` 把
"响应式需要重绘"死绑到**那一个** window。`doc.set_on_mutate(move || w.request_redraw())`
是 per-Doc(天然可多份,无碍)。`tasks::set_waker` 发 `UserEvent::Wake`(全局一份即可)。

## 2. 目标结构

```
struct App {                      // 循环级
    proxy, backend, epoch, show_fps, fps_*,
    windows: HashMap<WindowId, Pane>,
    pending: Vec<PendingWindow>,  // open_window 请求,resumed/创建时兑现
}
struct Pane {                     // 每窗一份(= 旧 App 的每窗字段 + WinState)
    window, presenter, access,
    doc, _scope, layout, cursor, hovered, pressed,
    drag_input, drag_scroll, last_click, mods,
    ime_allowed, caret_blink_reset, last_frame_key,
    frame_buf, damage, a11y,
}
```

- `paint(&mut self)` → `Pane::paint(&mut self, backend, epoch, show_fps…)`(循环级只读项传参)。
- `window_event(&mut self, _, id, event)` → `self.windows.get_mut(&id)` 路由后调 `pane` 方法。
- `about_to_wait`:遍历各 `pane` 求"最早的光标翻转唤醒",取 min 设 `WaitUntil`。

## 3. frame_scheduler 解耦(**本重构的核心一步**)

单窗:`set_frame_scheduler(|| window.request_redraw())`。
多窗:响应式冲刷不知道"哪个窗脏"——effect 可能改任意 Doc。**最简且正确**:

```rust
// 循环级设一次:唤醒事件循环,请求所有窗重绘
let proxy = self.proxy.clone();
sv_reactive::set_frame_scheduler(move || { let _ = proxy.send_event(UserEvent::RedrawAll); });
// UserEvent::RedrawAll → for pane in windows.values() { pane.window.request_redraw(); }
```

- **正确性**:保守地全窗请求重绘;每个 `Pane::paint` 用 `last_frame_key`(版本+尺寸+
  scale+闪烁相)短路静止帧——**没变的窗当帧直接跳过绘制**,只多一次 version 比较,
  不多画一像素。这是现成机制,天然吸收"广播"的浪费。
- **精度优化(可选,后续)**:`Doc::set_on_mutate` 是 per-Doc 的,可为每个 Doc 记
  "本帧是否被 mutate",RedrawAll 时只 request 脏 Doc 的窗。首发不需要——短路已够。
- **动画**:`anim::pump` / `tasks::pump` 是全局的,循环级 `paint` 前跑一次即可
  (所有窗共享同一动画时钟),然后各窗按需重绘。

## 4. 对外 API

```rust
pub fn run_app(title, build) -> Result<()>              // 不变:开首个窗
pub fn run_multi(windows: Vec<(String, BuildFn)>) -> Result<()>   // 启动即开 N 窗
// 运行期再开:build 闭包里拿到的句柄
pub struct WindowHandle { proxy: EventLoopProxy<UserEvent> }
impl WindowHandle { pub fn open(&self, title, build: BuildFn); }  // 发 UserEvent::OpenWindow
// UserEvent::OpenWindow(title, build) → App 在 resumed/user_event 里 create_window + 建 Pane
```

- 每个新窗**新建一个 `Doc`** + 一个 `create_root`(独立所有权树);共享的 `state`
  从外层 root 或自由 signal 来(见 §0 测试形态)。
- `open_window` 的 `build` 可捕获既有 signal → 新窗直接联动老窗,零额外接线。

## 5. 增量 PR 顺序(每步可独立编译 + 单窗行为不变)

1. **P1 结构拆分(零行为变更)**:抽出 `Pane`,`App.windows` 只放一个;`paint`/
   `window_event`/`about_to_wait` 改走 `windows.get_mut(&id)`。frame_scheduler 暂仍绑
   首窗。**验收**:单窗示例(counter/showcase)行为逐帧不变。
2. **P2 frame_scheduler 广播 + RedrawAll**:改 §3;仍只有一个窗,行为不变(广播到 1 个窗)。
3. **P3 open_window / run_multi + WindowHandle**:真正开第二个窗;新增 `examples/multi-window`
   (两个窗共享一个计数器,演示 §0)。
4. **P4 精度优化(可选)**:脏 Doc 定向重绘;每窗独立 IME/焦点/快捷键作用域复核。

## 6. 已知风险与验证缺口

- **⚠ 无显示器无法验窗口化行为**:当前开发/CI 环境是 headless,`--png` 离屏路径不走
  事件循环。P1–P3 的**窗口管理正确性(真开窗、事件路由、重绘)只能验编译,不能验行为**。
  实现这几步**必须在有显示器的环境手验**(开两个窗、点一个窗的按钮看另一个窗是否跟随)。
  故本规划先交付**可离线证明的核心**(§0 测试)+ 完整规格,窗口管理留作有显示器时的专项。
- **每窗 a11y adapter**:`accesskit_winit::Adapter` 是 per-window 的(已在 `WinState`),
  拆进 `Pane` 天然正确;`UserEvent::Access` 带 `window_id`,按 §2 路由。
- **clipboard / waker 全局单例**:全应用一份即可,不随窗口数变——保持循环级设置。
- **焦点/快捷键作用域**:`sv_ui` 的焦点与快捷键目前是**全局**的(单窗假设)。多窗下
  应是"每窗一个焦点环 / 快捷键表"。P1 拆 `Pane` 时焦点态随之每窗一份;快捷键分发
  (`shortcuts`)若是全局 thread_local,需在 P3 前确认是否要每窗隔离(记为 P3 前置检查)。

## 7. 交叉引用

- 核心证明:`crates/sv-shell/src/lib.rs::tests::shared_signal_drives_multiple_docs`
- 响应式单线程模型:CLAUDE.md「响应式是单线程模型(thread-local runtime,句柄 Copy + !Send)」
- 帧调度语义:docs/DESIGN.md ADR-6
