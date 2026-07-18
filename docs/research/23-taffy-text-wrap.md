# 23 · taffy 布局引擎接入 + 文本换行:档 A 最后两块的落地设计

> 2026-07-18。方法:联网核实业界先例(crates.io API 查版本、GitHub/docs.rs 查
> taffy 0.12 协议与 Blitz/Slint/iced/floem/cosmic-text/parley 的真实依赖),
> 再逐文件核对本仓库 sv-ui/sv-shell/sv-compiler 源码,给出改动清单、分步验收与人周。

## 0. 一句话结论

**两块同批做,顺序是"taffy 骨架先行、折行骑在 measure func 上"**:taffy 0.12
以"变更帧重建 TaffyTree + measure function"的最小形态封在 `layout_tree` 函数内
(`Vec<Placed>` 输出契约不变,paint/命中测试/缓存全部不动);文本换行**不等
Parley**,在 swash 现有线性排版上加 unicode-linebreak 断点(Slint 同款依赖),
作为**计划内报废**的过渡投资(~1.5 人周,换档 A 提前一个季度)。全程 ~9 人周,
与 ADR-8 C2 的 6–10 人周口径吻合。

## 1. 业界先例(联网核实,2026-07-18)

| 框架 | 布局 | 文本换行 | 出处 |
|---|---|---|---|
| Blitz/Dioxus Native | taffy,**低层 trait 直接实现在自家 DOM 上**(零平行树) | Parley(inline root 整段接管,文本节点不单独 measure) | github.com/DioxusLabs/blitz(blitz-dom/src/layout/mod.rs:`impl LayoutPartialTree/CacheTree/... for BaseDocument`) |
| Slint | taffy **0.10**(default-features off,仅 `flexbox`+`taffy_tree`+`alloc`) | unicode-linebreak 0.1.5;文本栈已迁 parley 0.11(`shared-parley` 默认 feature) | github.com/slint-ui/slint · internal/core/Cargo.toml |
| floem | taffy(README 明言 Flexbox + Grid 皆直通) | —(未核实其文本栈) | github.com/lapce/floem |
| iced / COSMIC | **自研** `Limits`/`Node` 约束式布局 + `flex` 子模块,不用 taffy | cosmic-text 0.19(自研折行/bidi,shaping 走 HarfRust) | docs.rs/iced_core `layout` 模块 · github.com/iced-rs/iced Cargo.toml · github.com/pop-os/cosmic-text |
| egui | 自带即时模式布局;第三方桥 egui_taffy 存在 | 自带(细节本次未核实) | crates.io/crates/egui_taffy |
| Masonry/Xilem | 布局协议形态**未核实到**(docs.rs 首页无说明) | Parley("Parley for the text stack",官方明言) | docs.rs/masonry |
| Zed GPUI | 维护 taffy fork(据此推断在用,**未直接核实代码引用**) | — | github.com/zed-industries/taffy |

**taffy 本体现状**(github.com/DioxusLabs/taffy · docs.rs/taffy · crates.io API):
- 最新 **0.12.2(2026-07-15)**;0.12.0 于 2026-07-03、0.11.0 于 2026-06-12 发布。
  注意版本新鲜度:0.12.0 发布当天即出 0.12.1 修两个缓存紧急 bug——**cache 路径
  刚翻修过**,0.12 的 cache key 变全面(典型 +10% 开销,换正确性)。
- 0.11 有 breaking:对齐类型从 enum variant 改关联常量(`AlignContent::Start`
  → `AlignContent::START`)。Slint 仍钉在 0.10——踩坑可退的兜底版本。
- 算法:Flexbox / Grid / Block(+float 子特性),feature 可裁
  (`flexbox`/`grid`/`block_layout`/`calc`/`content_size`/`taffy_tree`)。
- **measure function 协议**(examples/measure.rs + `TaffyTree::compute_layout_with_measure`):
  叶子节点带任意 context,布局期回调
  `|known_dimensions, available_space, node_id, node_context, style| -> Size<f32>`,
  官方文档明言这就是"接文本布局"的通道。
