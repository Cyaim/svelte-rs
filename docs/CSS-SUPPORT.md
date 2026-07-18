# 现代 CSS 对比矩阵:哪些实现了,哪些没有

> 生成:2026-07-18。"现代 CSS"按 2026 年中 **Baseline** 口径(含 2023–2026 落地的
> nesting/:has()/容器查询/@layer/oklch/锚点定位/视图过渡/滚动驱动动画等新浪潮)。
> "sv 现状"以本仓库代码实证;裁决依据 [DESIGN.md ADR-8](DESIGN.md) 与联网核实的
> 调研 [11](research/11-css-industry-strategies.md)/[12](research/12-css-semantics-mapping.md)。
>
> 大前提:sv 的样式是**真 CSS 语法的封闭子集 + 编译期样式表**(scoped by default、
> 零运行时选择器引擎)。因此"未实现"分五种去向,不是一刀切:
>
> | 标记 | 含义 |
> |---|---|
> | ✅ | 已实现(有测试/示例) |
> | 📅 C1 / 📅 C2 | 已排期(ADR-8:C1≈3–5 人周;C2=迁移无感线,踩 M1 taffy) |
> | ✏️ P2 | 降级/编译期形态支持,远期 |
> | ⏳ | 桌面有意义,未排期(前置条件注明) |
> | ❌ | 永不(桌面无对应/被架构裁掉,给替代写法) |

## C1 已落地(2026-07-18)

> **本节覆盖正文行的状态列**(冲突以此为准)。C1 批次已实现并验证
> (`css_c1_box_model_vars_nesting` 等 46 个编译器测试 + showcase 渲染):
>
> - **盒模型**:`padding`/`margin` 1–4 值简写与 `-left` 系长手(`Edges` 四方向)、
>   `border: 2px solid <color>`(实线)与 `border: none`
> - **继承**:`color`/`font-size` 沿树解析(`fg=None`/`font_size=NAN` 哨兵,
>   渲染期 O(depth) 父链回溯);`currentColor`/`inherit` 关键字 → 继承语义
> - **单位**:`rem`(编译期 ×16);`em/%` 仍按引导报错(动态基准/taffy,C2)
> - **颜色**:`hsl()`/`hsla()`/`hwb()` 编译期折叠、现代空格斜杠语法
>   `rgb(255 62 0 / .5)`、`#hex` 4/8 位 alpha、命名色扩到 ~60
> - **变量**:`:root { --x }` + `var(--x[, fallback])` 编译期文本代入
> - **CSS 嵌套**:规则内 `&:hover { }` / `&:active { }` 展平
> - **伪类**:`:active`(按压状态位 + pointer down/up 自动接线,LVHA 序生效)
> - **选择器**:元素类型规则(`text { }` 组件内打底,specificity 直觉:元素 < 类)
> - **`cursor`**(pointer/text/grab/not-allowed,shell 悬停时切换系统光标)
>
> 修正统计:**✅24 / 📅C2 19 / ✏️P2 14 / ⏳18 / ❌16**(border-radius 四角独立值
> 从 C1 挪到 P2;`:focus/:disabled` 归 C2 随焦点链)。

## TL;DR

- **已实现**是刻意的小切片(≈12 项):标准属性名、px、hex/rgb()/颜色名、类 + `:hover`、
  scoped、opacity、`gap`/`flex-direction` 等——先证明"真 CSS 语法能进编译期样式表"。
- **差距大头在 C1/C2 排期内**(≈35 项):简写四值、em/rem/%、hsl()/currentColor、继承、
  margin/border 盒模型、flex/grid 全家、var() 主题、transition 属性、@media、:active/:focus。
  C2 完成即达"Svelte 开发者无感迁移线"(调研 11 的 Lynx 档结论)。
- **现代新浪潮大多 ⏳/❌**(≈40 项):容器查询/锚点定位/视图过渡/滚动驱动动画等要么
  依赖布局与帧调度基建(⏳),要么属于文档流/浏览器宿主语义,桌面场景树里没有对应物(❌)。
- 三条**永不做**的架构级裁决:specificity 数值计数与 `!important`(声明序+通道优先级替代)、
  运行时选择器引擎(一切匹配编译期做)、伪元素(装饰节点直接写模板)。

---

## A. 选择器与作用域

