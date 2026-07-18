# 25 · 弹层体系与发布工程:overlay 分层设计 + 首发/打包落地方案(档 B 收尾两件)

> 2026-07-18。方法:业界先例联网核实(Slint 博客/文档、egui/iced/taffy docs.rs、
> Svelte issue、cargo-dist/packager/bundle 仓库 API、crates.io 名称实查),
> 方案对准本仓库 sv-ui/sv-shell/sv-compiler 源码逐文件给改动点;人周为单人粗估并标注置信度。

## 0. 一句话裁决

弹层**不做通用 z-index**,做 egui 式**离散层 + 注册序**:场景树加 overlay 根列表,
布局后把 overlay 的 `Placed` 追加在基础层之后——现有"树序绘制 + `rev()` 命中"
**零改动**即得正确遮挡与优先命中;锚定自造(taffy 的 position:absolute 管不了
"锚到触发节点+越界翻转",业界也都是布局后处理);`.sv` 侧不发明模板语法,走
`<overlay>` 内建元素 + 标准组件自举。发布工程:改名裁决一周内做完(**`sv` 在
crates.io 已被 Bitcoin SV 库占用,单字名后手已失效**;`sv-*` 六名仍全空闲),
应用打包选 cargo-packager(GUI 主力)+ cargo-dist(CI 流水线参照,2026-07 仍活跃)。

## 1. 业界先例(均联网核实,查不到的明说)

### 1.1 弹层

- **Slint(“desktop-ready”系列 + 当前文档)**:模态新 API(window-modal /
  application-modal,"临时阻断对应用其余部分的交互");popup 计划从"主窗口内绘制"
  升级为"窗口系统可见的真窗口"(定位 API 映射 Wayland 协议);tooltip 原生样式与
  完全自定义双轨;系统托盘四平台。`PopupWindow` 元素:`show()`/`close()`/`is-open`,
  **close-policy 三值**:`close-on-click`、`close-on-click-outside`(两者都含 Esc)、
  `no-auto-close`;定位是 x/y 绝对坐标(锚定由调用方算);限制:弹窗内部属性外部
  不可访问。`ContextMenuArea`:右键/Menu 键/`show(Point)` 打开,MenuItem/子菜单/
  分隔符齐全。v1.13.0 发布于 2025-09-03(GitHub Releases API 核实;逐条 changelog
  抓取失败,见文末未核清单)。
- **egui**:`layers::Order` 五层枚举——Background / Middle(普通可点击换序窗口)/
  **Foreground("popups、menus 等,永远画在窗口之上")** / **Tooltip(最上层且
  不可交互)** / Debug。tooltip 延时在 `style::Interaction`:`tooltip_delay`
  (鼠标停住后延时秒数)+ `tooltip_grace_time`(刚看完一个 tooltip 再悬停下一个
  免延时)——**离散层 + 注册序,无通用 z-index**,是本方案的直接原型。
- **iced**:`iced_core::overlay` 模块,`Overlay` trait + `Group`/`Nested` 叠层,
  在基础 widget 树之上独立 layout/绘制/命中——COSMIC 的下拉/菜单走这条通道。
- **taffy**:`style::Position` 仅 `Relative`/`Absolute` 两值;Absolute 相对最近
  positioned 祖先 + inset 定位、不占流内空间。**流内绝对定位可直通 taffy,但弹层
  锚定(锚到触发节点 rect、越界翻转、窗口 clamp)不在其职责内**。
- **Svelte**:无内置弹层/传送门;portal 提案 sveltejs/svelte#7082
  "[feature] Add Portals to Svelte" 至今 open,生态靠 svelte-portal/bits-ui 等
  第三方。→ 裁决依据:`.sv` 不发明 `{#teleport}`/`<dialog>` 语法。
- **Floem / Blitz / Masonry / COSMIC 细节**:未核实(本次检索预算耗尽),不引用。

### 1.2 发布工程

- **cargo-dist**(axodotdev/cargo-dist):活跃——push 2026-07-17、v0.32.0
  (2026-05-21)、2070 star;产物:shell/powershell/msi/homebrew/npm 安装器 +
  updater + 校验和 + GitHub attestations + **Windows 代码签名**;自动生成整套
  release CI。采用背书:astral-sh/uv 仓库根有 `dist-workspace.toml`(API 实查)。
  macOS 公证支持程度与 axo 公司经营传闻未核实。
