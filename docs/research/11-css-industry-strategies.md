# 11 号调研:桌面/跨端框架的 CSS 支持策略光谱 + Rust 可复用基建

> 生成:2026-07-17,全部论断经联网核实(来源清单见文末 §9)。
> 背景:sv 的 `.sv` 单文件组件已支持 scoped `<style>` 块,但样式语言是自造迷你语法
> (封闭属性集 `键:值`,类编译成组件内样式函数,零运行时选择器,见
> `crates/sv-compiler/src/style.rs` 与 `crates/sv-ui/src/lib.rs` 的 10 字段 `Style`)。
> Svelte 开发者在浏览器写的是**真 CSS**。本报告回答:桌面端如何**无缝支持 CSS 心智**、
> 最小化迁移负担——业界九家怎么做的、Rust 生态能省什么、sv 该走哪档。

## 0. TL;DR

1. **业界光谱五档**:零 CSS(Flutter/Compose/QML)→ CSS 风格属性值(Slint)→
   CSS 属性名对象子集(React Native)→ **真 CSS 语法子集引擎(NativeScript、Lynx)**
   → 浏览器级 CSS(Blitz/Dioxus Native 用 stylo;Tauri/Electron 整个浏览器引擎)。
2. **口碑分界线不在"100% 浏览器级"**,而在"是否真 CSS 语法 + 选择器 + 状态伪类 +
   变量 + transition/keyframes"。Lynx 砍了伪元素、把普通属性继承做成 opt-in、不支持
   `inherit` 关键字,仍被社区盛赞"true CSS support";RN 用了 CSS 属性名却没有选择器/
   级联/单位,催生出整个 NativeWind 生态和成篇的迁移吐槽文。**Lynx 档就是"感觉不到
   迁移"档**。
3. **Svelte 的 scoped-by-default 把需求面进一步压小**:Svelte 组件样式的主体是"扁平
   类规则 + 元素选择器 + 状态伪类 + 自定义属性 + @media",跨组件级联本来就被 Svelte
   自己禁掉了。sv 要复刻的不是浏览器 CSS,而是"**一个 Svelte 组件 `<style>` 块里
   实际会出现的 CSS**"——这个面小得多。
4. **Rust 基建现成度高**:lightningcss(MPL-2.0)给编译期"按规范文法解析→每属性
   类型化值",直接替换手写解析器;taffy(MIT,已在 M1 路线图)给 flex/grid/block 的
   规范级布局;simplecss / selectors 给两档运行时选择器匹配;stylo 2024-04 起独立上
   crates.io(0.6.x,Blitz 实证可用)但重且与"编译期样式表"架构相斥,列为 C3 可选项。
5. **推荐三阶段甜点线**:C1「CSS 语法真化」(真 CSS 语法 + 盒模型全套 + 状态伪类,
   仍零运行时选择器,约 3–5 人周)→ **C2「CSS 行为完备」= Lynx 档 = 迁移无感线**
   (taffy 属性面 + CSS 变量继承 + transition/@keyframes + @media + 组件内后代组合子)
   → C3「引擎级」(stylo,仅当未来要渲染任意 HTML,默认不做)。09 §4 的自造样式
   语言应升级为"**真 CSS 语法的封闭子集**"——与"模板语法 100% Svelte、语义按桌面
   裁剪"是同一哲学:**语法保真,面积裁剪**。

---

## 1. 调研问题与方法

三个问题:① 各桌面/跨端框架把 CSS 支持到哪层、砍了什么、开发者迁移反馈如何;
② Rust 生态有哪些可复用的 CSS 基建、各自能省什么;③ sv 的"CSS 心智覆盖率 vs
实现成本"甜点在哪一档。方法:官方文档为准 + 社区反馈交叉验证(GitHub issues、
迁移吐槽/好评文章、KOL 评测),Rust crate 以 crates.io/仓库 README 实证。

---

## 2. 九家框架逐一核实:CSS 支持策略光谱

### 2.0 光谱总图

| 档位 | 框架 | 样式语言 | 选择器 | 级联 | 继承 | 动画走 CSS? |
|---|---|---|---|---|---|---|
| **零 CSS** | Flutter | Widget 构造参数/ThemeData | 无 | 无 | Theme 显式下发 | 否(代码) |
| | Compose | Modifier 链(+2026 新 Styles API) | 无 | 无 | CompositionLocal | 否(代码) |
| | QML | 属性绑定 + Style 单例 | 无 | 无 | 无 | 否(Animation 对象) |
| **CSS 风格属性** | Slint | DSL 属性(px、CSS 色名、brush) | 无 | 无 | 无(显式) | 否(animate 块) |
| **CSS 属性名对象** | React Native | JS 对象(StyleSheet)+ Yoga | 无 | 无 | 仅 Text 嵌套 | 否(Animated/Reanimated) |
| **真 CSS 子集** | NativeScript | .css 文件 | 类型/类/id/后代/属性 + `:highlighted` | 有 | **无** | 部分 |
| | **Lynx** | .css 文件(编译期处理) | 类/id/类型/通配 + 4 种组合子 + `:active :not :root` | 有 | 变量默认继承;普通属性 opt-in | **是**(@keyframes/transition) |
| **浏览器级** | Blitz/Dioxus Native | stylo(Firefox 同款引擎) | 浏览器级 | 有 | 有 | 部分(施工中) |
| | Tauri / Electron | 系统 WebView / 捆绑 Chromium | 100% | 100% | 100% | 100% |