- `taffy::Style` 字段(docs.rs 核实):display/box_sizing/overflow/scrollbar_width/
  position/inset/size/min_size/max_size/aspect_ratio/margin/padding/border/gap/
  六个 align·justify/flex 四件/text_align/grid 十件——我们要的全都在。

**两种集成形态的先例分界**:Blitz 走低层 trait(DOM 即 taffy 树,帧间缓存原地
持久化);floem/Slint 走高层 `TaffyTree`(平行树)。前者增量最优但侵入 ViewNode;
后者简单、与"版本键全量重算"模型天然合拍。

## 2. 方案设计(对准本仓库代码)

### 2.1 接入形态裁决:变更帧重建 TaffyTree,封在 layout_tree 内

现状(`sv-shell/src/render.rs`):`measure`(89 行起,递归行列堆叠)+ `place`
(168 行起)→ `layout_tree`(261)→ `layout_tree_cached`(235,缓存键 =
`(doc.identity, doc.version, w_bits, h_bits)`)→ 产出 `Vec<Placed>`,被
`paint_tree`/`hit_click_target`/sv-shell 事件层消费。

**改法:只换 `layout_tree` 的函数体**,签名与 `Placed` 契约不变:

```rust
fn layout_tree(doc, lw, lh) -> Vec<Placed> {
    doc.read(|inner| {
        let mut tree: TaffyTree<MeasureCtx> = TaffyTree::new();
        tree.disable_rounding();                    // 见 2.6 HiDPI
        let root = build(&mut tree, inner, inner.root); // 递归:View→with_children,
                                                    // 叶子→new_leaf_with_context
        tree.compute_layout_with_measure(root, definite(lw, lh), measure_leaf)?;
        walk_absolute(&tree, root, /*累加 location 得绝对坐标*/) // → Vec<Placed>
    })
}
```

- `MeasureCtx` = `{ kind, text 引用或索引, resolved_font_px }`(继承字号在 build
  期沿父链解析,与现状 `inherited_font` 下传一致,继承不进 taffy)。
- ViewId↔NodeId 用 build 期的 `Vec<(NodeId, ViewId)>` 回查,不改 ViewNode。
- 静止帧照旧被 `layout_tree_cached` 短路(细粒度模型下静止是常态,ADR-9),
  变更帧 O(n) 重建——虚拟化工况 n≈34(`virtual_list_million_rows_few_nodes`
  钉死),重建成本 µs 级;全量档见 2.6。
- **增量预留**(不在本切片):若全量档实测超预算,升级为 Blitz 式低层 trait
  (`LayoutPartialTree` + `CacheTree` 直接实现在 `DocumentInner` 上,ViewNode 加
  cache 槽),触发条件写死为"30k 档 build+layout > 2ms"。

### 2.2 sv-ui Style → taffy::Style 映射表

原则:**sv-ui 不依赖 taffy 类型**(与 Painter 边界同理,sv-ui 是双前端编译目标,
类型稳定优先);sv-shell 内做纯函数 `to_taffy(&Style) -> taffy::Style`。

| sv `Style` 现有字段(lib.rs:112) | taffy::Style | 备注 |
|---|---|---|
| `direction: Column/Row` | `display: Flex` + `flex_direction` | 全节点缺省 Flex |
| `gap: f32` | `gap: Size{ w: length(g), h: length(g) }` | **语义差**:现状 gap 只作用主轴;taffy 双轴(wrap 后交叉轴也生效)。nowrap 下等价,单测钉住 |
| `padding: Edges` | `padding: Rect<LengthPercentage>` | 直通 |
| `margin: Edges` | `margin: Rect<LengthPercentageAuto>` | 现状"父容器代记 margin"(measure 129/198 行)改由 taffy 原生处理,对拍验证 |
| `border: Option<Border>` | `border: Rect<LengthPercentage>`(width) | 颜色仍留绘制层 |
| `width/height: Option<f32>` | `size: Size<Dimension>`(None→Auto) | 现状注释"显式宽高 = border-box 覆盖"(render.rs:156);taffy 有 `box_sizing`,缺省值以单测钉死 border-box 行为 |
| corner_radius/bg/fg/font_size/opacity/cursor | 不进 taffy | 纯绘制/继承层 |

**新增字段(flex 全家 + 换行,分批暴露)**:

