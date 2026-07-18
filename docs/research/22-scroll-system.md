# 22 · 滚动体系落地方案:容器 + 滚轮 + 裁剪 + 滚动条,与 virtual_list 合流

> 2026-07-18。方法:六个框架的滚动实现联网核实(egui/iced/Masonry/Slint/Flutter/
> floem,逐条给出处 URL)+ 本仓库五个代码位逐行核对(paint.rs/render.rs/
> sv-shell lib.rs/sv-ui lib.rs/sv-compiler codegen.rs),产出可直接开工的分步清单。
> 对应调研 19 P0 缺口第 3 项("全仓 MouseWheel 零命中;Painter 连 clip 都没有")。

## 0. 一句话结论

**滚动做成 View 的 `overflow` 属性而非新 ElementKind;offset 是节点内状态
(真源在树上,像 `checked`),Signal 只作可选桥;Painter 加 `push_clip/pop_clip`
两个动词(tiny-skia 侧 v0 走手动矩形裁剪,vello 侧 push_layer 现成);滚动条由
shell 合成绘制不入树;virtual_list 的 `offset: Signal<usize>` 正好是现成接缝,
加一个"虚拟内容尺寸覆盖"即接通真实滚轮输入。** 全程约 5–8 人周,档 A 切片 4–5 人周。

## 1. 业界先例(联网核实)

| 框架 | 滚动容器 | offset 存放 | 裁剪 | 滚动条 | 虚拟化 |
|---|---|---|---|---|---|
| egui | `ScrollArea` | 框架内部 state,`scroll_offset()` 可程序设 | clip rect | 自绘,`scroll_bar_visibility()` | `show_rows()`/`show_viewport()` 只渲染可见范围 |
| iced | `Scrollable` | widget 内部,`snap_to(Id, RelativeOffset) -> Task` 程序驱动 | viewport | `Scrollbar`/`Rail`/`Scroller` 三件套 | (COSMIC 生态另做) |
| Masonry | `Portal` | `viewport_pos: Point`,`set_viewport_pos`/`pan_viewport_by`/`pan_viewport_to` | 裁剪子部件 | `horizontal/vertical_scrollbar_mut()` 内置 | — |
| Slint | `Flickable`(低层元素,ScrollView 的地基) | `viewport-x/y`(负值)+ `viewport-width/height` | 元素自动裁剪 | **不带滚动条**,由包装 widget 提供 | — |
| Flutter | `Viewport` | `offset: ViewportOffset`("滚动 = 变 offset") | `clipBehavior` 默认 `Clip.hardEdge` | 独立 Scrollbar widget | sliver 懒布局 + `scrollCacheExtent` |
| floem | `scroll`/`Scroll` | 内部 | 内部 | 内部 | `virtual_stack`:"lazily loads the items as they appear in a scroll view" |

跨框架公因子(即我们该抄的作业):
1. **offset 是滚动容器自己的一份状态**,程序可写(egui `scroll_offset`、iced
   `snap_to`、Masonry `set_viewport_pos`、Flutter `ViewportOffset`)——不是全局的,
   也不是每次重算的;
2. **裁剪默认硬边矩形**(Flutter `Clip.hardEdge`);
3. **虚拟化 = 滚动容器把"可见范围"交给回调**(egui `show_rows`、floem
   `virtual_stack`、Flutter sliver),与我们 `virtual_list` 的槽位模型同构;
4. **滚动条拖拽依赖指针捕获**:Masonry `EventCtx::capture_pointer`/
   `release_pointer`——捕获后指针移出部件仍收事件,`Up`/`Cancel` 自动释放;
5. **滚轮双通道**:winit `MouseScrollDelta::LineDelta(f32, f32)`(行/列数)与
   `PixelDelta(PhysicalPosition<f64>)`(触摸板等设备,物理像素);正值 = 内容向
   右/下移动;