- **cargo-packager**(crabnebula-dev):活跃——push 2026-07-16、v0.11.8
  (2025-11-27);Tauri 系 GUI 打包(nsis/wix/dmg/appimage/deb)+ updater。
- **cargo-bundle**:存活但节奏慢(push 2026-05-30)。
- **crates.io 名称实查(2026-07-18,API 逐个查)**:`sv` **被占**(Bitcoin SV 库,
  2.8 万下载);`svelte`、`svelte-rs`、`sv-ui`、`sv-reactive`、`sv-shell`、
  `sv-compiler`、`sv-macro`、`sv-build`、`svelte-ui`、`svello`、`verve` 全空闲;
  `runa`/`sylph` 被占。

## 2. 方案设计 · 弹层(对准本仓库代码)

### 2.1 现状事实(源码核对)

- **绘制顺序=树序**:`render.rs` 的 `place()` 前序遍历产出 `Vec<Placed>`,
  `paint_tree` 顺序绘制;命中/悬停用 `.rev()` 取最上层(`hit_click_target`、
  `lib.rs update_hover`)。全仓无任何 z 概念。
- `ElementKind` 仅 View/Text/Button/Checkbox(sv-ui lib.rs);`window_event` 只有
  CursorMoved + 左键 MouseInput 分支,无 KeyboardInput(sv-shell lib.rs)。
- **可复用机制已在**:`mount`/`MountHandle`(独立 root 作用域,弹层生命周期现成);
  `tasks` 背景桥 + waker(tooltip 延时现成,事件循环零改);`layout_tree_cached`
  版本键控(overlay 属于 Doc,失效免费);`virtual_list` 无模板语法也可用的先例
  (`.sv` 语法可后置)。

### 2.2 架构裁决:离散层 + 注册序,不做通用 z-index

三案对比:(a) `ViewNode` 加 `z_index: f32` → 全树排序破坏"树序绘制/命中一致"
的架构简单性,且 keyed each 行间穿插语义成谜;(b) **独立 overlay 根列表**
(egui Order 同构):Base → Popup(注册序=打开序)→ Tooltip 三层;(c) 等 taffy
position:absolute → 只解决流内绝对定位,锚定仍要自造。**裁决 (b)**;`z-index`
属性进 ADR-8"永不支持清单"(层间顺序=注册序,层内无穿插,与 egui 同口径)。

### 2.3 类型与签名(sv-ui lib.rs)

```rust
// DocumentInner 增字段(注册序即层内叠序)
pub overlays: Vec<OverlayEntry>,

pub struct OverlayEntry {
    pub root: ViewId,              // 独立子树根,不挂在 doc.root 下(即传送门语义)
    pub anchor: Anchor,
    pub layer: OverlayLayer,       // Popup | Tooltip(Tooltip 恒最后画、不可命中)
    pub modal: bool,               // 真:命中测试跳过本 overlay 之下的一切
    pub close: CloseBehavior,      // 对齐 Slint close-policy 三值;Esc 恒生效
    pub on_dismiss: Option<Rc<dyn Fn()>>, // shell 检出关闭手势时调用(回写 signal)
}
pub enum Anchor { Node { id: ViewId, side: Side, gap: f32 }, Point(f32, f32), WindowCenter }
pub enum Side { Below, Above, Left, Right }
pub enum CloseBehavior { OnClickOutside, OnAnyClick, None }
```

新绑定原语(双前端共同编译目标,与 `if_block` 同构、与 `mount` 同实现骨架):

```rust
pub fn overlay_block(
    doc: &Doc,
    open: impl Fn() -> bool + 'static,          // derived 化,相等剪枝
    anchor: impl Fn() -> Anchor + 'static,
    opts: OverlayOpts,                          // layer/modal/close
    build: impl Fn(&Doc, ViewId) + 'static,
)
```

open 翻真 → `create_root` 建独立作用域 + 建 overlay 子树 + 注册 entry;翻假或外层
卸载 → dispose + 摘 entry(`MountHandle` 现成)。**on_dismiss 不直接关弹层,只回写
signal → open 翻假走同一条路**——单一数据源,与 `bind:checked` 的双向绑定同型。
`dump()` 需同步输出 overlay 段,否则金样测试写不了(见风险 3)。

### 2.4 布局/绘制/命中(sv-shell,本期零 Painter 改动)

`layout_tree` 尾部追加一段:对每个 entry → `measure` overlay 子树(无约束)→ 从
基础层 `placed` 里查锚点节点 rect 算原点(`Side::Below` = anchor.bottom+gap,
放不下翻转 Above,最后 clamp 进窗口;`WindowCenter` 即对话框)→ `place()` 结果
**追加进同一 `Vec<Placed>`**,并记录每个 entry 的区间 `[start, end)`(随
`Vec<Placed>` 一起返回给 App)。