- 第一批(平铺进 Style):`justify_content`、`align_items`、`align_self`、
  `flex_grow`、`flex_shrink`、`flex_wrap`、`min/max_width/height: Option<f32>`、
  `text_wrap: Wrap/NoWrap`(缺省 Wrap,Button/Checkbox 语义 NoWrap)、
  `text_align: Left/Center/Right`。
- 第二批(`Option<Box<StyleExt>>` 冷字段,绝大多数节点不付内存):`flex_basis`、
  `position: Relative/Absolute` + `inset: Edges`(**弹层体系的前置**)、百分比
  (`width: Option<f32>` → `Option<Length{Px,Percent}>`,**breaking**,sv-macro
  codegen 与测试同步改——CLAUDE.md 约束)、`overflow`(留给滚动切片)。
- 第三批:grid 十件(floem 先例证明可直通;语法面大,只先暴露
  `grid-template-columns/rows` 简写)。
- **内存护栏**:`core_struct_sizes_within_budget`(sv-ui lib.rs:1237)现钉
  Style ≤128B/ViewNode ≤320B;第一批平铺约 +16–24B,预算上调为 Style ≤160B 并
  在该测试留一行理由注释;冷字段进 Box 保 ViewNode 不破 320B。

### 2.3 measure function:文本节点的两趟测量协议

taffy 对叶子的询问天然就是"(已知宽度→高度)两趟":先以
`available_space = MaxContent/MinContent` 问固有宽(不折行宽 / 最长不可断段宽),
再带 `known_dimensions.width = Some(w)`(或 `Definite(w)`)问折行后高度。对应实现
(`sv-shell/src/render.rs` 新增,替换 `measure_text` 76 行):

```rust
/// (宽, 高, 行切分):wrap_w=None 即单行(现 measure_text 语义)
fn measure_text_wrapped(font, text, px, wrap_w: Option<f32>) -> (f32, f32, Vec<Range<usize>>)
```

measure_leaf 按 kind 分派:`Text` 走上式;`Button` 恒单行(现状 measure 107 行
语义);`Checkbox` 恒 `fs.max(14.0)` 方块(现状 115 行)。折行结果(行切分)
进 thread-local 缓存,键 `(text_hash, px_bits, wrap_w_bits)`,与字形缓存同款
两代淘汰(paint.rs:187 glyph_cache 的分代策略复用)——paint 期 `shape_text_wrapped`
直接复用行切分,不二次扫描。

### 2.4 文本换行裁决:swash 简版**值得做**,不等 Parley

- **裁决依据**:调研 19 把换行列为九项交集第 4 项、档 A 必备;Parley 在 M2
  (还捆着 IME/fallback/emoji 一整包)。等 Parley = 档 A 至少推后一个季度。
- **成本极低**:现状 `shape_text`(render.rs:303)已是"charmap 逐字 + advance
  推进",折行 = 在 advance 累计上加断点判定 + 多行 y 偏移(`GlyphPos` 已是逐字
  绝对坐标,`Painter::glyph_run` 动词**零改动**,多行就是 y 不同的同一段 run)。
- **断点规则**:直接引 **unicode-linebreak 0.1.5**(UAX #14 表驱动,Slint 同款,
  零传递依赖)而非手写"空格+CJK 逐字"——手写版会立刻在标点禁则上翻车
  (行首出现"、。"),而 crate 免费给出正确 CJK 断点。fallback:超长不可断段
  (长 URL)按字符强制断。
- **明确不做**(冻结面,与 ADR-3b"tiny-skia 栈能力冻结"同一逻辑):kerning、
  连字、bidi、hyphenation、justify 对齐、行内混排字号。`text-align:
  left/center/right` 顺手做(逐行 x 偏移,Button 居中已有同类代码 render.rs:386)。
- **与 Parley 的关系**:折行全部收在 `measure_text_wrapped`/`shape_text_wrapped`
  两个门面后;M2 Parley 落地时整体换门面,本切片代码**计划内报废**(~1.5 人周
  过渡投资)。Parley 0.11(2026-06-26)+ fontique/HarfRust 已被 Blitz/Masonry/
  Slint 三家采用,归宿不变。

### 2.5 sv-compiler / sv-macro 侧改动