| 现代 CSS | sv 现状 | 说明 / 替代 |
|---|---|---|
| 类选择器 `.a`、多类叠加 | ✅ | 类=编译期样式表索引;`class="a b"` 声明序合成 |
| `:hover` | ✅ | 编译期生成悬停状态+指针接线,与用户回调合成 |
| `:active` / `:focus` / `:focus-visible` / `:disabled` / `:checked` | 📅 C1/C2 | 按压位随 shell;焦点链已落地(R1,调研 20),`:focus` 伪类接线排 C2(per-element `__fc` signal,复用 `:hover` 模式) |
| 元素类型选择器(`text { }`) | 📅 C1 | 组件内静态匹配 |
| 后代/子组合子 `.card .title`、`.a > .b` | ✏️ P2 | **编译期**对模板树匹配,不跨组件;匹配不到=警告 |
| `:first-child` / `:last-child` / `odd/even` | ✏️ P2 | 静态位置编译期判;each 行由 keyed reconcile 维护标志位;任意 `An+B` 砍 |
| CSS Nesting(2023 Baseline) | 📅 C1 | 嵌套语法糖编译期展平(`.btn { &:hover {} }`) |
| `:has()`(2023) | ❌ | 反向依赖需运行时引擎;桌面写法:状态提升 + `class:` |
| `:is()` / `:where()` | ✏️ P2 | 编译期展开;scoped 小世界里价值低 |
| `:not()` | ✏️ P2 | 编译期布尔取反 |
| 属性/ID/兄弟选择器(`[attr]`/`#id`/`~`/`+`) | ❌ | 场景树无属性字符串/ID 概念;条件用 `class:` |
| 伪元素 `::before/::after/::marker/::selection` | ❌ | **架构裁决**:装饰节点直接写模板(retained 树加节点是零成本操作) |
| `@scope`(2024) | ❌ | Svelte scoped-by-default 已覆盖其动机 |
| `:global()` | ❌ | 全局样式走 `:root` 令牌/主题表,不开全局选择器口子 |

## B. 级联系统

| 现代 CSS | sv 现状 | 说明 / 替代 |
|---|---|---|
| specificity 数值计数 | ❌ | **架构裁决**:声明序+通道优先级(类<内联<条件类<伪类<`style:` 指令);Svelte 自身用 `:where` 压平 specificity 佐证该心智 |
| `!important` | ❌ | 同上;终极覆盖 = `style:` 指令 |
| `@layer`(2022) | ❌ | 动机(第三方样式排序)在 scoped 模型下不存在 |
| 继承(color/font-* 沿树) | 📅 C1 | **P0 最重要单项**:layout 前 O(n) resolve 遍历,白名单继承 |
| `inherit` / `initial` / `unset` / `revert` 关键字 | ❌ | 继承是白名单自动行为,不开显式关键字(调研 12 裁决) |
| `all` 属性 | ❌ | 无级联栈,无意义 |

## C. 值、单位与函数

| 现代 CSS | sv 现状 | 说明 / 替代 |
|---|---|---|
| `px` / 无单位数 | ✅ | 逻辑像素,HiDPI 自动 |
| `em` / `rem` | 📅 C1 | resolve 期随继承折算 |
| `%` / `vw` / `vh` | 📅 C2 | % 需 taffy;vw/vh 接窗口尺寸 |
| `ch` / `lh` / `vmin` / `vmax` 等长尾单位 | ❌ | 低频长尾,裁 |
| `calc()` | ✏️ P2 | 常量折叠先行;混合单位进 resolve 期;急用 `style:` 写 Rust 表达式全能替代 |
| `min()` / `max()` / `clamp()`(2020) | ✏️ P2 | 同 calc() 通道 |
| `var()` 自定义属性 | 📅 C1/C2 | 局部=编译期常量;`--theme-*`=运行时主题表;逐节点覆盖继承 P2 |
| `@property`(2024,类型化变量) | ❌ | 变量在编译期就是类型化的(Rust 类型系统),无需注册语法 |
| `env()`(安全区) | ⏳ | 桌面对应物=窗口边衬/标题栏区域,随多平台壳设计 |
| `attr()` 扩展(2025) | ❌ | 无 HTML 属性域 |

## D. 颜色

