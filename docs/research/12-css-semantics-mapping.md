# 12 · CSS 语义逐项映射到 retained 场景树 —— `<style>` 块升级设计蓝图

> 生成:2026-07-17。联网核实:GTK4 CSS 文档、Qt Style Sheets 参考、JavaFX CSS Reference、
> Blitz/Stylo 架构、Svelte scoped-styles 文档与 `:where()` PR、taffy 0.12.2 `Style` 字段
> (docs.rs,2026-07-15 发布)、W3C css-variables-1 / css-transitions-1、MDN 继承属性与
> 属性值处理管线、lightningcss/cssparser 生态。仓库现状以
> `crates/sv-compiler/src/style.rs`、`crates/sv-ui/src/lib.rs`(Style 结构)、
> `crates/sv-compiler/src/codegen.rs`(样式合成序)实读为准。
> 前置阅读:09 号报告 §4(现行极简样式语言的设计动机)、DESIGN.md ADR-6/ADR-7、
> SVELTE-SUPPORT.md F 表。

---

## 0. 结论先行(TL;DR)

**问题**:Svelte 开发者在浏览器写的是真 CSS;sv 现在的 `<style>` 块是自造迷你语法
(9 个封闭键、裸数字、`bg`/`fg`/`radius` 自造名、无继承、无伪类、无 margin)。
迁移者的每一条肌肉记忆都会撞墙。

**总裁决:把 CSS 心智拆成三层,按层收编,而不是二选一("全 CSS 引擎" vs "自造语法")。**

| 层 | 内容 | 裁决 | 成本落点 |
|---|---|---|---|
| **① 语法层** | 标准属性名、简写(`padding: 8px 16px`)、单位(px/em/rem/%/vw)、颜色全格式、`var()`/`calc()` | **基本全收** | 纯编译期展开,运行时零成本 |
| **② 静态语义层** | 继承(color/font-* 沿树)、伪类(`:hover` 等)、盒模型(margin/border)、flex/grid | **真做**(继承是 P0,桌面有真 hover) | 继承走渲染遍历顺路下传;伪类走节点状态位;布局交 taffy |
| **③ 级联引擎层** | specificity 数值计数、任意选择器运行时匹配、`!important`、全局层叠 | **不做**(Svelte scoped 已把级联剪进组件;组件内用"声明序 + 通道优先级"替代) | 省掉整个运行时选择器引擎 |

**关键洞察三条**(均有先例核实,见 §2):

1. **"CSS-on-retained-tree"不是新路**:GTK4 在 retained 部件树上跑真 CSS 子集
   (选择器、`:hover` 全家、`transition`/`@keyframes` 都做了);Qt QSS 照搬 CSS
   盒模型四矩形;JavaFX CSS 沿场景树做继承。桌面场景树吃下大半 CSS 心智
   **被三个十年级工业框架验证过**。sv 的差异化是把匹配尽量挪到编译期。