- `sv-compiler/src/style.rs` 的 match(432–548 行)新增键:
  `justify-content`/`align-items`/`align-self`/`flex-grow`/`flex-shrink`/
  `flex-wrap`/`min(max)-width(height)`/`white-space`(normal|nowrap →
  text_wrap)/`text-align`;第二批 `flex-basis`/`position`/`top·right·bottom·left`
  /百分比(`parse_length` 增 `%`,值类型随 2.2 的 Length 改造);未知键错误提示
  (541 行)同步更新支持列表。
- `flex-direction` 现仅收 `row|column`(508 行)→ 增 `row-reverse|column-reverse`
  (taffy 免费)。
- sv-macro:`view!` 的 style 属性走同一份解析(核心合并前双份代码,按 CLAUDE.md
  约束同步改 codegen 与 tests/view.rs)。
- 本切片**不涉及**事件语法(`bind:value`/`on:keydown` 属文本输入切片);但
  `overflow` 键预留给滚动切片,`position/inset` 给弹层切片——三个切片对 Style
  的字段申请在本报告 2.2 一次规划,避免三次 breaking。

### 2.6 与 virtual_list / ADR-9 预算 / HiDPI 的咬合

- **百万控件不会被拖累**:虚拟化后场景树恒 ~34 节点(20 万行档同理是视口槽位数),
  变更帧重建 34 节点的 TaffyTree 在 µs 量级,p99 预算 6.94ms(ADR-9)无感。
  滚动帧 = 逐槽 set 文本 → version bump → 重建布局:**槽位定高**(行高固定)时
  文本变化不改布局结果,可加"文本变更但各叶子尺寸不变 → 复用上帧 Placed"的
  快路径(折行缓存命中即尺寸未变),把滚动帧布局成本进一步压平。
- **全量档**(membench 10k/30k)是真风险点:每帧重建 + taffy 0.12 变重的 cache
  key。验收线:30k 档 build+layout ≤ 2ms 且 ≤ 现状 measure/place 的 3 倍;
  超线即触发 2.1 的低层 trait 升级路径。
- **HiDPI**:现状布局是逻辑坐标 f32、绘制乘 scale(render.rs:334);taffy 缺省
  对布局取整,逻辑坐标取整在 2x 屏会放大成整物理像素跳动 →
  `TaffyTree::disable_rounding()`,取整继续留给绘制端。
- **对拍安全网**:`recording_painter_golden`/`vello_offscreen_parity`/
  `offscreen_click_roundtrip` 全部在换引擎后必须原样通过(数值容差 ≤0.5 逻辑 px),
  这是 Painter 双后端金样体系的免费复用。

## 3. 分步落地(验收 = 测试名;人周为单人全职,置信度标注)

| 阶段 | 内容 | 验收 | 人周(置信度) |
|---|---|---|---|
| T1 | taffy 0.12 接入骨架:重建式 build + measure_leaf(单行)+ walk_absolute;现有字段直通(2.2 表上半) | `taffy_layout_matches_legacy_stack`(现有三个金样/回路测试全绿 + 新对拍);`gap_cross_axis_semantics_pinned` | 2(高) |
| T2 | 文本折行:unicode-linebreak + measure_text_wrapped/shape_text_wrapped + text_wrap/text_align + 折行缓存 | `text_wraps_at_container_width`、`cjk_wraps_without_spaces`、`no_break_before_cjk_punct`、`wrapped_measure_two_pass`(MaxContent 单行宽 / Definite 折行高)、`long_token_force_breaks` | 1.5(中) |
| T3 | flex 第一批暴露:grow/shrink/wrap/justify/align/min-max + sv-compiler 键 + sv-macro 同步 + showcase 案例 | `flex_grow_distributes_free_space`、`justify_space_between`、`align_items_center`、`style_c2_flex_keys_compile`(.sv 端到端) | 2(中) |
| T4 | Length 改造(百分比)+ position:absolute/inset(弹层前置) | `percent_width_resolves_against_parent`、`absolute_inset_positions`(breaking 波及 sv-macro 测试同步) | 1.5(中低) |
| T5 | grid 简写直通 + @media 尺寸类(C2 清单收尾)+ CSS-SUPPORT 矩阵更新 | `grid_template_two_columns`、`media_min_width_switches` | 2(低,grid 语法面可能砍) |
| 性能回归 | membench 增 taffy 档、1M 虚拟化复测 | `virtual_list_p99_budget_with_taffy`(p99 < 6.94ms)、30k 档 ≤2ms 线 | 含在各阶段 |