| 现代 CSS | sv 现状 | 说明 / 替代 |
|---|---|---|
| `#hex`(3/6 位) | ✅ | |
| `rgb()` / `rgba()` | ✅ | 现代空格语法 `rgb(255 62 0 / .5)` 随 C1 |
| 命名色 | ✅ 常用 8 色 | 完整 147 色表 C1 顺手补 |
| `hsl()` / `hwb()` | 📅 C1 | 编译期折叠为 RGBA |
| `currentColor` | 📅 C1 | 随继承管线 |
| `oklch()` / `oklab()` / `lab()` / `lch()`(2023) | ✏️ P2 | 编译期折叠到 sRGB;设计令牌友好,观察需求 |
| `color-mix()`(2023) | ✏️ P2 | 编译期可折叠,常量场景可做 |
| 相对颜色语法 `rgb(from ...)`(2024) | ❌ | 折叠链复杂度不值;Rust 侧 `Color` API 替代 |
| 广色域 display-p3 / `color()` | ⏳ | 依赖渲染后端色彩管理(vello/壳层),M2 后评估 |

## E. 盒模型、边框与背景

| 现代 CSS | sv 现状 | 说明 / 替代 |
|---|---|---|
| `padding`(单值) | ✅ | |
| `padding`/`margin` 四值简写与分向 | 📅 C1 | 编译期展开;margin 字段随盒模型补 |
| `margin`(含 `auto` 居中) | 📅 C2 | 随 taffy |
| `border`(宽/色/样式简写) | 📅 C2 | Style 增 border 通道 + 渲染描边;`dashed/dotted` 样式 P2 |
| `border-radius`(单值) | ✅ | 四角独立值 C1 |
| `box-sizing` | 📅 C2 | 缺省即 border-box(桌面直觉),`content-box` 不做 |
| `outline` | 📅 C2 | 焦点环载体,随焦点链 |
| `box-shadow` | ⏳ | 渲染后端(vello)有高斯模糊后开;CPU 原型不做 |
| `background-color` | ✅ | |
| 渐变 `linear/radial/conic-gradient` | ⏳ | 随 vello 后端(M2);tiny-skia 也可但不提前做 |
| `background-image` / 多背景 / `background-size` | ⏳ | 依赖图片子系统(解码/缓存),独立议题 |
| `aspect-ratio`(2021) | 📅 C2 | taffy 原生支持 |

## F. 布局

| 现代 CSS | sv 现状 | 说明 / 替代 |
|---|---|---|
| `flex-direction` / `gap` | ✅ | 现为自研行列布局 |
| Flexbox 全家(grow/shrink/basis/wrap/justify/align) | 📅 C2 | taffy 字段直通(调研 12 有对照表) |
| Grid 全家(template/areas/span) | 📅 C2 | taffy 0.12 含 `grid-template-areas` |
| `subgrid`(2023) | ⏳ | taffy 未覆盖,跟踪上游 |
| `masonry` 布局(2025 实验) | ❌ | 上游未定稿,不追 |
| `display: block/inline/none` 文档流 | ❌ | 场景树无文档流;`display:none` 的语义由 `{#if}` 承担(更 Svelte) |
| `float` / `clear` | ❌ | 网页排版遗产,无对应物 |
| `position: absolute/fixed/sticky` | ⏳ | 绝对定位随 taffy(C2 可带);sticky 依赖滚动系统(M1 滚动之后) |
| 锚点定位 `anchor()`(2024-25) | ⏳ | 桌面刚需(弹出菜单/tooltip),但要等定位系统;先用 mount+手动坐标 |
| `inset` / 逻辑属性(`margin-inline` 系) | ✏️ P2 | 逻辑↔物理映射编译期做;RTL 支持时升级 |
| `z-index` / stacking context | ⏳ | 场景树按文档序绘制;分层随渲染后端合成层设计 |
| `overflow` / 滚动 | ✅ R2(调研 22) | `overflow: visible\|hidden\|scroll\|auto`(auto=scroll);滚轮 + 最近可滚祖先滚动链、裁剪(CPU 矩形交集/vello 图层)、滚动条 shell 合成绘制、`onscroll`/`bind:scrolly`、virtual_scroll 虚拟化桥;按轴拆分(overflow-x/y)与拖拽滚动条留 C2/档 B |
| `writing-mode` 竖排 | ❌ | 随文本栈(Parley)再议,当前裁 |

## G. 排版