参照系(不在任务清单但有信息量):**Avalonia**——XAML 桌面框架,却原样吸收了 CSS
概念:选择器语法(`Button.primary:pointerover`)、样式类、伪类(:pointerover/:focus/
:disabled/:pressed/:checked)、特异性规则(id > 伪类/属性 > 类 > 类型,同级后声明胜)。
说明**桌面框架在样式复用压力下会自发收敛到 CSS 的概念体系**,哪怕语法外壳不是 CSS。

### 2.1 Flutter:彻底无 CSS(零档)

- **支持哪层**:无任何 CSS。样式即 Widget 构造参数(`TextStyle`、`BoxDecoration`、
  `EdgeInsets`),复用靠 `ThemeData` + 组合;官方专门维护一页
  "[Flutter for web developers](https://docs.flutter.dev/get-started/flutter-for/web-devs)"
  教你把每条 CSS 心智翻译成 Widget 嵌套。
- **砍了什么**:全部——语法、选择器、级联、继承、单位体系。
- **迁移反馈**:web 出身开发者的抱怨集中且长期:样式与结构耦合导致**嵌套过深、
  过于啰嗦**("给按钮加个 padding 要多包一层 Padding widget");多样式文本比 CSS
  笨重;无法"按 class 批量改样式"。官方 issue
  [#52454](https://github.com/flutter/flutter/issues/52454)(请求 CSS 单位/样式)
  和 Niku 这类"把样式做成链式 API 模仿 CSS 复用"的社区库是需求存在的实证。
  结论:零档的代价是持续的 DX 摩擦,靠强主题系统与热重载补偿。

### 2.2 Jetpack Compose:Modifier 显式链(零档,但正在长出"样式表")

- **支持哪层**:无 CSS。`Modifier` 链显式声明,顺序即语义;主题走 `MaterialTheme` +
  CompositionLocal。设计哲学是"显式、可预测、类型安全",官方文档明说这是对 CSS
  隐式性的反动。
- **值得注意的新动向(2026-06)**:官方推出 **Styles API**,把"视觉(颜色/边框/
  圆角/内边距)"从 Modifier 中分离成可复用、可主题感知的 Style 对象,Modifier 退回
  行为域。**连最反 CSS 的阵营也在重新发明"样式表"**(无选择器版)——样式复用是
  刚需,区别只在要不要选择器和级联。
- **迁移反馈**:Android 原生人群无 CSS 心智包袱,抱怨少;但这恰说明 Compose 的
  目标用户不是 web 迁移者,对 sv 参考价值在"样式/行为分离"这一步。

### 2.3 QML:属性绑定(零档;CSS 需求由第三方证明)

- **支持哪层**:纯 QML 无 CSS。社区标准做法是 Style 单例对象集中放样式常量,
  属性绑定分发。注意区分:**Qt Widgets 的 QSS(Qt Style Sheets)是 CSS 风格语法**
  (选择器/伪状态子集),但 QML/Qt Quick 刻意没有继承这条路,Qt Quick Controls
  的"样式"是整套委托(delegate)替换。
- **需求实证**:Ableton 开源了 [aqt-stylesheets](https://github.com/Ableton/aqt-stylesheets)
  ——专门给 QML 外挂 CSS 样式表的库。工业用户(Ableton Live 界面团队)愿意自建
  CSS 层,说明**大型桌面产品的设计/开发协作里,"样式集中在样式表"的工作流有真实
  价值**,DSL 属性绑定并不能完全替代。

### 2.4 Slint:CSS 风格的属性值,无样式表(第一档)

- **支持哪层**:`.slint` DSL 的属性系统**借用 CSS 的值词汇**:逻辑像素 `px` 单位、
  CSS 颜色名(在 color/brush 类型语境内)、brush/渐变、`padding` 系属性、对齐语义
  官方文档明说"匹配 CSS flexbox"。有 property 响应式绑定和 states(状态切换)。
- **砍了什么**:没有样式表文件、没有选择器、没有级联/继承;"换皮"走内置 widget
  style(fluent/material/cupertino,跟随系统深浅色),不是用户级 CSS。
- **反馈**:Slint 用户以嵌入式/桌面 Rust/C++ 人群为主,对 CSS 缺席抱怨不多;但
  Slint 的选择(值词汇学 CSS、结构不学)印证了一个低成本策略:**先把"单位、颜色、
  属性名"这些微观心智对齐,就能吃掉相当一部分熟悉感**。sv 现状(`padding:24` 无
  单位、`#ff3e00` 颜色)已经在这一档的门口。

### 2.5 React Native:CSS 属性名对象 + Yoga(第二档;反面教材主角)

- **支持哪层**:`StyleSheet.create` 的 JS 对象,**属性名是 camelCase 的 CSS 子集**;
  布局是 Meta 的 C++ flexbox 实现 **Yoga**。官方文档自述"名字和值和 web CSS 匹配,
  但这不是 CSS"。
- **砍了什么**(官方与社区文档双重确认):**无选择器、无级联**(样式只来自
  style prop 与数组合成)、**无继承**(唯一例外:嵌套 `<Text>` 继承文本样式)、
  无 @ 规则/伪类/伪元素、无媒体查询、长期无 CSS 单位(数字即密度无关像素)。
  动画不走 CSS(Animated/Reanimated 代码驱动)。
