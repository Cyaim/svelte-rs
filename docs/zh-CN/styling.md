**中文** | [English](../en/styling.md)

# 样式:真 CSS 语法的封闭子集

sv 用 `<style>` 块写**真 CSS 语法**——但是一个刻意收口的封闭子集,全部在编译期折叠([../DESIGN.md](../DESIGN.md) ADR-8)。三句话讲清动机:业界的心智迁移分界线在"真 CSS 语法 + 状态伪类 + 变量",不在完备性——React Native 的属性名对象和 Flutter 的零 CSS 都在这条线上付了大代价。Svelte 的 scoped-by-default 又把开发者实际会写的东西压缩到很小的面:扁平类规则、状态伪类、盒模型、变量、继承。于是 sv 解析真 CSS,把每条规则在编译期折叠成 `Style` 字段赋值——**零运行时选择器引擎、零 specificity 计算、零样式重算**;类就是编译期样式表的索引。

这是探索原型,子集按批次扩(C1 已于 2026-07-18 落地,下一批是 C2),API 会变。下文所有内容今天都已实现并有测试覆盖(sv-compiler 测试套里的 `css_c1_box_model_vars_nesting` 等);其余全部在[差距矩阵](#还没有的东西)里逐项记账。

## `<style>` 块

选择器只有两种形态:`.类` 规则和元素类型规则(`view` / `text` / `button` / `checkbox`)。其它写法——后代组合子、`#id`、`[attr]`——目前都是编译错误。

```svelte
<view class="card">
  <text class="card-title">{title}</text>
</view>

<style>
.card { padding: 16px; gap: 8px; border-radius: 10px; background-color: #f0f0f6; }
.card-title { font-size: 20px; color: #223344; }
text { color: #223344; }   /* 元素规则:给组件内所有 <text> 打底 */
</style>
```

规则**按组件 scoped**,和 Svelte 一致:`Stepper.sv` 的 `.btn` 和 `Showcase.sv` 的 `.btn` 是两张互不相干的样式表。元素规则排在类规则之下(熟悉的"元素 < 类"直觉,靠应用顺序实现,不靠 specificity——见[级联](#级联声明序不是-specificity)一节)。同一组件内重复定义同名规则是编译错误。

## 在标记里用样式

| 形式 | 示例 | 说明 |
|---|---|---|
| `class` 属性 | `<view class="card row">` | 只接受静态字符串;多个类按书写顺序合成。未定义的类是编译错误。 |
| `class:` 指令 | `<text class:muted={done}>` | 条件类;简写 `class:muted` 直接读同名变量。 |
| 内联 `style` | `<view style="direction:row; gap:8">` | 与 `<style>` 块共用同一个声明解析器;仅限静态字符串。 |
| 裸样式属性 | `<text font-size="20" fg="#ff3e00">` | 非事件属性一律按单条样式声明解析。 |
| `style:` 指令 | `<text style:fg={expr}>` | 逐字段动态样式的逃生舱,值是 Rust 表达式;见[组件指南](./sv-components.md)。 |

动态的 `class={expr}` 或 `style={expr}` 是编译错误,报错信息会分别引导到 `class:名字={cond}` 和 `style:字段={expr}`。

## 支持的属性集

权威列表就是 `crates/sv-compiler/src/style.rs` 里的那个 `match`,不在表里的键会报编译错误并列出支持面。

| 属性 | 别名 | 取值 |
|---|---|---|
| `background-color` | `background`、`bg` | 任意[颜色形态](#颜色) |
| `color` | `fg` | 颜色、`currentColor`、`inherit`(后两者=继承) |
| `font-size` | `font_size` | 长度 |
| `padding`、`margin` | — | 1–4 个长度,CSS 简写展开(上/右/下/左) |
| `padding-top` … `margin-left` | — | 单长度,8 个长手全有 |
| `border` | — | `none`,或 `[solid] <宽度> [solid] [<颜色>]`——仅实线,颜色缺省黑;`dashed`/`dotted`/`double` 报编译错误(P2) |
| `border-radius` | `corner-radius`、`radius` | 单值(四角独立值 P2) |
| `gap` | `row-gap`、`column-gap`(都写同一个 gap) | 长度 |
| `flex-direction` | `direction` | `row` \| `column` |
| `width`、`height` | — | 长度 |
| `opacity` | — | 数字 0.0–1.0 |
| `cursor` | — | `pointer` / `default` / `text` / `grab` / `not-allowed` |

## 单位

| 单位 | 状态 |
|---|---|
| `px`、裸数 | 逻辑像素,HiDPI 自动缩放 |
| `rem` | 编译期折算成 px(×16) |
| `em`、`%`、`vw`、`vh`、`vmin`、`vmax`、`pt`、`ch` | **设计上的编译错误**,报错会解释原因:`em` 要等动态字号基准(P2),`%`/`vw`/`vh` 要等 taffy 布局系统(C2) |

```
error: 单位 `%` 暂不支持——需要布局系统(taffy,C2);请用 px/rem/裸数(`width: 8px`)
```

## 颜色

所有颜色形态都在编译期折叠为 RGBA。

| 形态 | 示例 |
|---|---|
| 十六进制 3/4/6/8 位 | `#f06`、`#ff3e00`、`#ff3e0080` |
| `rgb()` / `rgba()` | 逗号语法 `rgb(255, 62, 0)` 与现代空格斜杠语法 `rgb(255 62 0 / .5)` |
| `hsl()` / `hsla()` | `hsl(20 100% 50%)`,色相可带 `deg` 后缀 |
| `hwb()` | `hwb(20 10% 5%)` |
| 命名色 | 约 60 个:CSS 基础 16 色 + 常用扩展色(`rebeccapurple`、`hotpink`、`steelblue`…)、`transparent` |
| `currentColor` | 用在 `color` 上时走[继承](#继承)解析 |

alpha 接受数字(`0.5`)或百分比(`50%`)。

## `:root` 变量与 `var()`

```svelte
<style>
:root { --accent: rgb(255, 62, 0); --btn-pad: 8px 14px; }

.btn {
  padding: var(--btn-pad);
  background-color: var(--accent);
  color: var(--missing, white);   /* fallback 形态 */
}
</style>
```

`var(--x)` 是**编译期文本代入**——不存在运行时自定义属性链。`:root` 块可以写在 style 块任意位置;用未定义的变量且没给 fallback 是编译错误。基于变量的运行时主题切换排在 C2。

## 嵌套与状态伪类

CSS 嵌套只支持一种形态:规则内的 `&:hover` / `&:active`。独立写法 `.btn:hover { }` / `.btn:active { }` 同样可用。其它伪类(`:focus`、`:disabled`)是编译错误,随键盘焦点链排在 C2。

```svelte
<style>
.btn {
  background-color: var(--accent);
  cursor: pointer;
  &:hover  { background-color: orange; }
  &:active { opacity: 0.7; }
}
</style>
```

运行时没有任何选择器匹配。对每个带 `:hover` 规则的元素,编译器生成一个私有布尔信号,接到该元素的 pointer-enter/leave 回调上;`:active` 是第二个状态位,接 pointer-down/up。整个元素的样式变成一个响应式闭包(`bind_style`):先铺基础声明,再叠 hover,最后叠 active——按压态最终生效,对齐 CSS 的 LVHA 顺序。你自己写的 `onpointerenter`/`onpointerleave` 会与内部接线合成,不会被覆盖。

## 继承

`color` 和 `font-size` 沿树继承,和网页一致。实现上,`Style { fg: None, font_size: f32::NAN }` 是"继承"哨兵(也是默认值);渲染期沿父链回溯解析,根 fallback 为黑色 / 16.0。写 `color: inherit` 或 `color: currentColor` 会把元素自身的值清回哨兵。其它属性不继承;继承白名单随文本栈扩展(如 `line-height`,C1/C2)。

```svelte
<style>
view { color: #223344; }  /* 下面所有 text 未覆盖时都继承这个色 */
.muted { color: #667; }
</style>
```

## 级联:声明序,不是 specificity

**这是与网页的刻意差异。**没有 specificity 计数,没有 `!important`——组件内谁生效由声明序加一个固定的通道优先级决定:

```
元素规则 < 类(按 class="a b" 顺序) < 内联 style="" < class: 条件类 < :hover < :active < style: 指令
```

这与 CSS 自己在 specificity 相同时的声明序规则一致;Svelte 官方也用 `:where()` 压平 specificity——这套心智是可迁移的。终极覆盖手段是 `style:` 指令。裁决依据见 [../DESIGN.md](../DESIGN.md) ADR-8。

## 还没有的东西

完整账本是 [../CSS-SUPPORT.md](../CSS-SUPPORT.md) 的 91 项差距矩阵:按 2026 年 Baseline 口径的"现代 CSS"逐项裁决。C1 批次落地后的总览:

| 状态 | 数量 | 含义 |
|---|---|---|
| ✅ 已实现 | 24 | 有测试,`examples/showcase` 在用 |
| 📅 C2 已排期 | 19 | flex/grid 走 taffy、`@media`、transition、`:focus`、`%` 单位、margin `auto`… |
| ✏️ P2 降级/编译期形态 | 14 | `calc()` 常量折叠、后代组合子、`oklch()`… |
| ⏳ 等基建 | 17 | 渐变/阴影/滤镜(vello 后端)、字体(parley)…(滚动已于 R2 落地:`overflow: scroll` + 滚轮/滚动条/裁剪) |
| ❌ 永不做,给替代写法 | 16 | specificity、`!important`、`@layer`、伪元素、运行时选择器、`:has()`… |

C2 完成即定义为对 Svelte 开发者的"迁移无感线"(44/91 落地,覆盖高频面)。

想看以上全部特性跑起来:`cargo run -p showcase`(或加 `-- --png out.png` 离屏渲染一帧)。样式源码在 `examples/showcase/src/Showcase.sv`、`Card.sv`、`Stepper.sv`、`TaskRow.sv`。

## 相关阅读

- [组件与模板](./sv-components.md) — `class:`/`style:` 指令的完整上下文
- [../CSS-SUPPORT.md](../CSS-SUPPORT.md) — 91 项差距矩阵全文
- [../DESIGN.md](../DESIGN.md) — ADR-8,这一切背后的决策记录