| 现代 CSS | sv 现状 | 说明 / 替代 |
|---|---|---|
| `font-size` | ✅ | |
| `color`(文本色) | ✅ | |
| `font-family` / `@font-face` | ⏳ | 随 Parley/fontique 字体系统(M2);现为系统字体单栈 |
| `font-weight` / `font-style` | 📅 C2 | 需字体子族选择,Parley 前可先假粗斜 |
| `line-height` / `letter-spacing` | 📅 C1/C2 | line-height 进继承白名单 |
| `text-align` | 📅 C2 | 随文本布局 |
| `text-decoration` / `text-transform` | ✏️ P2 | 下划线渲染小;transform 编译期折叠 |
| `text-overflow: ellipsis` / `line-clamp` | ⏳ | 依赖文本测量与截断(Parley) |
| `text-wrap: balance/pretty`(2023-24) | ❌ | Parley 能力边界外,不承诺 |
| 可变字体 `font-variation-settings` | ⏳ | Parley 支持后透传 |

## H. 动效

| 现代 CSS | sv 现状 | 说明 / 替代 |
|---|---|---|
| `opacity` | ✅ | 含祖先链乘积近似组透明 |
| `transition` 属性(值变化插值) | 📅 C2 | computed 值 A→B 插值引擎;与 `transition:fade` 指令(进出场)双轨正交 |
| `@keyframes` / `animation` | ✏️ P2 | 编译成动画数据表接 anim 模块 |
| `transform: translate/scale/rotate` | ⏳ | 渲染后端矩阵通道(vello/M2);动画刚需,优先级高 |
| 3D transform / `perspective` | ❌ | 2D 桌面 UI 裁 |
| 滚动驱动动画 `animation-timeline`(2024-25) | ⏳ | 滚动系统之后 |
| 视图过渡 View Transitions(2023-25) | ⏳ | 对应物=路由/`{#key}` 切换的快照过渡,帧调度+合成层后立项 |
| `@starting-style`(2024) | ✏️ P2 | 进场初值,与 `in:` 指令动机重合,语法可收 |
| `prefers-reduced-motion` | 📅 C2 | 随 @media 通道,接系统无障碍设置 |

## I. 响应式与条件

| 现代 CSS | sv 现状 | 说明 / 替代 |
|---|---|---|
| `@media`(宽高) | 📅 C2 | 窗口尺寸查询 |
| `prefers-color-scheme` | 📅 C2 | 深色主题表(令牌换表) |
| 容器查询 `@container` size(2023) | ⏳ | 桌面面板刚需,但有布局环风险,P2 正式裁决(调研 12) |
| 样式查询 `@container style()`(2024-25) | ❌ | 依赖运行时变量链,与架构相斥 |
| `@supports` | ❌ | 无特性碎片问题:编译期就知道支持什么 |

## J. 视觉效果与杂项

| 现代 CSS | sv 现状 | 说明 / 替代 |
|---|---|---|
| `filter` / `backdrop-filter`(毛玻璃) | ⏳ | 随 vello 后端效果通道(M2);桌面质感刚需,列高优先 |
| `mask` / `clip-path` | ⏳ | 同上,矢量裁剪 vello 可做 |
| `mix-blend-mode` | ❌ | 合成层成本高、UI 低频 |
| CSS counters / `content` | ❌ | 伪元素域;计数写模板表达式 |
| `will-change` / `contain` / `content-visibility` | ❌ | 浏览器渲染提示;retained 树+编译期样式无此优化面 |
| `popover` / `dialog` 配套 CSS(2024) | ⏳ | 对应物=弹层系统(锚点定位同批) |
| `cursor` | 📅 C1 | 桌面必需,shell 已有指针事件基建,顺手做 |
| `user-select` / `pointer-events` | ✏️ P2 | pointer-events:none=命中测试跳过,易做;user-select 随文本选择系统 |

---

## 统计与读法

按上表 91 项:**✅ 12 | 📅 C1 14 | 📅 C2 18 | ✏️ P2 13 | ⏳ 18 | ❌ 16**。

- **C1+C2 完成后**(32 项落地,合计 ✅44/91),覆盖日常桌面 UI 样式书写的绝大多数
  高频面——这正是调研 11 定义的"迁移无感线":真语法、盒模型、flex/grid、继承、
  变量主题、transition、@media、状态伪类。
- **⏳ 的共同特征**是等基建不等设计:渐变/阴影/滤镜/transform 等 vello 后端(M2)、
  滚动/焦点链(M1)、弹层与锚点定位(独立议题)、图片子系统、Parley 文本栈。
- **❌ 的三类**:文档流/浏览器宿主遗产(float/display 流/`@supports`)、被架构替代
  (specificity/!important/@layer/伪元素/运行时选择器)、低频长尾(3D/混合模式/长尾单位)。
  每一条都有文档化替代写法,不是能力黑洞。