- **但方向在回摆**:RN 0.77(2025-01)一口气加入 `display: contents`、`boxSizing`、
  `mixBlendMode`、`outline*`,官方博客明说是**向 web 标准对齐**;新架构持续吸收
  CSS 语义。框架自己在承认"离 CSS 太远是负债"。
- **迁移反馈(证据最丰富的一家)**:
  - 吐槽侧:成篇的"[Styles in React Native Aren't CSS — Stop Treating Them Like
    They Are](https://medium.com/@tharunbalaji110/from-web-to-native-styles-in-react-native-arent-css-stop-treating-them-like-they-are-fd5c71817fe1)"类文章;react-native-css-modules 的 FAQ 维护着一整页
    "与浏览器 CSS 的差异"清单。
  - 用脚投票侧:**NativeWind**(把 Tailwind 类名编译成 StyleSheet)成为 web 出身
    开发者的默认选择,"心智模型直接迁移"是其头号卖点,竞品(Uniwind、twrnc、
    Tamagui)层出不穷——一个框架的样式层催生出一个"翻译层"生态,本身就是
    **原生样式层心智失配**的最强证据。
- **对 sv 的教训**:"属性名像 CSS"不等于"CSS 心智"。缺选择器/状态伪类/级联的
  对象样式,web 开发者依然觉得"这不是 CSS"。第二档是个尴尬档,不如 either 退回
  Slint 档(诚实的 DSL)or 进到 Lynx 档(诚实的 CSS)。

### 2.6 NativeScript:真 CSS 文件子集(第三档)

- **支持哪层**:平台无 WebView,但样式就是 `.css` 文件:类型/类/id/后代/属性
  选择器、选择器组合、`calc()`、CSS 主题类(`.ns-dark`/`.ns-android` 等状态类做
  平台/深浅色适配)。
- **砍了什么**:伪类只有 `:highlighted`(按压态);**无 CSS 继承**;属性面限于
  文档列表(超出静默无效);无伪元素;媒体查询长期缺席(后由社区插件补)。
  GitHub 上多年 open 的 [#98(高级选择器)](https://github.com/NativeScript/NativeScript/issues/98)、
  [#50(伪类/状态选择器)](https://github.com/NativeScript/NativeScript/issues/50)、
  [#191(扩属性集)](https://github.com/NativeScript/NativeScript/issues/191)
  勾勒出用户不断顶到的天花板位置:**状态伪类和属性覆盖面是最先被要的两样**。
- **反馈**:web(尤其 Angular/Vue)人群评价普遍是"样式上手几乎零成本";没人写
  "NativeScript 的样式不是 CSS"式檄文。第三档已经越过了口碑分界线。

### 2.7 Lynx(字节跳动,2025-03 开源):真 CSS 引擎,本调研重点

> 注:任务背景里写"2024 开源",经核实开源时间是 **2025-03-05**(InfoQ 报道),
> 此前在字节内部(TikTok Studio/Shop 等)多年打磨。

- **定位**:双线程架构(主线程渲染 + 后台线程跑用户 JS)、自研 PrimJS 引擎、
  Rust 工具链(rspeedy 构建器),**"原生平台上的真 CSS"是它对 RN 的头号差异化**。
  样式写在 `.css` 文件,构建期由 Rust 工具链处理,运行时由 C++ 引擎按规则应用。
- **CSS 引擎范围(逐项核实,官方文档)**:
  - **选择器**([api/css/selectors](https://lynxjs.org/api/css/selectors.html)):
    类型、类、多类复合、id、通配 `*`;组合子:后代(空格)、子代 `>`、后续兄弟
    `~`、相邻兄弟 `+`;选择器列表 `,`;伪类:`:active`、`:not()`、`:root`。
    **明确没有:伪元素**("Lynx doesn't have pseudo-element support yet");
    `!important` 到 engineVersion 3.9+ 才可经插件配置启用。
  - **级联**:有——遵循级联规范,后声明覆盖先声明,`style` 属性覆盖样式表规则,
    specificity 按标准算。
  - **继承(最有信息量的裁剪)**:**普通 CSS 属性默认不继承**,需配置显式开启
    (性能考量);**CSS 自定义属性(变量)默认继承**、符合 web 标准;`inherit`
    关键字**不支持**。——即 Lynx 把"继承"拆成两半:主题所需的变量继承保住,
    昂贵的全属性继承默认关掉。
  - **动画**:`@keyframes` 全套 + `transition` 全套 + JS `animate()` API;
    动画可跑在主线程保流畅。
  - **视觉面**:背景/边框/box-shadow/text-shadow、线性与径向渐变、`clip-path`、
    `mask-image`;`border-image` 施工中;一批 `-x-` 前缀私有属性。
  - **布局**:flexbox、grid、linear(私有线性布局)、relative(私有相对布局)。
  - **生态兼容**:支持 Sass/PostCSS(嵌套语法过 postcss-nesting)、CSS Modules。
- **迁移反馈**:KOL Theo Browne 评为"我见过的对 RN 最强的挑战者",**点名"能用
  CSS"是核心理由之一**;技术媒体(InfoQ/The New Stack/Appwrite)对比文一致把
  "true CSS support"列为对 RN 的头号优势。未见"Lynx 的 CSS 不够用"类抱怨成气候
  (开源仅一年余,样本有限,列为观察项)。
- **对 sv 的意义**:Lynx 是"**非浏览器引擎上自研 CSS 子集**"的最新、最完整样板,
  其裁剪清单(砍伪元素、砍 `inherit`、普通属性继承 opt-in、`!important` 默认关)
  是一份经 TikTok 体量验证过的"CSS 哪些部分性价比低"的答案卷。

### 2.8 Blitz / Dioxus Native:直接用 Firefox 的 CSS 引擎(第四档)

- **支持哪层**:[Blitz](https://github.com/DioxusLabs/blitz) 是模块化 HTML/CSS
  渲染引擎:**stylo(Firefox/Servo 同款 CSS 引擎)做解析与样式解算,taffy 做盒级
  布局,parley 做文本布局**(渲染 vello/wgpu),上面架 Dioxus Native。相当于
  "webview 减去 JS 引擎,换成 Rust 直连"。CSS 能力即浏览器级(选择器/级联/继承/
  变量全有),现代布局(flex/grid)完整,刻意不做的是 JS 执行、浏览器级网络缓存/
  安全/进程隔离,以及部分 CSS2 时代遗留特性。
- **成熟度**:官方自评 **alpha**,"可实验、勿生产";目标 2025 底 beta、2026 内
  production-ready;dioxus-native 0.7.3(2026-01)仍在快速迭代。
- **对 sv 的意义**:证明了两件事——① stylo 独立出 Servo **真的能用**(见 §4.1);
  ② 走这条路等于把"样式解算"整个交给运行时引擎,和 sv"编译期样式表 + 零运行时
  选择器"的架构决策方向相反。它是 sv 的 C3 参照系,不是 C1/C2 的路。

### 2.9 Tauri / Electron:整个浏览器引擎(第五档,基线参照)

- Electron 捆绑 Chromium(每个应用背 80–120MB),CSS 100% 且跨平台一致;
  Tauri 用系统 WebView(Windows=WebView2/Chromium、macOS=WKWebView/WebKit、
  Linux=WebKitGTK,经 wry),体积极小但 CSS 行为随三家 WebView 有差异(WebKitGTK
  的滞后是社区长期痛点,甚至有"求换 Chromium"的讨论)。
- **对 sv 的意义**:这是"CSS 心智零迁移"的上界,同时也标出了代价:要么背引擎
  体积,要么背跨平台不一致。sv 的存在前提就是不走这档,列为对照即可。

---

## 3. 裁剪规律:各家"砍什么"对照

把第三档以上(真 CSS 语法)玩家的裁剪清单并排,规律非常清晰:

| CSS 机制 | NativeScript | Lynx | Blitz | 规律 |
|---|---|---|---|---|
| 类/类型/id 选择器 | ✅ | ✅ | ✅ | **没人砍**——底线 |
| 后代/子代组合子 | ✅(后代) | ✅(全套) | ✅ | 基本没人砍 |
| 状态伪类(:hover/:active…) | ⚠️ 仅 :highlighted(用户长期催) | ✅ :active(+:not/:root) | ✅ | 砍了会被催——刚需 |
| 伪元素(::before…) | ❌ | ❌ | ✅ | **自研引擎全砍**,无人抱怨 |
| 级联/特异性 | ✅ | ✅ | ✅ | 真 CSS 档没人砍 |
| 普通属性继承 | ❌ | ⚠️ opt-in | ✅ | **性能顾虑集中点**,自研引擎倾向砍/关 |
| CSS 变量 + 默认继承 | ⚠️(后补) | ✅ | ✅ | 主题刚需,Lynx 特意保住 |
| `inherit` 关键字 | ❌ | ❌ | ✅ | 自研引擎砍 |
| `!important` | — | ⚠️ 3.9+ opt-in | ✅ | 砍了反而被视为美德 |
| @keyframes / transition | 部分 | ✅ | 施工中 | **Lynx 的口碑关键件** |
| @media | ⚠️ 插件 | ✅ | ✅ | 桌面对应"窗口尺寸查询",要 |
| 伪元素外的高级选择器(:nth-child 等) | ❌ | ❌(仅 :not) | ✅ | 自研引擎全砍,无人抱怨 |

**提炼**:自研 CSS 引擎的共识裁剪 = 伪元素、`inherit` 关键字、`!important`(默认)、
高级结构伪类;共识保留 = 类/类型选择器 + 常用组合子 + 状态伪类 + 级联特异性 +
CSS 变量(含继承)+ keyframes/transition。普通属性继承是唯一的分歧点(性能 vs 心智),
Lynx 的 opt-in 是工程妥协——**sv 的场景树浅、样式解算在编译期已定序,可以反着做:
默认开继承子集(color/font-*),把 Lynx 不敢给的还给开发者**。

---

## 4. Rust 可复用基建盘点

### 4.1 stylo:浏览器级 CSS 引擎,已独立可用,但重

- [servo/stylo](https://github.com/servo/stylo):驱动 Firefox 与 Servo 的 CSS 引擎,
  **2024-04 起独立发布到 crates.io**(0.0.1 → 现 0.6.x,随仓库 rebase 发版),
  维护者含 Nico Burns(Blitz/taffy 作者)。MPL-2.0。独立使用**不需要 bindgen/clang**
  (那是 Gecko 集成模式的历史包袱);配套 `stylo_taffy` crate 做 stylo→taffy 样式
  类型转换(即 blitz-dom 的实现细节,官方注明"可独立用作集成参考")。
- **能省什么**:解析、级联、特异性、继承、变量、计算值——整个"样式解算"层,
  浏览器级正确性免费。
- **代价**(为何不进 C1/C2):
  ① 接入面大——要给自己的树实现 selector-matching 的 element trait 一族,API 服务
  于 Servo/Firefox 的节奏("版本随 rebase 发",无稳定性承诺,文档自认滞后);
  ② 编译重——stylo 家族十几个 crate,拖累 sv 目前"分钟级构建"的原型体验;
  ③ **架构相斥**——stylo 是运行时引擎,吃掉 sv"样式编译成组件内定点更新代码、
  零运行时选择器"的差异化。结论:C3 选项,触发条件是"要渲染任意 HTML/富文本域"。

### 4.2 lightningcss:编译期解析器的最佳候选

- [parcel-bundler/lightningcss](https://github.com/parcel-bundler/lightningcss):
  Rust 写的 CSS 解析/转换/压缩器(Parcel 御用),底层用 Mozilla 的 cssparser +
  selectors。**按 CSS 规范文法解析所有值,每个属性给类型化的值结构**(不是字符串!),
  支持浏览器目标降级、visitor API 自定义转换。MPL-2.0。
- **能省什么(对 sv 是量身定做)**:sv 的样式解算在**编译期**(build.rs 里),
  lightningcss 正好是"库形态的编译期 CSS 前端":
  ① 免写整个 CSS 语法解析器(现 `style.rs` 的手写解析全退役);
  ② 类型化属性值(`Length`/`Color`/`FlexDirection`…)直接映射 `sv_ui::Style` 字段,
  单位换算、颜色函数(rgb/hsl/oklch)、简写展开(`margin: 8px 16px`)全部免费;
  ③ 选择器 AST 现成(class/type/伪类解析好给你),sv 只做"封闭子集校验 + 编译期
  定序",超出子集报编译错(带 .sv 行列,优于 Lynx 的静默忽略);
  ④ 未来 @media/@keyframes/变量的语法解析也在里面。
- **代价**:MPL-2.0(文件级 copyleft,作为依赖使用无碍,产物代码是我们自己生成的,
  不受传染;口径需在 LICENSE 说明中写清);版本长期 1.0.0-alpha.x(但被 Parcel 等
  大规模生产使用);编译期依赖不进运行时,体积无虞。

### 4.3 cssparser + selectors:更底层的两块砖

- [cssparser](https://crates.io/crates/cssparser)(MPL-2.0):Mozilla 的 CSS 词法/
  语法框架(tokenizer + 解析辅助),**不含属性语义**——值解析要自己写。若嫌
  lightningcss 太大,这是手搓属性层的底座;但那等于重写 lightningcss 已写好的部分,
  仅当"封闭属性集小到不值得引 lightningcss"才划算(sv 属性集会长到几十个,不成立)。
- [selectors](https://github.com/servo/stylo/tree/main/selectors)(MPL-2.0,stylo
  仓库内独立发布):浏览器级选择器解析 + **对泛型元素树的匹配**。C2 若要运行时
  后代组合子匹配,这是"重解"(实现其 Element trait);先评 simplecss 够不够。

### 4.4 simplecss:白菜价的选择器匹配

- [linebender/simplecss](https://github.com/linebender/simplecss)(Linebender 维护,
  resvg 在用):CSS 2.1 子集解析器 + 选择器匹配 + **按 specificity 排序**。
  限制:@ 规则跳过、**属性值不解析(拿到字符串)**、大小写敏感。
- **能省什么**:如果 C2 只需要"组件内后代/子代组合子 + 少量伪类"的运行时匹配,
  simplecss 几百行的心智负担就够;值解析反正走 lightningcss(编译期),两者职责
  不重叠。缺点是 CSS 2.1 文法较老(如 `:not()` 不在)——届时也可以只抄它的匹配
  算法自写百行(sv 场景树 API 固定,匹配器很小)。

### 4.5 taffy:布局属性面的规范级实现(已在路线图)

- [DioxusLabs/taffy](https://github.com/DioxusLabs/taffy)(MIT):**CSS Block +
  Flexbox + Grid 布局算法**的高性能实现,自述"忠实实现规范,web 文档可直接套用";
  Blitz/Bevy/Zed(fork)等共同压测;高层 `TaffyTree` + 测量函数闭包正好接 sv 的
  文本测量。2026-06 仍活跃。
- **能省什么**:C2 的整个布局属性面——`display:flex/grid`、`justify-content`/
  `align-items`/`gap`/`flex-wrap`/`flex-grow`/`grid-template-*`/`position`/`inset`/
  `min/max-*`/`aspect-ratio`/百分比单位……sv 只做"lightningcss 类型化值 → taffy
  Style 字段"映射(参考 `stylo_taffy` 的转换代码,以及 taffy
  [#440](https://github.com/DioxusLabs/taffy/issues/440) 里社区对"CSS 文本 → taffy
  Style"的讨论)。**taffy 属性名与 CSS 一一对应,是"CSS 心智"最大单件来源**,
  且 M1 已立项——C2 的地基已经在路线图里了。

### 4.6 组合矩阵:每档用什么

| 阶段 | 编译期解析 | 布局 | 运行时匹配 | 动画 | 引入成本 |
|---|---|---|---|---|---|
| C1 语法真化 | **lightningcss** | 现有(direction/gap) | 无(全编译期定序) | — | 一个编译期依赖 |
| C2 行为完备 | lightningcss(+@media/@keyframes/var) | **taffy**(M1 已定) | simplecss 级小匹配器(仅组件内组合子) | 现有 anim 模块扩通道 | taffy 已立项;匹配器自写百行级 |
| C3 引擎级 | — | taffy(stylo_taffy) | **stylo + selectors** | stylo transitions | 重;架构转向 |

---

## 5. Svelte 开发者实际用到的 CSS 面

关键洞察:**Svelte 的 scoped-by-default 已经替 sv 砍掉了 CSS 最贵的部分**。
Svelte 组件的 `<style>` 默认只作用于本组件(编译器加 hash 类实现),跨组件级联
被 Svelte 自己禁掉(逃逸口 `:global` 在 sv 已裁决不做,见 SVELTE-SUPPORT F 表)。
因此一个 Svelte 组件样式块里高频出现的是:

1. **扁平类规则 + 元素类型规则**(`.card {…}` `button {…}`)——绝对主体;
2. **状态伪类**:`:hover` `:active` `:focus` `:disabled` `:checked`——桌面交互刚需,
   Svelte 心智是"hover 样式写 CSS,不写 JS";
3. **浅后代/子代组合子**(`.list .item`、`ul > li`)——组件内部结构选择;
4. **盒模型 + flex/grid + 单位**(px/rem/%/em)+ 颜色函数;
5. **CSS 自定义属性**:组件主题化(Svelte 还有 `--style-props` 组件传参语法);
6. **transition/animation 属性 + @keyframes**(与 Svelte `transition:` 指令并存);
7. **@media**(浏览器语境是视口,桌面对应窗口尺寸/深浅色查询);
8. **继承**:`color`/`font-*` 从容器流到文本——写 `.card { color: #888 }` 时
   默认里面的字都变灰,这是 CSS 心智里最隐形也最基础的一条。

**低频/几乎不用**:伪元素(桌面组件树里可用真元素表达)、结构伪类(`:nth-child`
——有 `{#each}` 的 index 在,模板层解决)、`!important`(scoped 下无对手,本来就
不需要)、`inherit/initial/unset` 关键字、层叠层 `@layer`、`@supports`。
——这份低频清单与 §3 的"业界共识裁剪清单"**几乎完全重合**,交叉印证:
砍这些,Svelte 开发者感觉不到。

---

## 6. 推荐路线:三阶段甜点线

### 总判断

**C2 档(≈ Lynx 档,再加默认继承子集)就是"Svelte 开发者感觉不到迁移"的档位。**
论证链:① Lynx 以低于浏览器级的 CSS 子集赢得"true CSS support"口碑(§2.7),
分界线是"真语法 + 选择器 + 状态伪类 + 变量 + 动画"而非完备性(§3);② RN 反例
证明"半 CSS"(属性名像、行为不像)不过线(§2.5);③ Svelte scoped 前提把需求面
压缩到 §5 的八条,C2 全覆盖,低频清单与业界共识裁剪重合。sv 比 Lynx 还多两张牌:
**编译期报错**(超子集当场红,不是静默失效——迁移期最重要的安全网)与**默认
继承子集**(场景树浅 + 编译期定序,付得起 Lynx 付不起的成本)。

### C1「CSS 语法真化」——把自造语法换成真 CSS,零运行时原则不动

放进 M1 尾或 M2,估 **3–5 人周**:

1. **解析器换 lightningcss**(build.rs 编译期):`<style>` 块与 `style=""` 内联全走
   规范文法。现 `style.rs` 手写解析退役。
2. **属性集扩到盒模型全套**(仍封闭,超出报错):margin/padding 四边与简写、
   border(width/style/color/radius 分角)、min/max-width/height、background(纯色
   先行)、font-family/weight/style、line-height、text-align、opacity、单位
   px(=现逻辑像素)/%(转 taffy 前先支持宽高)/rem(锚根字号)、颜色函数
   rgb()/hsl()/色名/#hex。**写法即 CSS:`padding: 24px` 而非 `padding:24`**
   (旧语法一个大版本内兼容告警)。
3. **选择器进第一批**:类、元素类型、多类复合 + **状态伪类** `:hover :active
   :focus :disabled :checked`。实现:每类编译成"基础样式 + 状态变体样式"的静态
   表,状态翻转 = 现有 `bind_style_patch`/StyleClass 机制打补丁——**仍然零运行时
   选择器匹配**,特异性与声明顺序在编译期排好。
4. **继承子集默认开**:color/font-size/font-family/font-weight/line-height/
   text-align 沿场景树解析(样式求值时向上取父值,场景树已有 parent 链)。
5. **级联语义**:同组件内按 (specificity, 声明顺序) 定序;层级优先级保持现有链:
   类规则 < `style=""` < `style:` 指令。
6. **诊断**:未支持的属性/选择器/单位 → 编译错误带 .sv 行列 + "在路线图/永不支持"
   的分类说明(学 Svelte 编译器的教学式报错,别学 NativeScript 的静默)。

修订 09 §4:自造"极简键:值语言"升级为"**真 CSS 语法的封闭子集**";`@tokens/@theme`
自造语法撤销,改用 CSS 自定义属性(C2);"永远 scoped、无 `:global`"维持;
"一切动态样式走 `style:`"纪律放宽——**状态伪类是声明式动态样式,归 `<style>`**。

### C2「CSS 行为完备」——迁移无感线

依赖 taffy(M1 已立项)与帧调度/anim(M2),增量估 **6–10 人周**:

1. **布局属性面对齐 CSS**:taffy 接入后,`display:flex/grid`、justify/align 全家、
   gap、flex-*、grid-template-*、position/inset、aspect-ratio、百分比与自动尺寸,
   映射层参考 stylo_taffy。这一步完成后,"web 上怎么写布局,这里就怎么写"成立。
2. **CSS 自定义属性 + var()**:默认继承(Lynx 同款裁决),主题 = 根节点变量集;
   与编译期样式表共存的方式:变量引用编译成"运行时查询节点变量链"的求值闭包,
   非变量部分仍编译期常量折叠(需 spike,见 §8)。
3. **transition 属性 + @keyframes**:接 anim 模块(现有 opacity 通道扩到
   transform/颜色/尺寸);与 `transition:fade` 指令并存,对齐 Svelte 双轨心智。
4. **@media 桌面语义**:窗口宽高、prefers-color-scheme(接 M2 深色模式)。
5. **组件内后代/子代组合子**:scoped 前提下大部分组合关系编译期可静态判定
   (模板树已知);仅动态边界({#if}/{#each} 跨层)退化为挂载时一次匹配
   (simplecss 级自写匹配器,场景树 API 固定,估百行级)。
6. **明确永不支持清单**(文档化,对齐业界共识):伪元素、`:global`、结构伪类
   (:nth-child 系,模板层有 index)、`!important`、`inherit` 关键字、@layer、
   @supports。每条给出 sv 侧替代写法。

### C3「引擎级」——默认不做的选项

触发条件:未来要渲染任意 HTML/Markdown 富文本域,或用户实证需要浏览器级选择器。
路径:stylo + selectors 换装(Blitz 已验证组合),彼时"编译期样式表"退为快速路径。
在此之前不投入。

### 成本/覆盖对照

| 阶段 | CSS 心智覆盖 | 人力 | 风险 |
|---|---|---|---|
| 现状 | ~10%(9 个属性键,自造语法) | — | 每个新用户都要学一门新样式语言 |
| C1 | ~55%(语法/盒模型/状态伪类/继承子集) | 3–5 人周 | lightningcss 值类型映射覆盖率需 spike |
| **C2** | **~90%(Svelte 组件实用面全覆盖)** | +6–10 人周(taffy 本已立项) | 变量运行时继承 vs 编译期折叠的共存设计 |
| C3 | ~99%(浏览器级) | 人月级 + 架构转向 | 吃掉零运行时差异化;编译时间 |

---

## 7. 对现有设计文档的修订建议

1. **09 §4(样式语言设计)**:核心修订——样式语言从"自造极简语法"改为"真 CSS
   语法的封闭子集",与模板层"语法 100% Svelte、语义按桌面裁剪"统一哲学。
   `style.rs` 的手写解析器在 C1 退役,错误定位机制(offset → .sv 行列)保留复用。
2. **SVELTE-SUPPORT F 表**:`<style>` 块条目的"极简类+属性集"描述在 C1 后过时;
   `:global` 不做的裁决维持;新增"CSS 支持矩阵"附表(建议以 §3 对照表为模板,
   逐特性标 ✅/⏳/❌,学 Lynx 给每个 CSS 属性建 API 文档页的做法)。
3. **DESIGN.md 风险清单**:新增一条——MPL-2.0 依赖(lightningcss/cssparser)与
   MIT/Apache 主许可的共存口径(依赖使用无传染,产物代码为编译器生成,需在
   NOTICE 写明;法务级确认列 M2 前置)。
4. **`sv_ui::Style` 结构**:10 字段在 C1 需扩为分组结构(box/font/visual/layout),
   并为继承子集加"unset 哨兵"(区分"未设置需继承"与"显式默认值")——建议
   `Option<T>` 语义从"未设置"细化为三态或引入 `Inherit` 变体,随 C1 设计定案。

---

## 8. 未决问题(需 spike 或裁决)

1. **lightningcss → sv Style 映射覆盖率**:哪些属性 lightningcss 给类型化值、哪些
   落在 `unparsed` 兜底,需要 1–2 天 spike 出清单(决定 C1 属性集边界)。
2. **CSS 变量与编译期样式表的共存**:变量解析点在编译期(常量折叠,主题切换即
   重编译——热重载数据面可救)还是运行时(节点变量链查找,吃掉部分零运行时
   优势)?建议 spike 两版性能/复杂度对比,倾向"运行时查找仅限 var() 引用处"。
3. **状态伪类与 StyleClass 索引的会师**:状态位掩码(hover/active/focus/checked
   4 bit)× 类索引的变体表,还是每状态独立 patch 闭包?影响 C1 codegen 形态。
4. **继承的实现位点**:样式解算时向上取值(读时) vs 场景树变更时向下推(写时)?
   浅树下读时简单正确,深树/高频动画下需基准。
5. **单位体系定案**:px=逻辑像素(Slint/Lynx 同款)已隐式成立;rem 锚点(根字号
   可否运行时改——影响无级缩放特性)与 % 在 taffy 前的临时语义需定。
6. **组合子在 keyed each 动态行上的重匹配策略**(行插入/移动后,`.list .item:hover`
   类规则的失效面)——挂 C2,与 ADR-7 reconcile 一起设计。
7. **Lynx 长期口碑跟踪**:开源仅一年余,"CSS 子集哪里不够用"的社区反馈尚薄,
   建议每半年复查其 issue 趋势,校准 sv 的裁剪清单。

---

## 9. 来源清单

**Lynx**:[官方选择器 API](https://lynxjs.org/api/css/selectors.html) ·
[Styling 指南](https://lynxjs.org/guide/ui/styling.html) ·
[外观/视觉效果](https://lynxjs.org/guide/styling/appearance.html) ·
[动画(@keyframes/transition/animate)](https://lynxjs.org/guide/styling/animation) ·
[主题(变量继承/enableCSSInheritance)](https://lynxjs.org/guide/styling/custom-theming) ·
[InfoQ 开源报道(2025-03)](https://www.infoq.com/news/2025/03/tiktok-lynx-cross-platform-apps/) ·
[The New Stack(Theo Browne 评价)](https://thenewstack.io/cross-platform-ui-framework-lynx-competes-with-react-native/) ·
[Appwrite 对比](https://appwrite.io/blog/post/bytedance-lynx-vs-react-native)

**React Native**:[官方 Style 文档](https://reactnative.dev/docs/style) ·
[RN 0.77 博客(display:contents/boxSizing/mixBlendMode/outline)](https://reactnative.dev/blog/2025/01/21/version-0.77) ·
[react-native-css-modules 差异 FAQ](https://github.com/kristerkari/react-native-css-modules/blob/master/docs/faq.md) ·
[React Native for Web:Styling(无级联自述)](https://necolas.github.io/react-native-web/docs/styling/) ·
["Styles in RN Aren't CSS" 迁移吐槽](https://medium.com/@tharunbalaji110/from-web-to-native-styles-in-react-native-arent-css-stop-treating-them-like-they-are-fd5c71817fe1) ·
[NativeWind](https://www.nativewind.dev/) ·
[RN 样式生态对比 2026(NativeWind/Tamagui/twrnc)](https://www.pkgpulse.com/guides/nativewind-vs-tamagui-vs-twrnc-react-native-styling-2026)

**Flutter**:[Flutter for web developers](https://docs.flutter.dev/get-started/flutter-for/web-devs) ·
[issue #52454(求 CSS 单位/样式)](https://github.com/flutter/flutter/issues/52454) ·
[社区样式复用探索(Niku 等)](https://dev.to/saltyaom/effortless-styling-in-flutter-3d33)

**Compose**:[Styles in Compose(官方)](https://developer.android.com/develop/ui/compose/styles) ·
[Styles vs Modifiers(官方)](https://developer.android.com/develop/ui/compose/styles/styles-vs-modifiers) ·
[Modifier 文档](https://developer.android.com/develop/ui/compose/modifiers)

**QML/Qt**:[Qml Styling(Qt Wiki,Style 单例模式)](https://wiki.qt.io/Qml_Styling) ·
[Ableton aqt-stylesheets(QML 外挂 CSS)](https://github.com/Ableton/aqt-stylesheets)

**Slint**:[Properties](https://docs.slint.dev/latest/docs/slint/guide/language/coding/properties/) ·
[Positioning & Layouts(px 单位、CSS flexbox 对齐)](https://docs.slint.dev/latest/docs/slint/guide/language/coding/positioning-and-layouts/) ·
[Widget Styles](https://docs.slint.dev/latest/docs/slint/reference/std-widgets/style/)

**NativeScript**:[Styling 文档(选择器子集/无继承/calc())](https://docs.nativescript.org/guide/styling) ·
[issue #98 高级选择器](https://github.com/NativeScript/NativeScript/issues/98) ·
[issue #50 伪类](https://github.com/NativeScript/NativeScript/issues/50) ·
[issue #191 扩属性集](https://github.com/NativeScript/NativeScript/issues/191)

**Blitz/Dioxus**:[DioxusLabs/blitz(stylo+taffy+parley)](https://github.com/DioxusLabs/blitz) ·
[blitz.is/about(alpha、beta 2025 底、production 2026)](https://blitz.is/about) ·
[dioxus-native crate](https://lib.rs/crates/dioxus-native) ·
[Webengines Hackfest 2024 幻灯](https://webengineshackfest.org/2024/slides/blitz_a_truly_modular_hackable_web_renderer_by_nico_burns.pdf)

**Tauri/Electron**:[Tauri Webview Versions](https://v2.tauri.app/reference/webview-versions/) ·
[tauri-apps/wry](https://github.com/tauri-apps/wry) ·
[WebKitGTK 不稳定讨论](https://github.com/orgs/tauri-apps/discussions/8524)

**Avalonia(参照系)**:[Style selectors](https://docs.avaloniaui.net/docs/styling/style-selectors) ·
[Pseudoclasses](https://docs.avaloniaui.net/docs/reference/styles/pseudo-classes)

**Rust 基建**:[servo/stylo(README:独立 crates、MPL、随 rebase 发版)](https://github.com/servo/stylo) ·
[stylo crate(0.0.1 于 2024-04-29)](https://crates.io/crates/stylo) ·
[stylo_taffy](https://crates.io/crates/stylo_taffy) ·
[lightningcss(cssparser+selectors 底座、类型化属性值、MPL-2.0)](https://github.com/parcel-bundler/lightningcss) ·
[lightningcss 文档](https://lightningcss.dev/docs.html) ·
[linebender/simplecss(CSS 2.1、specificity 排序、值不解析)](https://github.com/linebender/simplecss) ·
[DioxusLabs/taffy(Block/Flexbox/Grid,MIT)](https://github.com/DioxusLabs/taffy) ·
[taffy #440(CSS→Style 解析讨论)](https://github.com/DioxusLabs/taffy/issues/440) ·
[cssparser](https://crates.io/crates/selectors)