合计 ~9 人周,ADR-8 C2 的 6–10 人周口径内(折行 1.5 已含)。T1+T2 可并行度低
(T2 依赖 T1 的 measure 通道),建议串行。

## 4. 风险与开放问题

1. **taffy 0.12 新鲜度**:cache 路径两周前刚连出两个紧急修复(0.12.1),我们的
   "每帧重建"恰好绕开帧间 cache,但树内多趟 cache 仍在用。兜底:Slint 同款 0.10
   是已验证退路;对拍测试面要厚。
2. **每帧重建的全量档上限未实测**:2.6 的 2ms 线是拍的(置信度中),越线成本是
   低层 trait 改造 ~2–3 人周追加。
3. **对拍不可能逐像素相等**:margin 处理方式换轨(父代记 → taffy 原生)、
   舍入策略换轨,预期有 ≤1px 级差异,金样需要容差机制或一次性重录——重录会
   稀释"换引擎零回归"的证明力,倾向容差。
4. **baseline 对齐**(`align-items: baseline`)需要 measure 通道回报基线,taffy
   高层 API 是否暴露未核实——本切片不承诺,开放问题。
5. **折行与未来文本输入的行模型**:光标定位/点击命中需要行切分 + 每行字符→x
   映射;`measure_text_wrapped` 返回的行 Range 已是该接口的雏形,但字符级 x 映射
   本切片不做——文本输入切片要重开这一层时,可能倒逼提前上 Parley。
6. **gap 双轴语义**、box_sizing 缺省值:均以单测钉死(2.2),不留口头约定。
7. `text-align: justify`、`line-height` 属性、`word-break: break-all`:永不/暂缓
   清单,文档化到 CSS-SUPPORT。

## 5. 结论:最小可商用切片

- **档 A(内部工具可用)**:T1 + T2 + T3(~5.5 人周)即达标——flex 常用面 +
  容器宽折行 + CJK 断点正确,TodoMVC/设置面板级 UI 的布局与多行文本不再撒谎。
  百分比、grid、absolute 都可以不做;折行用 swash 简版,**不等 Parley**。
- **档 B(单桌面平台可商用)**:在 T4 + T5 之上再加:M2 Parley 替换折行门面
  (fallback/emoji/IME 的载体,先例三家已趟平)、baseline 对齐复评、增量布局
  按触发条件落地、taffy 版本钉住 + 上游 cache bug 跟踪纪律进 CI。布局这一项
  到此为止;档 B 的剩余距离在文本输入/弹层/发布工程(调研 19 §2)。

## 出处

- taffy:https://github.com/DioxusLabs/taffy · https://docs.rs/taffy ·
  https://docs.rs/taffy/latest/taffy/style/struct.Style.html ·
  https://github.com/DioxusLabs/taffy/blob/main/examples/measure.rs ·
  crates.io API(0.12.2 / 2026-07-15)· https://github.com/DioxusLabs/taffy/releases
- Blitz:https://github.com/DioxusLabs/blitz(blitz-dom 直接实现 taffy 低层 trait)
- Slint:https://github.com/slint-ui/slint · internal/core/Cargo.toml
  (taffy 0.10 flexbox-only + unicode-linebreak + parley 0.11)
- iced:https://docs.rs/iced_core(layout 模块)· https://github.com/iced-rs/iced
  (Cargo.toml,cosmic-text 0.19)
- cosmic-text:https://github.com/pop-os/cosmic-text(自研折行/bidi,HarfRust)
- floem:https://github.com/lapce/floem(README:taffy Flexbox+Grid)
- parley:https://github.com/linebender/parley(0.11.0,2026-06-26)
- Masonry:https://docs.rs/masonry(Parley 文本栈;布局协议未核实)
- egui 桥:https://crates.io/crates/egui_taffy · GPUI fork:
  https://github.com/zed-industries/taffy