- **绘制:`paint_tree` 一行不改**——overlay 的 Placed 天然在后、后画即在上。
  模态遮罩 = overlay 子树自带一个全窗 View(bg 半透明黑),普通节点普通画。
- **命中:核心不改**——`rev()` 天然先命中 overlay。模态阻断:App 持 entry 区间表,
  最上层 modal entry 的 `start` 之前的 Placed 整体跳过(比"遮罩节点吞事件"可靠:
  hover 命中只认带 handler 的节点,遮罩没 handler 会穿透,区间法一并堵住)。
- Painter/`PaintCmd` 零新增动词。菜单长列表滚动需要 clip 动词——归滚动体系
  (调研 19 P0 第 3 项),刻意不进本期;box-shadow 走 `caps.blur`(vello 已报
  true),CPU 后端跳过。

### 2.5 事件循环分支(sv-shell `window_event`)

- `MouseInput Pressed`:派发前先查最上层 auto-close entry——点在其区间外 →
  调 `on_dismiss`;`OnClickOutside` 时该次点击被吞(菜单点选项则正常下传)。
- **`KeyboardInput` 新分支**(可先于完整焦点链落地最小版):Esc → 最上层
  auto-close entry 的 on_dismiss(LIFO,嵌套菜单逐层关);Tab/方向键转发焦点链
  (调研 20,同批);文本/IME 不在本期。
- **tooltip 延时**:悬停进入 → `tasks::spawn(sleep(delay))`,完成回调核对
  "悬停代数计数器"未变才置 open signal——背景桥 + waker 现成,事件循环零改;
  egui 的 grace_time 优化记 P2。
- **焦点陷阱**(依赖调研 20):modal 打开时记住原焦点,Tab 环限定在 entry 区间内,
  关闭时恢复。焦点链未就绪时档 A 先交付"遮罩 + Esc"简版。

### 2.6 `.sv` 语法裁决(sv-compiler)

Svelte 无内置弹层(§1.1 已证),**不发明块语法**,分三层:

1. **内建元素 `<overlay>`**:`template.rs` Tag 枚举加 `Overlay`(现仅
   view/text/button/checkbox,L290-293),`style.rs` `ELEMENT_NAMES` 同步;
   `codegen.rs` 元素分派(L512-515 的 Tag 映射处)对 `Tag::Overlay` 生成
   `overlay_block` 调用。属性:`open={表达式}`(必填)、
   `anchor="below|above|left|right|center"`、`gap`、`modal`、
   `close="outside|any|none"`、`ondismiss={闭包}`;children 即 build 闭包。
2. **标准组件自举**:Dialog.sv / Menu.sv / Tooltip.sv 用 `.sv` 写(吃自家狗粮);
   `open` 声明为 `$bindable` → 调用侧 `<Dialog bind:open={show}>`——组件 prop 的
   `bind:` 归一化已支持(codegen.rs L994)。
3. **`on:keydown` 不顺手加**:codegen.rs L930 目前明确拒绝键盘事件并提示
   "待焦点链"——保持该口径,键盘事件属性随键盘通道工单落地,避免语法先行。
   view! 宏前端 v0 不加语法,用户直接调 `overlay_block`(与 `virtual_list` 现状
   一致);M1 双前端合并时一次补齐。

## 3. 方案设计 · 发布工程

### 3.1 改名裁决(建议一周内出 ADR-10)

事实:README 已标 working name `sv`,但 **crates.io `sv` 已被占**(§1.2)——单字
后手失效,`sv-*` 前缀仍全空闲。`svelte`/`svelte-rs` 虽空闲,但名称混淆/商标风险
(Svelte 商标状态未核实,保守按有险处理;Rust 生态惯例亦反对 `-rs` 蹭名)。
流程:候选 ≥5(已核空闲:svello、verve、sv-* 前缀、svelte-ui——最后两者仍含
sv/svelte 联想,需决策会定倾向)→ 商标粗查 + GitHub org 查重(未做)→ ADR-10
记录裁决与迁移表(crate 名/repo/文档/CI badge)→ **以真实 0.1.0 发布占名**
(crates.io 政策反对空壳占坑,发真实版本合规)。改名必须先于首发——名字是唯一
改不动的 API。

### 3.2 crates.io 首发清单