6. **平滑滚动是后置增强**:egui `InputState::smooth_scroll_delta`("smoothed
   over a few frames",ScrollArea 消费后清零)+ `animated()`;Slint Flickable 用
   "8 逻辑像素内 / 500ms" 手势判定区分点击与拖动滚动。

后端裁剪原语核实:vello `Scene::push_layer(clip_style, blend, alpha, transform,
clip: &impl Shape)` 与 `push_clip_layer(clip_style, transform, clip)`(文档注明
后者 blend 语义尚不正确,见 issue #1198——纯裁剪用 push_layer 保守)、
`pop_layer()`;tiny-skia **没有矩形 clip API**,唯一机制是 `Mask`(8-bit alpha
整画布,`Mask::fill_path(path, fill_rule, anti_alias, transform)` 造形,
`fill_path`/`stroke_path` 尾参 `Option<&Mask>` 消费——本仓库现在传的就是 `None`,
paint.rs:319)。

## 2. 方案设计(对准本仓库代码)

### 2.1 场景树:overflow 属性,不加新 ElementKind

`crates/sv-ui/src/lib.rs`:
- `Style` 加 `overflow: Overflow`,`enum Overflow { Visible(默认), Hidden, Scroll }`
  ——View 仍是唯一容器 kind,measure/place/paint/dump 不加 kind 分支;与 ADR-8 的
  CSS 面天然咬合(`overflow: scroll` 就是一个枚举属性)。**注意两处既有护栏**:
  手写的 `impl PartialEq for Style`(lib.rs:153,漏加字段 = 相等剪枝失效)与
  memory_probe 的 `Style ≤128B / ViewNode ≤320B` 预算测试(lib.rs:1237);
- `ViewNode` 加 `scroll_x: f32, scroll_y: f32`(**节点内状态为真源**,与 `text`/
  `checked` 同款;业界公因子 1)+ `on_scroll: Option<Rc<dyn Fn(f32, f32)>>`
  (新 offset 回调,virtual_list 桥接与 `onscroll` 事件的载体)+
  `content_override: Option<(f32, f32)>`(虚拟内容尺寸覆盖,见 2.6);
- `Doc` 加:`set_scroll(id, x, y)`(clamp 到 `content − viewport`,相等剪枝后
  bump,抄 `set_checked` 的模板,lib.rs:377)、`scroll_of(id)`、
  `scroll_by(id, dx, dy) -> (f32, f32)`(**返回实际消费量**,滚动链路由的依据)、
  `set_on_scroll`/`scroll_handler`(抄 `set_on_click`/`click_handler` 对);
- 绑定原语 `bind_scroll_y(doc, id, sig: Signal<f32>)`:effect 里 `sig → set_scroll`,
  另把 `on_scroll` 接成 `sig.set`——双向桥,`.sv` 的 `bind:scrolly` 编译目标。

不选 `ElementKind::Scroll` 的理由:kind 匹配散布在 measure/place/paint/dump 四处
(render.rs:107/185/376,lib.rs:520),新容器 kind 每处都要长分支;而 overflow
是 View 的正交属性,改动收敛在布局与绘制各一处。不选"offset 只是 Signal"的
理由:shell 命中滚轮后需要从 ViewId 找到"写谁",树上有真源 + handler 取用,
与现有事件模型(`click_handler` clone 出来调,sv-shell lib.rs:249)完全同构。

### 2.2 布局:offset 平移 + clip 传播 + content 尺寸

`crates/sv-shell/src/render.rs`:
- `measure`:overflow ≠ Visible 的 View,border-box 尺寸**不被内容撑开**——显式
  `width/height`(或父 forced)即最终尺寸;内容自然尺寸(现 View 分支算出的
  main/cross,render.rs:122–154)另存为 `content_size`,供 clamp 与滚动条比例用;
  有 `content_override` 时以覆盖值为准;
- `place`:进入 overflow ≠ Visible 的容器,子起点改为
  `cx = x + padding.left + bw − scroll_x`(cy 同理);同时维护向下传播的
  `clip: Option<Rect>`(与祖先 clip 求交集);
- 布局产物双输出:`Placed` 加 `clip: Option<Rect>` 字段(命中测试直接用);
  `layout_tree` 旁路输出**clip 区段表** `Vec<(start, end, Rect, radius)>`
  (place 递归进入/退出滚动容器时记下标区间,天然嵌套)——paint 按区段
  push/pop,免于逐条比较 clip 链;
- `layout_tree_cached`(render.rs:235)不用动:滚动改 offset → bump 版本 →
  缓存键自然失效。**代价明说**:滚动帧 = 全树重布局 O(n),ADR-9"局部布局"
  阶梯之前,大全量树的滚动性能靠 virtual_list 兜底(它只有视口槽位,O(视口));
- `hit_click_target` 与 `update_hover`(sv-shell lib.rs:196):命中条件补
  `p.clip.map_or(true, |c| c.contains(x, y))`——视口外的子节点不可点/不可悬停。

### 2.3 Painter 加 clip 动词

`crates/sv-shell/src/paint.rs` 的 `Painter` trait(paint.rs:80)加两个动词
(物理像素,与现有动词一致):

```rust
fn push_clip(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32);
fn pop_clip(&mut self);
```

三个后端各自落法:
- **RecordingPainter**:`PaintCmd::PushClip{..}/PopClip` 入命令流——金样测试
  免费覆盖"区段表发射顺序正确"(与现有 `recording_painter_golden` 同款);
- **VelloPainter**(vello_backend.rs):`scene.push_layer(Fill::NonZero,
  Mix::Normal, 1.0, Affine::IDENTITY, &RoundedRect::new(..))` / `pop_layer()`
  ——1:1 现成(ADR-3b 当初"词汇对齐 vello Scene"的预留兑现)。注:以仓库锁定
  的 vello 0.9 实际签名为准,docs.rs latest 是五参形式(含 clip_style);
- **TinySkiaPainter**:裁决 **v0 手动矩形裁剪,不用 Mask**。painter 内维护
  `Vec<Rect>` 交集栈:`fill/stroke_rounded_rect` 先与栈顶求交(radius=0 精确;
  radius>0 时矩形近似,角部最多溢出 ~radius² 像素);`glyph_run` 在
  `blend_pixel`(paint.rs:295,已有画布边界判断)加 clip rect 判断,逐像素
  天然精确、近零成本。理由:滚动容器 99% 直角或小圆角;Mask 每次 push 要分配
  w×h 字节整画布 + 嵌套要逐像素相乘,与"CPU 栈能力冻结"(ADR-3b)相悖。
  Mask 路线留作圆角裁剪需要精确时的升级项(API 已核实可行,尾参位现成)。

### 2.4 sv-shell:滚轮接入、路由、滚动条、指针捕获

`crates/sv-shell/src/lib.rs` `window_event`(lib.rs:311)加分支:

```rust
WindowEvent::MouseWheel { delta, .. } => {
    let (dx, dy) = match delta {
        MouseScrollDelta::LineDelta(x, y) => (x * LINE_PX, y * LINE_PX), // LINE_PX ≈ 40 逻辑 px,可调
        MouseScrollDelta::PixelDelta(p) => ((p.x / scale) as f32, (p.y / scale) as f32),
    };
    // winit 正值 = 内容右/下移 → offset 减
    if let Some((id, ..)) = route_wheel(&self.doc, &self.placed, lx, ly, -dx, -dy) { ... }
}
```

- **路由 = 最近可滚祖先 + 滚动链**:抽成纯函数
  `route_wheel(doc, placed, x, y, dx, dy)`——命中最上层 `overflow == Scroll`
  且 clip 内的容器,`scroll_by` 消费;该方向已到边界(返回消费量 0)则沿
  `parent` 链上浮找下一个可滚祖先(浏览器 scroll chaining 语义)。纯函数才能
  离屏单测(winit 事件无法 headless 构造);
- 派发后调 `scroll_handler` 回调(带新 offset),并手动重跑 `update_hover`
  (滚动后指针下的内容变了);
- **滚动条**:shell 合成绘制,**不入场景树**(egui 同构;Slint Flickable 也
  明确不带滚动条)。`paint_tree` 在 clip 区段 pop 之后画 thumb:
  `thumb_len = viewport/content × track`,宽 6–8 逻辑 px,`fill_rounded_rect`
  两条命令(track 可选)。thumb 几何抽纯函数 `scrollbar_thumb(viewport, content,
  offset, track) -> Rect`,布局旁路输出 `Vec<(ViewId, Rect)>` 供拖拽命中;
- **指针捕获(顺带补调研 19 点名的指针缺口)**:`App` 加
  `captured: Option<CaptureTarget>`(`enum { Node(ViewId), ScrollThumb { id,
  start_cursor, start_offset } }`)。`MouseInput Pressed` 命中 thumb 几何 →
  设 captured;`CursorMoved` 且 captured 存在 → 不走 hover,把位移映射回
  `set_scroll`(`Δoffset = Δcursor × content/track`);`Released` → 释放
  ——Masonry capture_pointer 的最小同构。同时给 `ViewNode` 补
  `on_pointer_move: Option<Rc<dyn Fn(f32, f32)>>` 通道(捕获期间派发逻辑坐标),
  这是后续拖拽/Slider/文本选区都要的地基;
- **惯性/平滑(ADR-9"滚动物理"阶梯,可后置)**:滚轮不直接写 offset,而是写
  "目标 offset",`sv_ui::anim::pump`(已有动画泵,sv-shell lib.rs:125 每帧调)
  按临界阻尼逼近——egui `smooth_scroll_delta` + `animated()` 的同构最小版;
  触摸板 `PixelDelta` 直通不做平滑(设备已平滑)。

### 2.5 sv-compiler:CSS overflow + onscroll / bind:scrolly

- `crates/sv-compiler/src/style.rs`:自写解析器(ADR-8 C1)属性白名单加
  `overflow: visible|hidden|scroll|auto`(v0 `auto` 按 `scroll` 处理),落
  `Style.overflow`——枚举属性,照 `cursor` 的既有模式加;
- `crates/sv-compiler/src/codegen.rs` 两处同步:
  ① 属性预扫描的忽略清单(codegen.rs:559–566)加 `"onscroll"` 与
  `"bind:scrolly"`;② 事件/绑定 match:`"onscroll" | "on:scroll"` 生成
  `__doc.set_on_scroll(#el, ...)`(抄 onclick 分支,codegen.rs:770);
  `"bind:scrolly"` 生成 `::sv_ui::bind_scroll_y(&__doc, #el, sig)`(抄
  bind:checked 分支的"值必须是反应式变量"校验,codegen.rs:817–851)。同时把
  现有拒绝分支(codegen.rs:873 的 bind: 兜底、934 的 on: 兜底)提示语更新;
- **sv-macro 同步**(CLAUDE.md 约束:绑定原语签名变动要同步 sv-macro codegen
  与其测试):view! 前端加同名属性支持,测试对齐;
- 不加 `<scroll>` 元素:`<view style="overflow: scroll; height: 300px">` 即可,
  模板层零新语法。

### 2.6 virtual_list 合流:从程序驱动到真实输入驱动

现状:`virtual_list(doc, parent, count, offset: Signal<usize>, viewport_rows,
item_at, row)`(sv-ui lib.rs:706),offset 只被程序驱动(membench
`Driver::Scroll` 每帧 +1,examples/membench/src/main.rs:172)。接缝设计:

- 新原语 `virtual_scroll(doc, container, count, row_h, offset: Signal<usize>)`:
  ① effect 维护 `content_override = (0, count() × row_h)`(count 变化自动更新)
  ——滚动条比例与 clamp 由虚拟高度决定,这是 Flutter "sliver 懒布局 + 视口
  offset" 思想的最小等价物;② `set_on_scroll(container, px → offset.set((px /
  row_h) as usize))`——像素域到行域的唯一换算点;
- 用法:外层 `overflow: Scroll` 容器(显式 height)+ 容器内 `virtual_list`
  槽位 + `virtual_scroll` 桥,三行接通;**virtual_list 本体零改动**——
  `offset: Signal<usize>` 当初的接口选择在这里兑现;
- v1 整行滚(offset 对齐行,滚动条步进 = row_h);v2 像素级平滑:槽位容器
  place 时额外平移 `−(px mod row_h)`(一个字段 + place 一行);
- membench 加 `--input wheel` 模式:`route_wheel` 合成事件驱动,替代
  `Driver::Scroll`,让 ADR-9 的 1% low 数字在真实输入路径上复测。

## 3. 分步落地(可并行度:S1 与 S2 可并行;S2' 依赖 S2)

| 步 | 内容 | 验收(测试名) | 估算(置信度) |
|---|---|---|---|
| S1 | Painter `push_clip/pop_clip` 三后端(Recording/tiny-skia 矩形/vello push_layer) | `clip_cmds_golden`、`cpu_clip_excludes_outside_pixels`、`vello_clip_parity`(沿用非白像素比对拍) | 1 人周(高) |
| S2 | Style.overflow + ViewNode scroll/content_override + Doc scroll API + place 平移 + Placed.clip/区段表 | `scroll_offset_shifts_children`、`scroll_clamps_to_content`、`clipped_child_not_hit` | 1–1.5 人周(高) |
| S2' | sv-compiler:overflow 解析 + onscroll/bind:scrolly + sv-macro 同步 | `sfc_overflow_style_parses`、`sfc_bind_scrolly_roundtrip` | 0.5–1 人周(高) |
| S3 | MouseWheel 分支 + LineDelta/PixelDelta 换算 + `route_wheel` 最近可滚祖先/滚动链 | `wheel_routes_to_nearest_scrollable`、`wheel_chains_to_ancestor_at_edge`、`line_and_pixel_delta_normalize` | 0.5–1 人周(高) |
| S4 | 滚动条绘制 + thumb 拖拽(captured 通道 + on_pointer_move) | `scrollbar_thumb_geometry`、`thumb_drag_maps_to_offset`、`pointer_capture_routes_move_outside_bounds` | 1–2 人周(中:capture 与 hover 的相互作用有细节) |
| S5 | virtual_scroll 桥 + membench `--input wheel` + 开窗手滚 20 万行 | `virtual_list_driven_by_wheel`、membench READY 行含 wheel 模式数字 | 0.5–1 人周(高) |
| S6 | 平滑/惯性(anim 泵载目标 offset;可后置) | `smooth_scroll_converges_and_stops`(收敛后静止帧短路仍生效) | 1–2 人周(中低) |

合计 5.5–8.5 人周;不含 S6 为 4.5–6.5 人周。

## 4. 风险与开放问题

1. **tiny-skia 圆角裁剪是矩形近似**:角部溢出在深色内容 + 大圆角时可见;Mask
   兜底路线的内存(整画布 w×h 字节/层)与性能未实测——留给需要时基准;
2. **vello `push_clip_layer` 语义缺陷(issue #1198)**:v0 用 `push_layer` +
   默认 blend 绕开;锁定版 0.9 的实际签名与 docs.rs latest 是否一致开工时核对;
3. **滚动帧全树重布局 O(n)**:布局缓存按版本失效,滚动必 miss;全量物化大树
   (10k+)连续滚动可能撞帧预算——virtual_list 是正解,局部布局是 ADR-9 既列
   阶梯,此处不另起炉灶;同理 paint 侧未做视口剔除(clip 外的 Placed 仍发
   命令再被裁),v1 可在 paint_tree 按区段跳过;
4. **滚动条不入场景树**的代价:不可被 CSS 样式化、AccessKit(M2)无法暴露
   滚动语义——届时需要复评"合成绘制 vs 入树 widget",Masonry 是入树先例;
5. **触摸/拖拽滚动**(Slint 的 8px/500ms 手势判定、egui drag_to_scroll)不在
   本切片,鸿蒙(触摸为主)M3 前必须回补;
6. **ADR-6 帧调度未落地**:高频滚轮每事件 bump→request_redraw,靠 winit 合帧
   兜住;batch 语义与 mailbox 归 ADR-6,本方案不依赖但受益;
7. `bind:scrolly` 命名是自造面(Svelte 元素无 scrollTop 绑定先例,只有
   `<svelte:window bind:scrollY>`),`.sv` 语法冻结前需复评;
8. 横向滚动(Shift+滚轮、LineDelta.x)本方案 API 全程带 x 通道,但验收只押
   纵向;`Overflow` 按轴拆分(overflow-x/y)留 C2。

## 5. 最小可商用切片(对调研 19 分档)

- **档 A(内部工具)**:S1 + S2 + S2' + S3 + S5——有裁剪、滚轮可滚、最近可滚
  祖先路由、virtual_list 接真实输入;滚动条**只绘不拖**(egui 早期形态,拖拽
  缺失可用滚轮替代)。约 4–5.5 人周,正好嵌进调研 19 最短路径第 3 步;
- **档 B(商用)**:再加 S4(拖拽 + 指针捕获)+ S6(平滑/惯性)+ 触摸板
  PixelDelta 自然滚动手感调优 + 键盘滚动(PageUp/Down/Home/End,依赖 M1 焦点
  链)+ 嵌套滚动完备 + AccessKit 滚动语义(M2)。滚动体系自身无档 B 独有的
  未知段——最大不确定性仍在文本栈,不在这里。

## 出处(业界核实)

- egui ScrollArea:https://docs.rs/egui/latest/egui/containers/scroll_area/struct.ScrollArea.html
- egui InputState(smooth_scroll_delta):https://docs.rs/egui/latest/egui/struct.InputState.html
- iced scrollable 模块 / Scrollable / snap_to:https://docs.rs/iced/latest/iced/widget/scrollable/index.html · https://docs.rs/iced/latest/iced/widget/scrollable/struct.Scrollable.html · https://docs.rs/iced/0.13.1/iced/widget/scrollable/fn.snap_to.html
- Masonry Portal:https://docs.rs/masonry/latest/masonry/widgets/struct.Portal.html
- Masonry EventCtx(capture_pointer):https://docs.rs/masonry/latest/masonry/core/struct.EventCtx.html
- Flutter Viewport:https://api.flutter.dev/flutter/widgets/Viewport-class.html
- Slint Flickable(1.5.1 版文档;latest 站点该页路径变更未寻获):https://releases.slint.dev/releases/1.5.1/docs/slint/src/language/builtins/elements
- floem views(virtual_stack):https://docs.rs/floem/latest/floem/views/index.html
- winit MouseScrollDelta:https://docs.rs/winit/latest/winit/event/enum.MouseScrollDelta.html
- vello Scene(push_layer/push_clip_layer):https://docs.rs/vello/latest/vello/struct.Scene.html
- tiny-skia Mask:https://docs.rs/tiny-skia/latest/tiny_skia/struct.Mask.html