2. **specificity 在 scoped 世界里是负资产**:Svelte 自己为了不放大 specificity
   专门引入 `:where(.svelte-hash)` 混合策略(PR #10443)。组件内规则通常个位数,
   "同 specificity 靠声明序"这一条 CSS 规则保留即可,数值计数整个砍掉,
   行为反而更接近开发者的直觉预期。
3. **继承不必进响应式图**:场景树渲染/布局本来就有自顶向下遍历,继承属性
   (color/font-size…)作为 `InheritedContext` 顺路下传即可——零新增数据结构、
   零 signal 开销,父值变化靠 doc 版本号触发的重渲染自然生效。这正是浏览器
   computed-style 树遍历的本质,却不需要浏览器的失效引擎。

---

## 1. 现状盘点(改造对象)

**样式通道四条**(优先级从低到高,`codegen.rs:458-599` 实证):

```
<style> 块类(class="a b" 静态、class:x={c} 条件) < style="k:v" 内联 < style:字段={expr} 指令
```

- `Style` 结构 10 字段(`sv-ui/src/lib.rs:61-90`):`direction/gap/padding/bg/fg/
  font_size/width/height/corner_radius/opacity`——padding 是单值 f32,**无 margin、
  无 border、无 min/max 尺寸、无对齐属性**。
- 迷你语法(`style.rs:127-146`):`键:值` 分号分隔,裸数字(无单位),颜色仅
  `#rgb/#rrggbb`,键名自造(`bg`/`fg`/`radius`/`direction`)。
- 类编译为组件内 setter 闭包(`HashMap<String, TokenStream>`),`class="card"` 在
  编译期解析、类名不进运行时——**scoped 是免费且绝对的**,这一点是本设计的基石,
  升级全程保持。
- 无继承:`fg: None` 时渲染器用缺省色,不看父节点(`sv-shell` 渲染路径实证)。
- 条件类存在时整元素样式交给一个 `bind_style` 重算闭包(避免"撤销类"难题)——
  这个"整体重合成"模型正好是伪类/媒体查询要复用的机制。

---

## 2. 先例核查:四个坐标系(联网核实)

| 框架 | 树模型 | 收了哪些 CSS | 砍了哪些 | 对 sv 的启示 |
|---|---|---|---|---|
| **GTK4** | retained 部件树,每部件若干 CSS node | 真 CSS 子集:元素名/类/#名选择器、`:active :hover :disabled :selected :focus :checked` 伪类、margin/padding/border/min-width、`transition`/`animation`/`@keyframes`、颜色/字体属性 | `@media`;大量 web 属性不存在(文档明言"web CSS 属性经常被误以为可用") | 桌面 retained 树能吃下伪类+过渡+盒模型;封闭属性集是共同选择 |
| **Qt QSS** | retained 部件树 | CSS 盒模型**原样照搬**(margin/border/padding/content 四矩形,默认全 0)、选择器+伪状态(可链式 AND)、子控件 | transition/动画、继承弱(靠选择器批量刷) | 盒模型语义直接搬 CSS 不会水土不服 |
| **JavaFX** | retained 场景树 | CSS 2.1 基座:**沿场景树继承**、伪类状态驱动重算(state 变→该节点样式重 apply)、层叠 | 属性全部 `-fx-` 前缀(心智税,反面教材);HSB 而非 HSL;无 font-variant | 继承+伪类驱动重算在场景树上是成熟做法;**不要发明前缀**,标准名直收 |
| **Blitz(Dioxus Native)** | DOM 抽象 + Stylo + taffy + Parley | 全 CSS:复杂选择器、`@media`、CSS 变量(直接内嵌 Firefox 的 Stylo 引擎) | —(代价:浏览器级引擎、pre-alpha) | "全 CSS"路线在 Rust 桌面可行,但那是把浏览器搬进来;sv 的差异化恰是**编译期剪裁**,与零 diff 模板同一哲学 |

另:**Svelte 编译器本身就在编译期做"选择器能否匹配模板"的静态分析**(unused-CSS
警告靠它)。sv 的后代选择器方案(§5.2)就是把这一分析从"警告"升级成"直接打样式"。

---

## 3. 语法层:标准属性名、简写、单位、颜色、var()/calc()

### 3.1 标准属性名 —— 支持(P0,直接替换自造名)

裁决:**属性名 100% 用 CSS 标准名**,自造名废弃(v0 无存量用户,一步切换;
旧键错误信息给 did-you-mean 指向新名)。属性集仍封闭——未知属性是**编译错误**
而非 CSS 的静默忽略(刻意差异,见 §8 兼容表;桌面端"错字被忽略"只会浪费半小时)。

| 现名(废弃) | 标准名 | 备注 |
|---|---|---|
| `bg` | `background-color`(收 `background` 简写的纯色形态) | |
| `fg` / `color` | `color` | 继承属性(§4.2) |
| `radius` / `corner-radius` | `border-radius` | 先单值,四角形态 P2 |
| `direction` | `flex-direction` | 值 `row/column/row-reverse/column-reverse` 照 CSS |
| `padding/gap/width/height/font-size/opacity` | 同名保留 | padding/gap 扩多值简写 |

### 3.2 简写(shorthand)—— 支持(P0,编译期展开)

CSS 简写是纯语法糖,编译期展开成 longhand,运行时零成本:

- `padding: 8px` / `8px 16px` / `8px 16px 4px` / `8px 16px 4px 2px`
  → 上右下左四字段,1/2/3/4 值规则照 CSS(1=全部,2=上下/左右,3=上/左右/下,4=顺时针)。
  `padding-top` 等 longhand 同时收。margin 同规则(P1,§6.1)。
- `gap: 8px 16px` → `row-gap`/`column-gap` 两字段。
- `border: 1px solid #ccc` → width/style/color 三字段,顺序无关(P1)。
- `inset: 0` → top/right/bottom/left(P2,随 position)。
- `flex: 1` → `flex-grow:1; flex-shrink:1; flex-basis:0%`(CSS 规范展开,P1)。
- `transition: opacity 200ms ease-out` → 四 longhand(§7.1)。

实现要点:`Style` 中所有四边属性用 `Rect<T>{top,right,bottom,left}`(与 taffy 同构,
迁移即零转换);简写展开器是一个纯函数表,直接从 CSS 规范抄分配规则。

### 3.3 单位 —— 支持 px/em/rem/%/vw/vh(分两档)

裁决:**长度必须带单位,裸数字报错**(错误信息提示"是否想写 8px?")。理由:
CSS 心智的核心场景是**从浏览器项目里复制粘贴样式片段**,写 CSS 的人不写裸长度;
反向粘贴(sv → web)也成立。仅 `opacity`、`flex-grow/shrink`、`line-height`(倍数
形态)、`z-index` 等 CSS 本来就收纯数的属性收纯数——与 CSS 完全一致。
(现有 9 键裸数字语法在切换版本一次性迁移,编译器给自动改写建议。)

| 单位 | 语义 | 解析时机 | 实现 |
|---|---|---|---|
| `px` | 逻辑像素(× scale factor,HiDPI 已有) | 编译期折成 f32 | P0 |
| `rem` | 根字号倍数(应用级 `root_font_size`,默认 16,可运行时改→全局缩放免费实现) | resolve 期(§4.2) | P0 |
| `em` | **当前元素 computed font-size** 倍数;用在 `font-size` 自身时相对**父**字号(CSS 规则) | resolve 期,依赖继承链,font-size 先算 | P0 |
| `%` | 布局属性→taffy `LengthPercentage` 原生支持;`font-size: %` 相对父字号 | taffy / resolve 期 | P1(随 taffy) |
| `vw/vh` | 窗口逻辑尺寸百分比 | resolve 期,窗口 resize 触发重算(与 `@media` 同一失效源,§7.2) | P1 |
| `vmin/vmax/ch/ex/cm…` | — | — | 砍(需求出现再加,错误信息说明) |

实现要点:编译产物里长度不再是 `f32` 而是小枚举
`Length { Px(f32), Em(f32), Rem(f32), Percent(f32), Vw(f32), Vh(f32), Auto }`
(`Copy`,8 字节);纯 px 场景编译器可仍折成 `Length::Px` 常量,无性能回退。
specified → computed 的折算集中在 resolve 遍历一处(§4.2)。

### 3.4 颜色 —— 全格式支持(P0,全部编译期折叠)

hex `#rgb/#rgba/#rrggbb/#rrggbbaa`、`rgb()/rgba()`(含现代空格斜杠语法
`rgb(255 62 0 / .5)`)、`hsl()/hsla()`(编译期转 RGB)、147 个 CSS 命名色
(`red/rebeccapurple/transparent`,编译期常量表)、`currentColor`(= 取 computed
`color`,resolve 期解引用,继承机制的免费搭车)。全部折成现有
`Color{r,g,b,a}`,运行时零解析。`oklch()` 等新空间 P2 观察(桌面设计工具
输出趋势),`color-mix()` 砍。

### 3.5 `var()` 自定义属性 —— 降级支持,拆两档(P2)

CSS 的自定义属性是**继承驱动的运行时机制**(css-variables-1:`var()` 在
computed-value time 替换,替换发生在继承传递之前)。全语义照搬意味着每个节点
携带一张可继承的变量表——成本高且 90% 用法用不到。拆三档:

1. **组件局部常量**(P0):`<style>` 内 `:root { --accent: #4a9eff; }` +
   `var(--accent)` → **编译期直接替换**,零运行时痕迹。覆盖"魔法数命名"这一
   最大用法。(替代 09 §4 的自造 `@tokens` 语法——同能力,CSS 拼写。)
2. **应用主题令牌**(P1):`var(--theme-accent)` 查运行时主题表(名字→索引,
   编译期定死),主题表可整体切换(深色模式)且是响应式数据 → 引用处自动重算。
   `@media (prefers-color-scheme: dark)` 内的 `:root` 覆盖块编译为深色主题表(§4.4)。
3. **逐节点可覆盖 + 沿树继承的完整语义**(P2):等继承管线(§4.2)稳定后,
   把"变量表"做成 InheritedContext 的一个扩展槽(`SmallVec<(TokenId, Value)>`,
   TokenId 编译期分配)。`var(--x, fallback)` 的 fallback 语法一并收。
   在此之前,模板里逐实例覆盖的需求已有 `style:` 指令 + props 承接。

### 3.6 `calc()` —— 降级支持(P2)

- 纯常量 `calc(8px * 2)`:编译期折叠,免费(P1 顺手)。
- 混合单位 `calc(100% - 32px)`:CSS 也是留到 computed/used-value 阶段才折
  (MDN 属性值处理管线);sv 对应物是保留一棵小表达式树
  (`CalcExpr { Add/Sub/Mul/Div, Length 叶 }`)进 resolve 期,在参照(父尺寸/
  字号/视口)已知处折成 px。布局属性上的 calc 需要 taffy 配合(taffy 的
  calc 支持走宿主回调,需 spike 验证)。P2;在此之前 `style:` 指令里写 Rust
  表达式是全能逃生舱(这正是桌面比浏览器强的地方——真语言就在手边)。

---

## 4. 级联与作用域:specificity、继承、设计令牌

### 4.1 specificity —— 砍数值计数,保留"声明序"心智(P0 文档化)

Svelte scoped 样式已经把级联剪到组件内:跨组件不可能选中(sv 更绝对——类名
编译为索引,运行时根本没有类名)。组件内部剩下的冲突只有一种:**同一元素被
多个来源写同一属性**。裁决:不做 0-1-0 计数,用**确定性合成序**:

```
Style::default()                    (元素缺省)
  ⊕ 继承注入                        (inherited 白名单,仅未显式设置的字段,§4.2)
  ⊕ <style> 块激活类,按块内声明序   (静态 class + 激活的 class: 条件类)
  ⊕ 伪类变体 patch                   (基类激活且状态位为真;紧随其基类)
  ⊕ @media 激活块 patch              (§7.2)
  ⊕ style="" 内联静态
  ⊕ style: 指令                     (最高;对应 Svelte 里 style: 覆盖内联样式)
```

与 CSS 心智的对齐度逐条核对:
- "同 specificity 后声明胜" → 类按声明序叠加,**一致**;
- "内联 style 压过任何选择器" → 内联/指令在类之上,**一致**;
- "`.btn:hover` 压过 `.btn`"(0-2-0 > 0-1-0)→ 伪类 patch 在基类之后,**效果一致**;
- "`.a.b` 压过 `.a`"、ID 选择器竞赛 → **不存在**(无 ID 选择器,复合选择器 P2 前不收);
- `!important` → **砍**(无级联战争即无需核武;Svelte 里它也主要用于对抗第三方全局样式,sv 没有全局样式)。

未来若加后代选择器(§5.2),用两级规则"带祖先限定 > 无限定",仍不做数值计数。
写进文档的一句话心智模型:**"在 sv 里,后写的赢;内联赢过类;指令赢过一切。"**
——这恰是多数 CSS 开发者以为 CSS 是怎么工作的。

### 4.2 继承 —— 真做,P0(本蓝图最重要的单项)

`color`/`font-*` 沿树继承是 CSS 心智的地基(在 `.card` 上设 `color`,里面的文本
全变色);当前 sv 无继承,是迁移者遇到的第一堵墙。设计:

**继承白名单**(MDN 核实的 CSS 继承属性 ∩ 桌面适用):

| 属性 | 状态 | 属性 | 状态 |
|---|---|---|---|
| `color` | P0 | `text-align` | P1(taffy 0.12 有 text_align) |
| `font-size` | P0(em/rem 参照) | `letter-spacing` | P1(随 Parley) |
| `font-family` | P1(随 Parley/fontique) | `line-height` | P1(随文本换行) |
| `font-weight` / `font-style` | P1(随 Parley) | `cursor` | P1(桌面刚需,winit 有 API) |
| `visibility` | P2 | `white-space`/`word-spacing`/`text-transform` | P2(随文本系统) |

非继承属性(布局/盒模型/背景/边框)一律不继承——与 CSS 一致,无需动作。

**实现方案:specified/computed 两层 + 渲染遍历顺路 resolve。**

1. `Style`(specified)的继承字段改为"可未设置":`fg: Option<Color>` 已经是了;
   `font_size: f32` 改 `Option<Length>`。语义:`None` = `inherit`(继承属性的
   CSS 初始行为)。显式 `inherit`/`initial` 关键字 P2 再收(`None`/字段缺省已覆盖语义)。
2. 新增 `ComputedStyle`(全字段已折算:长度全 px、颜色全 RGBA、继承已注入)。
3. **resolve pass**:在 layout 前自顶向下遍历一次场景树,携带
   `InheritedContext { color, font_size, /* P1: font, text_align, cursor … */ }`:
   节点有显式值 → 折算(em/rem/% 在此折,font-size 先算再供 em 用)并更新
   context 下传;无 → 直接取 context。产出 `ComputedStyle` 存节点(或平行数组)。
   **顺序保证**:resolve → taffy layout(要 computed 长度)→ paint(要 computed 颜色)。
4. **失效模型:不进响应式图**。任何样式写入已经 bump doc 版本 → 下一帧重跑
   resolve pass。整树 resolve 是 O(n) 纯计算(每节点几十字节拷贝),对桌面场景树
   (千级节点)完全无感;等真出现万级节点再加"子树 dirty 位"剪枝(节点已有
   parent 链,标脏上溯即可),**不要提前做浏览器式失效引擎**。
5. `currentColor`、`em`、`var()` P2 档、未来 `rem` 动态改根字号,全部搭这趟车。

代价与边界:props 传值、`{@const}` 等响应式路径完全不受影响(继承发生在
render 数据面,不在 signal 图);`style:color={expr}` 指令写入的是 specified 层,
子节点照常继承到——**指令与继承自动正确组合,无特例**。

### 4.3 `:root` / 设计令牌 —— 支持(P0 局部 / P1 主题)

见 §3.5 的三档。语法全部用 CSS 拼写(`:root { --x: v; }` + `var(--x)`),
09 §4 的自造 `@tokens`/`@theme.*` 语法**废弃**——同能力下,CSS 拼写的迁移成本为零,
且设计师/AI 工具输出的就是这个形态。

### 4.4 深色模式令牌(P1)

```css
:root { --surface: #ffffff; --ink: #1a1a1a; }
@media (prefers-color-scheme: dark) {
  :root { --surface: #202028; --ink: #e8e8e8; }
}
```

编译为亮/暗两张主题表 + 运行时一个 `Signal<ColorScheme>`(M2 深色模式路线图
已有);引用 `var(--surface)` 的样式编译为查表调用,scheme 翻转 → 引用处重算。
这是"var() 运行时档"的第一个真实用户,也是它的验收标准。

---

## 5. 选择器:在"类 = 编译期索引"模型下逐项裁决

核心原则:**选择器匹配从运行时挪到编译期**——模板树在编译期是完全已知的
(这是 SFC 编译器路线独有的资产,浏览器没有),静态可判的选择器直接把 patch
打到节点上;运行时只剩布尔状态位的组合。

| 选择器 | 裁决 | 实现要点 |
|---|---|---|
| `.class` | ✅ 已有(P0 保持) | 编译期索引,运行时无类名 |
| 多类复合 `.a.b` | P2 | 编译期:两类都静态 → 直接合成;含条件类 → 编译成 `a_active && b_active` 的条件 patch(§5.1 同机制) |
| 元素选择器 `text` / `button` | P2(搭车) | 编译期按 `ElementKind` 匹配模板节点,免费;组件内批量排版有用(`.card text { color: … }`) |
| 后代 `.card .title` | **降级支持,P2**(§5.2) | 编译期静态匹配 + 条件类桥接 |
| 子代 `>` | P2(与后代同机制) | 编译期树结构已知,child 与 descendant 同价 |
| 兄弟 `~` `+` | ❌ 砍 | 用得少;高频场景(列表分隔线)由 `:first-child`/gap 覆盖 |
| `:hover :active :focus :disabled` | **✅ 支持,:hover/:active P0** | §5.3,节点状态位 + patch 组 |
| `:focus-visible` | P1(随焦点链) | 桌面语义精准:键盘焦点才显环;鼠标点击不显——直接采 CSS 现代口径 |
| `:first-child :last-child` | 降级支持,P2 | 静态模板位置编译期判;`{#each}` 行由 keyed reconcile 维护首末标志位,插删时更新两行 |
| `:nth-child(odd/even)` | P2(斑马纹高频) | each 行索引已有;行序变化时受影响行重合成 |
| `:nth-child(An+B)` 任意公式 | ❌ 砍 | 收益/复杂度失衡;`{#each}` 里 index 就在手边,`class:` 一行解决 |
| `:not() :is() :has()` | ❌ 砍(v0/v1) | `:has()` 浏览器自己都刚落地;`class:` 条件类是显式替代 |
| `::before ::after` | ❌ 砍 | 无生成内容机制;桌面里"装饰节点"就写在模板里(模板不是稀缺品,不需要藏进 CSS) |
| 属性选择器 `[x=y]` | ❌ 砍 | 无属性字符串集;`class:` 覆盖 |
| ID `#x` | ❌ 砍 | scoped 组件内 ID 无意义 |
| `:global()` | ❌ 维持不做(F 表既有裁决) | 跨组件外观走 props/令牌 |
| `*` 通配 | ❌ 砍 | 组件内节点可枚举;`view { }` 元素选择器覆盖"给所有容器设值"的需求 |

### 5.1 条件类参与匹配的统一机制

现状 codegen 已经在"有条件类时整元素交给一个重算闭包"。把这个模型推广为:
**每个元素的样式 = 一个纯函数 f(激活类集合, 状态位, 媒体位, 指令值)**,任何输入
变化 → 重算该元素(相等剪枝已有)。伪类、@media、后代条件桥接,全都是往这个
函数里加布尔输入——**永远不需要选择器引擎**,复杂度封在编译期。

### 5.2 后代选择器 `.card .title` 的编译期匹配(P2)

- 编译期对模板树跑选择器匹配(Svelte 编译器同款分析,它用来做 unused-CSS 警告,
  sv 用来直接打样式):`.card .title` → 找到所有"祖先带 card 类、自身带 title 类"
  的节点,把 patch 静态合入该节点。
- 祖先/自身任一方是 `class:` 条件类 → 编译为 `card_active && title_active` 条件
  patch(§5.1 的布尔输入)。
- 匹配不跨组件边界、不跨 snippet 实参(scoped 语义);`{#if}/{#each}` 内部照常
  匹配(结构编译期已知)。
- 匹配不到任何节点 → 编译警告(对齐 Svelte unused-CSS)。
- 明确的天花板(文档写死):**选择器只看模板结构,不看运行时树**。动态 mount
  进来的子树不参与父组件选择器——这在 scoped 模型下本来就该如此。

### 5.3 交互伪类(P0 主菜:桌面有真 hover)

`shell` 已派发 pointer enter/leave(showcase 有行为测试),万事俱备:

1. 类编译产物从"setter 闭包"升级为
   `StyleClass { base, hover, active, focus, disabled: Option<Patch> }`(09 §4
   已有此设计,语法从自造 `:pressed` 改回 CSS 拼写 `:active`)。
2. `ViewNode` 加 `InteractionState` 位集(`HOVERED/PRESSED/FOCUSED/DISABLED`);
   shell 事件层置位/清位(hover 已有;pressed 挂 mouse down/up;focused 待 M1
   焦点链;disabled 由未来控件属性置)。
3. 状态位变化 → 该节点样式重合成(§5.1 函数多一个输入)→ 相等剪枝 → 重绘。
   零选择器匹配、零全树扫描——JavaFX"状态变则该节点重 apply"的同款,更省。
4. `:disabled` 同时具备语义(禁点击)与样式两面,随控件系统(M1)接通。
5. cursor 属性与 `:hover` 是一对(`cursor: pointer`),P1 一并上(winit 直通)。

---

## 6. 盒模型与布局:margin/border/box-sizing + taffy 对照表

### 6.1 盒模型(P1,taffy 落地同批)

- **margin**:当前完全没有——加,`Rect<LengthPercentageAuto>` 直通 taffy;
  `margin: auto`(flex 里居中/推挤)taffy 原生支持,CSS 心智重要惯用法,收。
  v0 CPU 行列布局也可先加简化版(偏移即可),不必等 taffy。
- **border**:width(四边独立)+ color + style(v0 只 `solid`,`dashed/dotted` P2
  随渲染)。参与布局(taffy `border` 字段)+ 绘制(tiny-skia stroke / vello)。
  `border-radius` 已有(改名)。`outline`(不占布局的焦点环)P1 与 `:focus` 配套
  ——桌面可用性刚需。`box-shadow` P2(vello 后端);tiny-skia 阶段可做低配版
  (实心偏移影)。
- **box-sizing**:裁决——**默认 `border-box`**,提供 `content-box`(taffy 0.12 有
  `box_sizing` 字段,一行直通)。与 web 默认相反,但每个 web 项目第一行 reset 就是
  `* { box-sizing: border-box }`——采用开发者**实际生活**的默认值,兼容表里标注即可。

### 6.2 display:flex/grid 属性 → taffy 0.12 字段对照表(联网核实 docs.rs)

| CSS 属性 | taffy `Style` 字段 | 裁决/备注 |
|---|---|---|
| `display: flex / grid / none` | `display: Display` | P1;`none`=从布局摘除(与 `{#if}` 互补:保留状态只藏外观);**`block` 不收**——桌面无文档流,这是刻意差异(§8) |
| `flex-direction` | `flex_direction` | P1;缺省 **`column`**(桌面心智,与现状一致;web 缺省 row,兼容表标注) |
| `flex-wrap` | `flex_wrap` | P1 |
| `flex-grow / shrink / basis` | `flex_grow / flex_shrink / flex_basis` | P1;`flex: 1` 简写展开 |
| `justify-content` | `justify_content` | P1 |
| `align-items / align-self / align-content` | `align_items / align_self / align_content` | P1 |
| `justify-items / justify-self` | `justify_items / justify_self` | P2(grid 专属) |
| `gap / row-gap / column-gap` | `gap: Size<LengthPercentage>` | P0 有单值,P1 双值 |
| `width / height / min-* / max-*` | `size / min_size / max_size: Size<Dimension>` | P1(含 %/auto) |
| `aspect-ratio` | `aspect_ratio: Option<f32>` | P1(免费) |
| `padding` | `padding: Rect<LengthPercentage>` | P0 简写,P1 接 taffy |
| `margin` | `margin: Rect<LengthPercentageAuto>` | P1(含 auto) |
| `border-width` | `border: Rect<LengthPercentage>` | P1 |
| `box-sizing` | `box_sizing: BoxSizing` | P1,缺省 border-box |
| `position: relative / absolute` + `top/right/bottom/left` | `position: Position` + `inset: Rect<LengthPercentageAuto>` | P1(悬浮/角标刚需);`fixed/sticky` P2(fixed≈相对窗口,待滚动) |
| `overflow` | `overflow: Point<Overflow>` | P1 声明,滚动行为随 M1 滚动系统 |
| `grid-template-columns/rows`(`fr`/`repeat()`/`minmax()`/`auto-fill`) | `grid_template_columns/rows` | P2;taffy 忠实实现 CSS Grid 规范,fr/minmax/repeat 全有 |
| `grid-column / grid-row`(线号/`span n`) | `grid_column / grid_row: Line<GridPlacement>` | P2 |
| `grid-auto-flow / auto-rows / auto-columns` | `grid_auto_flow / grid_auto_rows / grid_auto_columns` | P2 |
| `grid-template-areas` | `grid_template_areas`(taffy 0.12 已支持) | P2(设计师最爱的 ASCII 布局,值得要) |
| `text-align` | `text_align: TextAlign` | P1,兼作继承属性 |
| `float / clear` | taffy 0.12 有 | ❌ **不做**——float 是文档流历史债,桌面 UI 无此心智 |

结论:**taffy 就是"CSS 布局属性面"的现成答案**,sv 要做的只是把 `<style>` 解析的
声明喂进 `taffy::Style`——对照表里 P1 列几乎全部是"直通字段",没有翻译损耗。
唯一的裁决点是缺省值(`flex-direction: column`、`box-sizing: border-box`、
`display` 只有 flex/grid/none),三处全部写进兼容表。

### 6.3 布局外盒属性

`z-index`(P2,paint 序;场景树子序即缺省序,z-index 做同层重排)、
`transform`(P2/M2,vello 变换免费,但命中测试要跟着变——先收 `translate/scale/rotate`
简单形态)、`clip-path`/`filter`(❌ 砍,M2 后随渲染能力再议)。

---

## 7. 动态:transition、@media、@keyframes

### 7.1 `transition` 属性 —— 支持(P1),与 `transition:fade` 指令双轨并存

两者在 Svelte 世界本来就**语义正交**,不冲突:

| | CSS `transition` 属性 | Svelte `transition:` 指令 |
|---|---|---|
| 触发 | 属性值 A→B 变化(hover 变色、勾选变灰) | 元素**进出场**(创建/销毁) |
| CSS 能否做 | ✅ | ❌(display:none 化的销毁瞬间,CSS 过渡不了——Svelte 指令正为此存在) |
| sv 对应 | **新增**:computed 值变化时插值 | 已有 `transition:fade / in:fade`(F 表第五批) |

语法照 CSS:`transition: background-color 150ms ease-out, opacity 200ms;`
(property/duration/timing-function/delay 四段简写 + 逗号多组 + `all`,
css-transitions-1 口径)。实现要点:

1. 声明是纯静态数据(热重载数据面友好):
   `Vec<TransitionDecl { prop_id, duration, easing, delay }>` 进 `Style`。
2. 插值引擎放在 **computed 层之后**:样式重合成产出新 `ComputedStyle` 时,
   diff 出被声明覆盖的变化字段,不直接写值,改为向 `anim` 模块(已有 opacity
   通道 + 帧泵)注册"从当前呈现值 → 目标值"的通道。呈现值(presentation value)
   与 computed 值分离——中途反向(hover 进出快速抖动)从当前呈现值重新出发,
   与浏览器行为一致。
3. 可插值属性白名单:`opacity`、颜色(RGBA 分量线性插值,P2 换感知空间)、
   长度类(padding/margin/width/height/gap/border-radius/font-size)、
   未来 transform。离散属性(flex-direction 等)不插值——与 CSS"无中间态
   不可过渡"的规则一致。
4. easing:`linear / ease / ease-in / ease-out / ease-in-out / cubic-bezier(a,b,c,d)`,
   编译期解析成四参数,运行时一个求值函数。
5. 与帧调度(ADR-6)的关系:anim 帧泵就是 transition 的时钟;ADR-6 落地前用
   现有 anim 泵即可先通,不必互相等。

### 7.2 `@media` —— 支持窗口尺寸/主题查询(P1)

桌面语义:viewport = 窗口。可自由缩放的窗口让 `min-width/max-width` 查询成为
**真需求**(窄窗折叠侧栏),这不是从 web 硬搬:

- 收:`(min-width: …px)` `(max-width: …px)`(逻辑像素)、`(prefers-color-scheme: dark)`
  (§4.4)、`(prefers-reduced-motion)`(无障碍,transition 全局开关,免费且体面)。
- 砍:打印/分辨率/hover 能力查询等 web 环境探测(桌面恒真/恒假,编译警告)。
- 实现:每个 `@media` 块编译为"媒体位"(布尔),窗口 resize/主题变化时重求值
  (纯比较,零成本);块内类 patch 挂在该位上——又是 §5.1 合成函数的一个布尔
  输入。媒体位翻转 → 引用该块的元素重合成。**没有运行时选择器,也没有样式表
  重算——只有受影响元素的函数重跑。**
- `@container`(容器查询):桌面组件复用场景其实比 @media 更对口,但布局后
  才知尺寸→样式改布局的环(浏览器靠 containment 约束才敢做)。P2 观察项,
  先用"组件 props 显式传尺寸档位"顶住,不承诺。

### 7.3 `@keyframes` —— 降级支持(P2)

loading spinner、脉冲提示是桌面真需求,且 Svelte 用户就是在 `<style>` 里写
`@keyframes` 的。关键帧是纯数据(热重载数据面友好),实现骑在 transition 同一
插值引擎上:多关键帧 + 迭代 + 方向。收
`animation: name duration timing iteration-count` 四项核心
(`fill-mode/play-state/direction` P2 后半);关键帧内属性限可插值白名单。
`transform: rotate()` 是 spinner 刚需 → 与 §6.3 transform 简单形态绑定排期。

---

## 8. CSS 心智兼容表(交付物 1)

> 读法:「你在浏览器已经会的」→ 在 sv `<style>` 里的遭遇。
> ✅ 原样可用 · ✏️ 改写(语法同、语义有刻意差异,已文档化) · ⛔ 不适用(有替代)

| 你已会的 CSS | 裁决 | 说明 / 替代 |
|---|---|---|
| 标准属性名(color/padding/border-radius…) | ✅ P0 | 属性集封闭;**未知属性=编译错误+did-you-mean**(web 是静默忽略——桌面选报错) |
| 简写四值系(padding/margin/gap/border/flex/transition) | ✅ P0/P1 | 编译期展开,规则照抄 CSS |
| px / em / rem / % / vw / vh | ✅ P0/P1 | 逻辑像素自动 HiDPI;vmin/ch 等长尾砍 |
| 无单位长度非法 | ✅ | 与 CSS 一致(opacity/flex-grow 等纯数属性除外) |
| hex/rgb()/hsl()/命名色/currentColor | ✅ P0 | 全编译期折叠;oklch P2 观察 |
| `:root { --x } + var(--x)` | ✏️ P0/P1/P2 三档 | 局部=编译期常量;`--theme-*`=运行时主题表;逐节点覆盖+继承是 P2 |
| `calc()` | ✏️ P2 | 常量折叠先行;混合单位进 resolve 期;急用时 `style:` 指令写 Rust 表达式全能替代 |
| color/font-* 沿树继承 | ✅ P0 | **新增继承管线**(渲染遍历顺路 resolve,白名单见 §4.2) |
| specificity 计数 / `!important` | ⛔ | scoped 小世界用"声明序+通道优先级"(§4.1);行为与你的直觉一致:后写的赢、内联赢、指令赢 |
| `.class` 与多类叠加 | ✅ P0 | 声明序合成;类是编译期索引,scoped 绝对 |
| `.card .title` 后代选择器 | ✏️ P2 | **编译期**对模板匹配(不跨组件、不看运行时树);匹配不到=警告 |
| `:hover :active` | ✅ P0 | 桌面真事件位驱动,单节点重合成 |
| `:focus / :focus-visible / :disabled` | ✅ P1 | 随焦点链/控件系统 |
| `:first/:last-child`、`odd/even` | ✏️ P2 | 静态位置编译期判;each 行由 keyed reconcile 维护标志位;任意 An+B 砍 |
| `::before/::after`、属性/ID/兄弟选择器、`:has()` | ⛔ | 装饰节点直接写模板;条件用 `class:`;`:global` 亦不做 |
| margin(含 auto)/ border / outline | ✅ P1 | 当前没有,随 taffy 落地 |
| `box-sizing` | ✏️ P1 | **缺省 border-box**(web 缺省 content-box,但你项目里第一行 reset 就是它) |
| `display: flex` 全属性面 | ✅ P1 | taffy 直通(§6.2 对照表);缺省 `flex-direction: column`(web 是 row) |
| `display: grid` 全家(fr/minmax/repeat/areas) | ✅ P2 | taffy 忠实实现 CSS Grid,含 template-areas |
| `display: block` / float / 文档流 | ⛔ | 桌面无文档流;一切皆 flex/grid(文本段内换行属文本布局,另章) |
| `position: absolute + inset` | ✅ P1 | taffy 直通;fixed/sticky P2 |
| `transition` 声明式过渡 | ✅ P1 | computed→呈现值插值引擎;与进出场的 `transition:fade` 指令**双轨并存,语义正交** |
| `@media (min-width)` | ✏️ P1 | viewport=窗口;媒体位驱动重合成;环境探测类查询砍 |
| `@media (prefers-color-scheme/reduced-motion)` | ✅ P1 | 深色主题表切换 / 动画全局降级 |
| `@keyframes` + `animation` | ✏️ P2 | 核心四项;骑 transition 插值引擎 |
| `transform / z-index / box-shadow` | ✏️ P2 | 随渲染层(vello);shadow 有低配先行版 |
| `@container` | ⛔(观察) | 布局环风险;props 传尺寸档位顶住 |
| `@import` / 外链样式表 / CSS-in-JS | ⛔ | 组件即作用域;跨组件=令牌/props(F 表既有裁决) |

**一句话总结给迁移者**:属性名、单位、颜色、盒模型、flex/grid、`:hover`、
`transition`、`@media`、变量——你都会,照写;你**不需要**再会的是 specificity
战争、`!important`、float 布局和选择器杂技;唯二要适应的是"未知属性会报错"
(帮你的)和"缺省纵向 flex"(桌面的)。

---

## 9. 分阶段实现清单(交付物 2)

### P0 —— `<style>` 块语法升级 + 继承(不依赖 taffy,现 CPU 布局即可全做)

1. 词法/解析器重写:声明级 CSS 语法(属性名标准化、必须带单位、简写展开器、
   颜色全格式、`/* */`);**继续自写解析器**(属性面小、错误定位风格统一、
   零依赖),用 lightningcss 做差分测试基准而非依赖(其编译时间问题有公开
   issue,且全属性库对封闭属性集是杀鸡牛刀;cssparser 若 P1 语法面扩大再评估)。
2. `Length` 枚举 + `Rect<T>` 四边字段进 `Style`;specified/computed 分层。
3. **继承管线**:resolve pass(InheritedContext 下传)、白名单 color/font-size、
   em/rem 折算、`currentColor`。
4. `:hover` / `:active`:`StyleClass` patch 组 + `InteractionState` 位集 + 事件置位。
5. `:root { --x }` 局部常量(编译期替换)。
6. 合成序文档化(§4.1)+ 未知属性 did-you-mean + unused-class 警告。
7. 迁移:9 个旧键一次性切标准名,编译器给逐条改写建议。

### P1 —— 盒模型/布局/动态(taffy 落地同批,M1-M2 路线图对齐)

1. margin(含 auto)/ border / outline / box-sizing(缺省 border-box)。
2. flex 全属性面 + width/height/min/max/% / aspect-ratio / position+inset /
   overflow 声明(§6.2 直通表)。
3. `transition` 属性:声明解析、呈现值通道、可插值白名单、cubic-bezier。
4. `@media` 窗口宽高 + `prefers-color-scheme`(主题表)+ `prefers-reduced-motion`。
5. `:focus/:focus-visible/:disabled`(随焦点链与控件)、`cursor` + 继承白名单
   扩到 font 家族/text-align(随 Parley/文本系统)。
6. vw/vh;`calc()` 常量折叠。

### P2 —— 选择器扩展与长尾

1. 后代/子代/元素/复合选择器的编译期匹配 + 条件类桥接。
2. `:first/:last-child`、`odd/even`(keyed each 标志位)。
3. grid 全家(含 template-areas)。
4. `var()` 逐节点继承档 + fallback;`calc()` 混合单位(resolve 期表达式树,
   taffy calc 回调 spike)。
5. `@keyframes`/`animation` 核心;transform 简单形态(spinner);z-index;box-shadow。
6. `@container` 评估报告(做/不做的正式裁决)。

### 与既有里程碑的钩挂

P0 可立即开工(仅动 sv-compiler/sv-ui/sv-shell 渲染读点);P1 与 M1 的 taffy、
焦点链、滚动同一波;P2 的选择器编译期匹配依赖 M1"编译器核心合并 + Template
数据化"(匹配结果要进模板数据面才能热重载)。**样式块始终保持 100% 静态数据**
(热重载数据面承重墙,09 §5 纪律不破):本蓝图所有运行时行为(继承/伪类/媒体/
过渡)的输入都是数据表 + 布尔位,无一处需要把表达式编译进样式。

---

## 10. 风险与开放问题

1. **em 的完整语义成本**:em 参照"自身 computed font-size"意味着长度折算必须
   在继承 resolve 之后——目前设计已排对顺序,但 taffy 的 % 与 em 混用
   (`width: 50%` + `padding: 1em`)要求 taffy 输入前全部折成 px,**% 例外**
   (taffy 自己算)。需要在实现时明确"哪些单位 resolve 期折、哪些透传 taffy"。
2. **`transition` 与继承的交互**:父 `color` 过渡时,子继承到的是每帧插值中的
   呈现值还是目标值?CSS 口径是继承 computed(目标)值、各自独立过渡——
   建议照抄(子无 transition 声明则瞬变),写测试钉住。
3. **窗口 resize 风暴**:vw/@media 在连续拖拽 resize 时每帧重求值+重合成,
   需要与帧调度(ADR-6)合并节流;v0 先靠相等剪枝顶住。
4. **taffy calc 回调**的实际形态未 spike(P2 前完成)。
5. **`:active` 的按压语义**:桌面按下后移出元素再松开,CSS `:active` 的清除
   时机有浏览器差异;定义为"按下且指针在内"并写进测试。
6. **样式表体积**:patch 组(base+4 伪类+媒体变体)按值内联进组件函数会放大
   代码;Template 数据化(M1)后应转为组件级静态样式表 + 索引,与热重载
   数据面同一存储。
7. 编号说明:本报告为 12 号;11 号由并行任务占用。

## 11. 来源

- GTK4:CSS in GTK 概览 https://docs.gtk.org/gtk4/css-overview.html ;
  CSS 属性表 https://docs.gtk.org/gtk4/css-properties.html
- Qt Style Sheets:盒模型 https://doc.qt.io/qt-6/stylesheet-customizing.html ;
  语法/伪状态 https://doc.qt.io/qt-6/stylesheet-syntax.html
- JavaFX CSS Reference(场景树继承/伪类/-fx- 前缀)
  https://openjfx.io/javadoc/11/javafx.graphics/javafx/scene/doc-files/cssref.html
- Blitz(Stylo+taffy+Parley,全 CSS 路线)https://github.com/DioxusLabs/blitz ;
  stylo_taffy https://lib.rs/crates/stylo_taffy
- Svelte scoped styles 与 `:where()` 混合策略:
  https://svelte.dev/docs/svelte/scoped-styles ;
  https://github.com/sveltejs/svelte/pull/10443 ;
  https://geoffrich.net/posts/svelte-scoping-where/
- taffy 0.12.2 `Style` 字段(2026-07-15)https://docs.rs/taffy/latest/taffy/struct.Style.html
- W3C css-variables-1(var() computed-value time)https://www.w3.org/TR/css-variables-1/ ;
  css-transitions-1 https://www.w3.org/TR/css-transitions-1/
- MDN:属性值处理管线(specified/computed/used)
  https://developer.mozilla.org/en-US/docs/Web/CSS/Guides/Cascade/Property_value_processing ;
  继承 https://web.dev/learn/css/inheritance/ ;
  transitions https://developer.mozilla.org/en-US/docs/Web/CSS/Guides/Transitions/Using
- lightningcss(cssparser 基座;编译时间 issue #357)
  https://github.com/parcel-bundler/lightningcss ;
  https://github.com/parcel-bundler/lightningcss/issues/357