- **依赖序发布**:sv-reactive → sv-ui → sv-macro / sv-compiler(+ `<name>-build`
  门面,调研 06 slint-build 三件套已定案)→ sv-shell → 伞 crate `<name>`
  (re-export + prelude)。examples 不发布;组件库日后按 sqlx 模式预生成 vendor。
- **元数据补全**:workspace.package 现仅 version/edition/license;补
  description/repository/keywords/categories/readme/`rust-version`
  (edition 2024 ⇒ MSRV ≥ 1.85,写明并进 CI)。
- **semver 政策**:0.x 期"minor = breaking、patch = 兼容"并写进 README;三类
  已排期 breaking(M1 双前端合并、`on:` 收敛、ADR-6 时序)清完再谈 1.0。
  CHANGELOG 按 Keep a Changelog,每 crate 一份或 workspace 单份带 crate 前缀。
- **CI 门禁**:cargo-semver-checks(发版前跑,0.x 规则适用)、cargo-audit 或
  cargo-deny(含许可证审计)、clippy `-D warnings` 转阻塞(调研 19 估 1–2 天)。

### 3.3 应用打包分发选型

库 ≠ 应用:库走 crates.io 即可;打包故事是给用户 App 与自家 showcase 的。
**裁决:cargo-packager 为主**(GUI 场景 nsis/wix/dmg/appimage/deb 全、CrabNebula
活跃维护),**cargo-dist 作 CI 发布流水线参照**(安装器 + updater + attestation
全自动、uv 级采用背书,但面向 CLI 分发);cargo-bundle 不选(功能面小、节奏慢)。
三平台故事:Windows = Authenticode(证书服务选型未核实);macOS = Developer ID +
公证(cargo-packager 自动化程度未核实);Linux = AppImage 起步、Flatpak 进阶
(沙箱与自绘 IME 的相性列开放问题)。v0 交付:showcase 三平台**未签名**产物的
CI 工作流,签名/公证是组织级账号事务,工程侧只做到钩子就位。

### 3.4 去 panic 审计与错误类型化

核实现状:非测试代码 expect——**sv-shell 8 处**(lib.rs 6:softbuffer
context/surface/resize/buffer/present + create_window;font.rs 1;render.rs 1
pixmap),与调研 19 点名一致;sv-reactive 的 panic 部分是语义设计(derived 写
保护 = Svelte state_unsafe_mutation),**保留并文档化为"类型化 panic"**。方案:
sv-shell 定义 `ShellError` 枚举,`run_app` 已返 Result 直接串;帧内 present/resize
失败降级为丢帧 + log 而非崩;`render_frame` pixmap 失败回退 1×1。范围不含
sv-compiler(编译期 panic 属诊断质量,另一条线)。

## 4. 分步落地(验收 = 测试名;人周为单人粗估)

| 步 | 内容 | 验收 | 人周(置信) |
|---|---|---|---|
| O1 | overlay 层:OverlayEntry/overlay_block + 锚定/翻转/clamp + Placed 追加 + dump 可见 | `overlay_paints_after_base`(金样)/`anchor_below_flips_when_clipped`/`hit_prefers_overlay` | 2–3(高) |
| O2 | 关闭策略 + 事件分支:click-outside、Esc(KeyboardInput 最小分支)、on_dismiss 回写 | `click_outside_dismisses_topmost`/`esc_closes_in_lifo_order` | 1–2(高) |
| O3 | 对话框:modal 区间阻断 + 遮罩 + 焦点陷阱(依赖调研 20) | `modal_blocks_base_hit_and_hover`/`modal_tab_cycles_inside`/`focus_restored_on_close` | 1–2(中) |
| O4 | 菜单/下拉:方向键导航、子菜单侧向锚定、hover 展开 | `menu_arrow_navigation`/`submenu_anchors_right`/`menuitem_click_dismisses_chain` | 2–3(中) |
| O5 | tooltip:hover 延时(tasks 桥 + 代数计数)+ Tooltip 层不可命中 | `tooltip_shows_after_delay_headless`/`tooltip_never_hit_target` | 1(高) |
| O6 | `.sv`:`<overlay>` 元素 + Dialog/Menu/Tooltip 组件自举 + todo-sfc 确认框 demo | `sv_overlay_codegen`/`dialog_sfc_bind_open_roundtrip` | 1–2(高) |
| R1 | 改名 ADR-10:候选核查表 + 商标粗查 + org 查重 | ADR 合入、迁移表齐 | 0.5(高,法务除外) |
| R2 | CI 门禁:semver-checks / audit(或 deny)/ clippy 阻塞 + MSRV | 工作流 `ci_semver_checks` 全绿 | 1(高) |
| R3 | 去 panic:ShellError + 帧内降级 + reactive 文档化 | `shell_no_expect_outside_tests`(grep 门禁)/`present_failure_drops_frame` | 1–2(高) |
| R4 | 首发:六 crate 依赖序 + 元数据 + CHANGELOG + 伞 crate | `cargo publish --dry-run` 全绿、docs.rs 构建过 | 0.5–1(高) |
| R5 | 打包:showcase 三平台 cargo-packager 工作流(未签名) | CI 产 appimage+dmg+nsis 三产物 | 1–2(中) |

