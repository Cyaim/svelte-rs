# 更新日志 / Changelog

格式沿用 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/),
版本号遵循下述 **0.x 语义化政策**。

## 版本政策(0.x 期)

尚未 1.0,API 会变。本仓库对 0.x 采用 Cargo 的 0.x 兼容规则并额外承诺:

- **minor 号(0.X.0)= 破坏性变更**;**patch 号(0.0.X)= 向后兼容**。
- 破坏性变更必须在本文件 `Changed` / `Removed` 段落写明**迁移方式**。
- 每次发版前跑 `cargo semver-checks`,漏报的破坏性变更按 bug 处理。
- 三类**已排期**的破坏性变更清完才会谈 1.0(见 `docs/DESIGN.md` §5):
  双前端内核合并、`on:` 事件语法收敛、帧调度语义(ADR-6)。

工作区所有 crate 同版本号发布(sv-reactive / sv-ui / sv-macro / sv-compiler /
sv-shell),按依赖序推送;`examples/` 不发布。

## [未发布]

### 新增

- **R3 弹层体系**:离散层(Base→Popup→Tooltip)+ `overlay_block` 原语 +
  `.sv` 的 `<overlay>` 内建元素;锚定四侧 + 越界翻转、关闭策略三值 + Esc LIFO、
  模态区间阻断与焦点陷阱、tooltip 悬停延时、Popup 内方向键导航。
- **R3 无障碍**:AccessKit 接入(`build_tree_update` 纯函数树映射 +
  accesskit_winit 懒激活适配器 + 动作回派)。
- **R3 文本栈**:Parley 0.11 接管 shaping/折行/光标几何,fontique 系统字体
  发现与 script fallback(CJK/Latin 混排不再出方框)。
- **文本输入编辑手势**:词跳与删词(Ctrl/⌥+←/→、Ctrl+Backspace/Delete)、
  拖拽选择、双击选词、三击全选、撤销重做(Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z)。
- **性能基准 CI 化**:membench 两场景(3k 全量 / 100k 虚拟化)p99 帧预算护栏。
- **发布工程**:`ShellError` 类型化错误、cargo-deny 依赖审计、MSRV 1.88 构建道、
  clippy `-D warnings` 阻塞门禁、发布演练(依赖序 `cargo package`)。

### 变更

- **keyed `{#each}` 行内容原地更新(ADR-7)**:行改持 `Signal<T>`,同 key 换内容
  不再显示旧数据(顺带修掉"列表一变就把所有行作用域悄悄销毁"的 bug);顺序未变
  时零树改动。迁移:项类型需 `Clone + PartialEq`;keyed 绑定名须为单个标识符
  (解构改用 `{@const}`);`sv_ui::each_block_keyed` 的 row 回调签名改为
  `Fn(&Doc, ViewId, Signal<T>)`。
- 新增 `sv_reactive::with_owner`:在指定作用域下建节点(effect 内建"活过重跑"的
  子作用域,同时保住 context 沿 owner 链可达)。

- **双前端共享 codegen 内核(ADR-2 无悔三步 ①)**:新增 `sv_compiler::emit`
  作为两个前端对 sv-ui 的唯一发射口;`sv-macro` 现依赖 `sv-compiler`。
  对用户无行为变化(两条路线生成的代码形状不变)。

- **帧调度(ADR-6,语义 breaking)**:开窗应用的 signal 写入不再当场跑 effect,
  改为攒到帧边界由渲染壳统一冲刷(一次事件连写 N 次 = 一帧一轮)。迁移:
  需要立刻看到结果的地方调 `sv_reactive::tick()`。离屏渲染与测试路径不受影响。

- `sv_shell::caret_x` / `caret_index_at` 迁至 `text` 模块并改签名:不再收
  `&FontRef` 参数(几何由 Parley 提供)。迁移:去掉第一个参数即可。
- `sv_shell::run_app` 的错误路径改为返回 [`ShellError`],窗口/呈现层不再 panic;
  帧内 present/resize 失败降级为丢帧。
- 长文本溢出时输入框的点击定位改为与绘制同源(此前忽略横向滚移,点击会偏)。

### 移除

- `sv_shell::ui_font_handle` 与内置字体探测(`font.rs`):字体一律经 fontique
  发现。`FontHandle` 现由 `sv_shell::text` 导出,保留键 0 的语义随之消失。

<!-- 首个公开版本发布后,这里开始出现 [0.1.0] - YYYY-MM-DD 等条目 -->

---

**EN** — This project keeps a Chinese-first changelog; entries follow
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). During 0.x, **a minor
bump means breaking changes** and a patch bump is backwards compatible; every
breaking change documents its migration path above.