弹层小计 8–13 人周,发布小计 4–6.5 人周;两线可并行(发布线多为工程配置)。

## 5. 风险与开放问题

1. O3/O4 硬依赖键盘通道 + 焦点链(调研 20 同批)——若延期,弹层只能交付
   "鼠标版",档 B 不达标。
2. 菜单长列表需要 clip + 滚轮,本方案刻意不含;滚动体系应先行或同批
   (调研 19 依赖序第 3 项),否则下拉超过窗口即穿帮。
3. overlay 子树游离于 doc.root 之外,`dump()`/现有遍历工具默认不可见——O1 必须
   同步改 dump,否则金样测试自己都写不了。
4. 每帧重锚定:锚点 rect 查询是对 placed 的线性扫——先接受(overlay 数 ≪ 节点数),
   ADR-9 增量布局落地时并入 dirty 子树。
5. 签名/公证的账号税(Apple Developer 年费、Win 证书)是组织级事务;Flatpak 沙箱
   与自绘 surface IME 的相性未验证。
6. 改名只做了 registry 占用实查,**商标法务未做**;svello 类候选仍隐含 svelte
   联想,需一次决策会拍板。
7. cargo-dist 背后 axo 公司可持续性传闻未核实——仓库 2026 仍活跃,且仅作参照非硬依赖。

## 6. 结论:最小可商用切片

- **档 A(内部工具)**:O1 + O2 + O5 + O3 的"遮罩 + Esc"简版(无焦点陷阱)+
  R3(sv-shell 不当场崩)。此时设置面板 + 确认框 + tooltip 可用,菜单缓行;
  无需 crates.io(git 依赖即可)。合计约 5–8 人周。
- **档 B(商用)**:上述全部 + O3 焦点陷阱 + O4 菜单 + O6 语法面,以及 R1–R4 全量
  (改名先行 → 首发 → semver/CHANGELOG/CI 门禁)+ R5 至少一平台出签名产物。
  两线合计约 12–19 人周,前置条件:调研 20 焦点链与滚动体系落地。

## 出处

- https://slint.dev/blog/making-slint-desktop-ready ·
  https://docs.slint.dev/latest/docs/slint/reference/window/popupwindow/ ·
  https://docs.slint.dev/latest/docs/slint/reference/window/contextmenuarea/
- https://docs.rs/egui/latest/egui/layers/enum.Order.html ·
  https://docs.rs/egui/latest/egui/style/struct.Interaction.html ·
  https://docs.rs/iced_core/latest/iced_core/overlay/index.html ·
  https://docs.rs/taffy/latest/taffy/style/enum.Position.html
- https://github.com/sveltejs/svelte/issues/7082(Portal 提案,open)
- https://github.com/axodotdev/cargo-dist(v0.32.0 2026-05)·
  https://axodotdev.github.io/cargo-dist/book/introduction.html ·
  https://github.com/crabnebula-dev/cargo-packager ·
  https://github.com/burtonageo/cargo-bundle · astral-sh/uv(dist-workspace.toml)
- crates.io API 名称实查(/api/v1/crates/{name},2026-07-18)

## 未能核实(诚实清单)

- Slint v1.13.0 的逐条 changelog(release 页多次抓取失败;desktop-ready 博客与
  当前文档已核,"1.13 系统性补弹层"沿用调研 19 表述)。
- cargo-dist 对 macOS 公证的支持;axo 公司经营状态;cargo-packager 对
  notarytool 的自动化程度;Windows 签名服务(Trusted Signing/SignPath)2026 现状。
- Floem/Blitz/Masonry/COSMIC(iced 之上)的弹层实现细节。
- "Svelte" 商标注册状态与归属;候选名的 GitHub org 占用;egui tooltip 延时默认值。
