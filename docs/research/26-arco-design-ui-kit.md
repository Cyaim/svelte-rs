# 26 号调研:以 arco.design 为视觉标准的 UI 组件库(sv-arco)可行性 + 分期计划

> 生成:2026-07-18。方法:arco 开源仓库逐文件核实(GitHub API 查许可证/组件目录、
> jsdelivr 拉取 `global.less` 原文、`palette.js`/`palette-dark.js` 算法源码全文抓取)
> + 本仓库源码逐行对照(sv-ui `Style`、sv-shell `Painter`、CSS-SUPPORT 矩阵、
> DESIGN.md R1–R5)。人周为单人粗估并标注置信度;查不到的明说(见 §7 未核清单)。
>
> 问题:**把字节跳动 Arco Design 的视觉体系(token + 组件规范)落在本仓库之上,
> 做一个 `sv-arco` 组件库,可行吗?什么时候能开工?卡点在哪?**

## 0. TL;DR 判决

**分档:条件可行(B 档)——token 层今天就能开工,组件分四波跟着 R1–R3 的能力
线走;"完整 arco 视觉"的达成点在 R3 弹层之后,且需要在路线图之外补三个渲染动词。**

1. **法务无硬伤**:`arco-design/arco-design` 与 `arco-design/color` 均为 **MIT**
   (GitHub License API 实查),token 值、色板算法、组件规范均可转译使用;
   合规动作 = 保留 LICENSE 副本 + NOTICE 署名(§4.3)。
2. **token 体系完全可编译期化**:arco 的全局 token 是纯静态 less 变量
   (圆角 2/4/8、字号 12–56 九档、间距 2–120、阴影三层九向、10 档色板),
   色板算法 60 行 HSV 数学(源码已全文核实,§1.2)——**正中本仓库
   "编译期样式表 + `:root` 变量"的架构甜点**,零运行时成本。
3. **最早可开工组件集**(R1 档 A 现状 + TextInput 落地后):Button / Tag /
   Badge / Divider / Alert / Typography 子集 / Checkbox / Radio / Switch /
   Input / Link / Space——约 12 件,现有 5 个渲染动词 + Style 13 字段即可
   渲染出"像素上认得出是 arco"的形态(无阴影无图标的降级版)。
4. **三大卡点**(按杀伤力):**图标管线**(arco 视觉一半是图标,本仓库无任意
   路径填充动词,需新增 `fill_path` + SVG 编译期转译,路线图上没有这一项);
   **box-shadow**(弹层/Card 的质感来源,CSS-SUPPORT 标 ⏳ 等 vello,且与
   ADR-3b"CPU 栈能力冻结"政策相撞,需裁决降级方案);**弹层体系 R3**
   (Select/Modal/Tooltip/Dropdown/Message 全家卡在这,约占 arco 高频组件半壁)。
5. **总量预估**:token 层 + 四波共约 30 个组件 ≈ **17–26 人周**(置信度中,
   §5),搭 R1–R3 的顺风车、不改主路线排期;新增渲染动词 2–4 人周计入 A3 波。

---

## 1. arco 体系核实(联网,逐项给出处)

### 1.1 定位、许可证与生态面

- **定位**:字节跳动企业级设计系统,"主要服务字节跳动中后台产品的体验设计和
  技术实现"([zhihu 官方介绍](https://www.zhihu.com/question/494828193)、
  [arco.design](https://arco.design/));React/Vue3 双实现
  ([arco-design](https://github.com/arco-design/arco-design) /
  [arco-design-vue](https://github.com/arco-design/arco-design-vue)),
  另有移动端 [arco-design-mobile](https://github.com/arco-design/arco-design-mobile)。
- **许可证**:`arco-design/arco-design` **MIT**、`arco-design/color` **MIT**
  (均经 GitHub License API `/repos/{repo}/license` 实查,spdx_id=MIT)。
- **npm**:[`@arco-design/web-react`](https://www.npmjs.com/package/@arco-design/web-react)
  ("60+ 组件、丰富 design token 可定制主题、TypeScript、暗色模式");主题定制
  双通道:less-loader 变量覆盖 或 [Design Lab](https://arco.design/themes)
  可视化配置 → 发布为 `@arco-themes/*` npm 主题包(实例:
  [`@arco-themes/react-ocean-design`](https://www.npmjs.com/package/@arco-themes/react-ocean-design)),
  主题包内核是 **token 名值对 JSON**(theme.json)——即 arco 官方自己就把
  token 当数据分发,给本报告 §4.2 的"转译而非手抄"提供了直接依据。

### 1.2 色板生成算法(源码全文核实,可直接移植 Rust)

[`@arco-design/color`](https://github.com/arco-design/color):任一基准色 →
10 档梯度色板,亮/暗双模式,API `generate(color, { index|list, dark, format })`,
预设 **14 组色板**(red/orangered/orange/gold/yellow/lime/green/cyan/blue/
arcoblue/purple/pinkpurple/magenta/gray)
([README.zh-CN](https://github.com/arco-design/color/blob/main/README.zh-CN.md))。

算法本体([src/palette.js](https://github.com/arco-design/color/blob/main/src/palette.js)
全文抓取,60 行):HSV 空间,**第 6 档=基准色**,向两端发散:

```js
const hueStep = 2;                      // 色相步长 2°
// h∈[60,240]:亮端左旋、暗端右旋;其余色域反向
// 亮端(i<6):s 向 9 递减(每步 (s-9)/5),v 向 100 递增(每步 (100-v)/5)
// 暗端(i>6):s 向 100 递增(每步 (100-s)/4),v 向 30 递减(每步 (v-30)/4)
```

暗色模式([palette-dark.js](https://github.com/arco-design/color/blob/main/src/palette-dark.js)):
取亮色板的**镜像档位**(`10-i+1`)的 h/v,饱和度按基准色相分三段修正
(h∈[0,50) 减 15、[50,191) 减 20、[191,360] 减 15)再线性展开。
**结论:纯确定性数学、无依赖,Rust 移植 ≤1 人日,拿 npm 包输出做金样对拍即可。**

### 1.3 全局 token(global.less 原文核实)

来源:[components/style/theme/global.less](https://github.com/arco-design/arco-design/blob/main/components/style/theme/global.less)
(经 jsdelivr 拉取原文逐行确认):

| 类别 | token | 值 |
|---|---|---|
| 圆角 | `border-radius-none/small/medium/large/circle` | 0 / 2px / 4px / 8px / 50% |
| 字号 | `font-size-body-1/2/3`、`title-1/2/3`、`display-1/2/3`、`caption` | 12/13/14、16/20/24、36/48/56、12 |
| 尺寸 | `size-1 … size-50` | 4px 等差到 200px |
| 间距 | `spacing-1 … spacing-22` | 2px–120px 非均匀阶梯 |
| 阴影 | `shadow-special`;`shadow1/2/3` ×9 方向 | `0 0 1px rgba(0,0,0,.3)`;blur 5/10/20px、偏移 2/4/8px、alpha 恒 0.1 |
| 文本色 | `color-text-1..4` | 标题正文→禁用四层 |
| 边框色 | `color-border-1..4` | 浅→深 |
| 功能色 | primary/success/warning/danger/link | 各自引用 10 档色板的 `rgb(var(--{palette}-{n}))` |

两个对本项目重要的观察:**① 阴影全系 alpha=0.1 的单层高斯,参数空间极小
(3 种 blur × 9 向偏移),适合预烘焙缓存(§3.2);② line-height 阶梯不在
global.less 中**(组件级 less 各自定义,官方 [token 文档页](https://arco.design/react/en-US/docs/token)
为 SPA 未能抓取正文,见 §7 未核清单)。

### 1.4 组件清单(GitHub contents API 实查,71 目录)

`components/` 下 71 项(去除 `_util/_class/_hooks/style/locale/index` 等 6 项
基建,实际组件 **65 个**):Affix, Alert, Anchor, AutoComplete, Avatar, BackTop,
Badge, Breadcrumb, Button, Calendar, Card, Carousel, Cascader, Checkbox,
Collapse, ColorPicker, Comment, ConfigProvider, DatePicker, Descriptions,
Divider, Drawer, Dropdown, Empty, Form, Grid, Icon, Image, Input, InputNumber,
InputTag, Layout, Link, List, Mentions, Menu, Message, Modal, Notification,
PageHeader, Pagination, Popconfirm, Popover, Portal, Progress, Radio, Rate,
ResizeBox, Result, Select, Skeleton, Slider, Space, Spin, Statistic, Steps,
Switch, Table, Tabs, Tag, TimePicker, Timeline, Tooltip, Transfer, Tree,
TreeSelect, Trigger, Typography, Upload, VerificationCode, Watermark。

### 1.5 图标与桌面先例

- 图标:[`@arco-design/web-react/icon`](https://arco.design/react/en-US/components/icon)
  内置图标为 **SVG React 组件**(`<IconXXX/>`),扩展走
  [IconBox](https://arco.design/iconbox) 平台;仓库内置图标随主仓 MIT。
  **确切数量未核实**(社区口径 200+,不引用为据)。**无官方 iconfont 字体档**
  ——本项目不能走"图标当字形塞 glyph_run"的捷径,见 §3.2。
- **桌面端适配先例:未找到可核实案例**。arco 自我定位是中后台 Web;检索未见
  官方或可信第三方的 Electron/原生桌面适配文档。这意味着"arco 视觉 → 桌面
  密度/命中区/键盘习惯"的再设计要自己做(风险 §6.5)。

---

## 2. 本仓库现状 vs arco 视觉需要什么(逐条代码证据)

### 2.1 已有的(够撑第一波组件)

| 能力 | 证据 |
|---|---|
| Style 13 字段:direction/gap/padding(Edges)/margin/border/bg/fg/font_size/width/height/corner_radius/opacity/cursor | `crates/sv-ui/src/lib.rs:125-142` |
| 渲染动词 5 个:fill_rounded_rect / stroke_rounded_rect / glyph_run / push_clip / pop_clip | `crates/sv-shell/src/paint.rs:85-102`(push/pop_clip 已落地,非"即将") |
| 真 CSS 语法封闭子集:`:root{--x}`+var()、继承(color/font-size)、:hover/:active、四值简写、hsl()/hex-alpha、cursor | DESIGN.md ADR-8 C1 ✅;CSS-SUPPORT.md "C1 已落地"节 |
| 元素:View/Text/Button/Checkbox/TextInput | `crates/sv-ui/src/lib.rs:191-202` |
| 焦点链+键盘四段路由+快捷键(R1 档 A ✅);TextInput+IME 进行中 | DESIGN.md §5 R1;sv-ui focusable 位 `lib.rs:212` |
| 双后端(CPU/vello)+ Painter 可切换;vello caps 已报 `blur:true` 待消费 | ADR-3b |

### 2.2 缺的(arco 视觉的硬依赖),与路线图的对应关系

| arco 视觉需要 | 本仓库现状 | 路线图归属 |
|---|---|---|
| box-shadow(shadow1/2/3,弹层与 Card 质感) | ⏳ 无排期,"vello 有高斯模糊后开;CPU 原型不做"(CSS-SUPPORT §E)| **路线图外**,与 ADR-3b CPU 冻结政策冲突,需裁决(§3.2) |
| 图标(SVG path 填充) | 无任意路径动词,Painter 只有圆角矩形+字形 | **路线图外**,需新增 `fill_path`(§3.2) |
| linear-gradient(Button 渐变态少量用) | ⏳(CSS-SUPPORT §E) | vello 后端可做,优先级低 |
| :focus/:disabled 伪类 | 📅 C2(焦点链已在,接线未做) | R1 收尾/C2 |
| font-weight(arco 标题 500/600) | 📅 C2 "Parley 前可先假粗斜"(CSS-SUPPORT §G);现状 swash 单字体单字重 | R3 Parley/fontique |
| transition(hover/active 缓动) | 📅 C2;帧调度 ADR-6 未实现 | C2 + ADR-6 |
| @media(暗色 prefers-color-scheme) | 📅 C2 | C2 |
| 滚动/overflow(Select 下拉列表、Table) | R2 方案已定(调研 22) | R2 |
| flex 对齐/换行(表单布局、Space/Grid) | R2 taffy(调研 23) | R2 |
| 弹层(Popup/Tooltip 离散层+锚定) | R3 方案已定(调研 25) | R3 |
| 文本省略 ellipsis(Typography/Table 单元格) | ⏳ 等 Parley | R3 |
| 图片(Avatar 头像/Image/Upload) | 无图片子系统,CSS-SUPPORT 标"独立议题" | **无排期** |

---

## 3. 逐组件差距表 + 两个路线图外工程项

### 3.1 组件 × 所需能力 × 可开工阶段

标记:✅=现有能力够 | 括号内为卡点。"降级版"指无图标/无阴影但布局配色圆角
字号全对齐 arco token 的形态。

**第一波 · R1 档 A 现状即可(降级版)**
| 组件 | 卡点备注 |
|---|---|
| Button | ✅ 4 状态(default/hover/active/disabled)×填充/描边/文字三型;loading 转圈缺图标+动画 |
| Tag / Badge | ✅ 纯色块+圆角+字号;可关闭 Tag 的 ✕ 用字形字符过渡 |
| Divider / Space | ✅(Space 复杂对齐等 taffy 更顺) |
| Alert | ✅ 色板底色+左侧色条;类型图标缺(降级为无图标) |
| Typography 子集 | ✅ 字号/色阶;粗体(500/600)失真到 R3,ellipsis 缺 |
| Link | ✅ :hover 色变;下划线 ✏️P2 可用 border 模拟 |
| Checkbox / Radio / Switch | ✅ Checkbox 已是内建元素;对勾用 stroke 折线或字形 ✓;Switch=圆角矩形+圆点位移(无缓动) |

**第二波 · R1 TextInput 落地后**
| 组件 | 卡点备注 |
|---|---|
| Input | TextInput 元素 + :focus 接线(C2)+ arco 边框三态色 |
| InputNumber | Input + 上下箭头(图标缺→字形 ▲▼ 过渡) |
| Form(纵向布局子集) | 标签+校验文案色;复杂对齐等 taffy |

**第三波 · R2(taffy + 滚动)后**
| 组件 | 卡点备注 |
|---|---|
| Card | 布局 ✅;hover 阴影缺(fill 边框降级) |
| Grid / Layout | taffy 直通 |
| List / Menu(平铺) | 滚动 + 虚拟化(ADR-9 现成);Menu 子菜单弹出卡 R3 |
| Progress / Skeleton / Spin | 直条 ✅;环形 Progress 需圆弧路径(fill_path);Spin/Skeleton 动画需帧循环(ADR-6 前可用重绘请求粗做) |
| Pagination / Steps / Breadcrumb / Statistic / Empty / Result | 布局件,taffy 后顺手;Steps/Result 图标降级 |
| Tabs | 布局+指示条 ✅;可滚动 Tabs 要 R2 滚动 |
| Slider / Rate | 命中+拖动 ✅ 指针捕获(R2 补齐);Rate 星形要 fill_path |

**第四波 · R3(弹层 + Parley)后**
| 组件 | 卡点备注 |
|---|---|
| Tooltip / Popover / Popconfirm | overlay_block + 锚定(调研 25);箭头三角要 fill_path;阴影 shadow2 |
| Dropdown / Select / AutoComplete / Cascader / TreeSelect | 弹层 + 滚动列表 + 图标(箭头/勾选) |
| Modal / Drawer | 弹层 + 遮罩(半透明 fill ✅)+ 进出场动效(transition C2) |
| Message / Notification | 弹层 + 定时(tasks 桥现成)+ 图标 |
| Table / Tree | 滚动+虚拟化 ✅;列宽 taffy;单元格 ellipsis 等 Parley;排序/筛选图标 |
| DatePicker / TimePicker / Calendar | 弹层 + 网格布局;工程量大但无新基建 |

**建议永不做/远期**(桌面无对应或依赖缺席子系统):Affix/BackTop/Anchor
(文档流滚动锚点语义)、Watermark、Carousel/Image/Upload/Avatar-图片态
(图片子系统无排期)、ColorPicker/VerificationCode/Transfer/Mentions/
Comment/PageHeader(低频,后评)。

### 3.2 两个路线图外的渲染工程项(sv-arco 的前置税)

1. **`fill_path` 动词 + SVG 图标编译期管线**(估 2–3 人周,置信度中):
   Painter 增 `fill_path(&[PathEl], color)`;tiny-skia(`Path`)与 vello
   (`kurbo::BezPath`)均原生支持,不违反 CPU 冻结政策(路径填充是两栈共有
   能力,非 ⏳ 项)。图标资产:build 期用 usvg 把 arco SVG 解析为路径数据表
   (`生成数据而非类型`,与 ADR-2 哲学一致),按需编入。**没有这项,arco
   视觉完成度上限约六成**——箭头/勾选/关闭/加载/类型提示图标无处不在。
2. **box-shadow 降级双轨**(估 1–2 人周,置信度中):vello 后端直接消费
   `blur:true`(ADR-3b 已预留);CPU 后端**不做实时高斯**(守住冻结政策),
   改为"预烘焙圆角矩形阴影九宫格贴图 + 缓存"——arco 阴影参数空间仅
   3 blur × alpha 0.1(§1.3),缓存键极小,视觉误差可接受。两后端 parity
   放宽为"结构一致、模糊边缘允差"。

---

## 4. 落地形态裁决

### 4.1 形态:独立 `sv-arco` crate 族 + .sv 组件 + 编译期 token

```
crates/sv-arco-tokens   token 常量 + 色板算法(纯 Rust,零依赖)
crates/sv-arco          组件库(.sv 单文件组件 + build.rs 走 sv-compiler)
```

- **独立 crate,不进 sv-ui**:sv-ui 是宏的编译目标(CLAUDE.md 约束),
  组件库是其消费者;arco 只是"第一个官方皮肤",内核不得绑定任何设计系统。
- **.sv 组件形态**:组件库是 .sv 前端最好的实弹靶场(counter-sfc 之后规模
  最大的 dogfooding),同时倒逼 R1–R3 的原语(bind:value/overlay_block/
  `:focus`)在真组件中验收;个别性能敏感件(Table 行)可退 view! 宏。
- **token = `:root` 变量 + Rust 常量双出口**:转译脚本产出
  ① `:root { --color-primary-6: ...; --border-radius-medium: 4px; ... }`
  (走已落地的 var() 编译期代入,组件 .sv 里写 `var(--color-primary-6)`);
  ② `pub const` Rust 常量(给 view! 宏与运行时逻辑用)。暗色主题等 C2
  @media/prefers-color-scheme 落地后换表。

### 4.2 token 获取:一次性**脚本转译**,不手抄

- 静态 token:脚本读 `global.less`(pin 到 `@arco-design/web-react` 具体
  版本,如 2.66.x)→ 生成 Rust/CSS 双出口 + 来源注释(变量名、原值、版本);
  脚本入库,arco 升版时重跑 + diff 审查。**手抄被否**:token 数量大
  (size 50 档 + spacing 22 档 + 阴影 28 个 + 色板 14×10×2 模式)、
  易漂移、无法审计。
- 色板:**算法移植**(§1.2,60 行数学)而非抄 140 个色值——换主色生成
  自有品牌主题是 arco 体系的核心卖点,抄值会把这个能力抄丢;金样测试
  对拍 `@arco-design/color` npm 输出锁行为。
- 组件级 token(千余个,Design Lab 口径):**不整体转译**,按组件落地时
  从对应 less 就近取值,记录在该组件的注释里——全量转译的维护面不可承受。

### 4.3 许可合规

- 代码/token/算法:MIT 允许复制、修改、再许可;义务仅"保留版权声明与
  许可文本"。动作:`sv-arco` 仓内放 `LICENSE-ARCO`(arco MIT 原文)+
  NOTICE 段落("视觉规范与 design token 派生自 ByteDance Arco Design,
  MIT License");转译产物文件头带来源注释。
- **商标注意**:MIT 不授予 "Arco" 名称/商标权。crate 名建议 `sv-arco`
  并在 README 声明非官方、无背书("unofficial, not affiliated with
  ByteDance");若未来商用发行,评估改中性名(如 `sv-kit-arco` 主题包
  形态)的成本(低,crate 名即可)。
- 图标:仅使用主仓随 MIT 分发的内置图标;IconBox 平台上的第三方上传
  图标**不可假定 MIT**,不纳入。

## 5. 分期计划(与 R1–R5 的插入点,人周单人粗估)

| 波次 | 内容 | 前置(插入点) | 人周(置信度) |
|---|---|---|---|
| A0 token 层 | sv-arco-tokens:色板算法移植 + global.less 转译脚本 + 金样对拍 + :root/常量双出口 | 无——**今天可开工**(C1 var() 已落地) | 1.5–2(高) |
| A1 静态六件 | Button/Tag/Badge/Divider/Alert/Typography 子集(降级版,无图标) | R1 档 A(已达成) | 2–3(高) |
| A2 表单件 | Input/Checkbox/Radio/Switch/InputNumber + :focus 接线验收 | R1 TextInput(进行中)+ C2 :focus | 3–4(中) |
| A3 渲染动词补齐 | fill_path + SVG 图标管线 + box-shadow 双轨(§3.2);回填 A1/A2 图标 | 可与 R2 并行(改 sv-shell,不动 R2 范围) | 4–7(中) |
| A4 容器布局件 | Card/Grid/List/Menu 平铺/Tabs/Progress/Pagination/Steps/Skeleton | R2(taffy+滚动) | 3–5(中) |
| A5 弹层件 | Tooltip/Dropdown/Select/Modal/Message/Popconfirm | R3 弹层 + transition C2 | 5–8(低——弹层组件交互态最密) |
| A6 数据重件 | Table/Tree/DatePicker 族 | R3 全部 + A5 | 另计(每件 2–4) |

合计 A0–A5 ≈ **17–26 人周**,产出约 30 组件。关键性质:**A 波次全程不改
R1–R5 主路线的范围与排期**,只消费其产物;唯一新增的仓库级工程是 A3 的两个
渲染动词(sv-shell 边界内,Painter trait 加法,RecordingPainter 金样可保底)。
建议验收样板:用 sv-arco 重写 showcase 为"arco 风设置面板 + TodoMVC",
与 R2/R3 的验收 demo 合流。

## 6. 风险清单(按杀伤力)

1. **图标管线是隐形深坑**:动词好加,资产管线难精——SVG 解析边界
   (arco 图标以 stroke 为主,usvg 需描边转填充)、HiDPI 光栅质量、
   按需编入避免二进制膨胀。图标不像,整个库"不像 arco"。
2. **字重缺失导致标题层级失真**:swash 现为单字体单字重,arco 的
   Typography/标题依赖 500/600 字重;R3 Parley/fontique 前只有"假粗"
   (质量差)或"全 regular"(层级塌)两个坏选项。A1 的 Typography
   只能交降级版,预期管理要写进 README。
3. **静态 arco ≠ arco 质感**:hover/active 缓动、Modal 进出场、Message
   滑入——arco 的"高级感"一半在动效;transition(C2)与帧调度(ADR-6)
   落地前,交付物是"截图像、上手糙"。评审时要按波次对齐预期。
4. **CPU/vello 视觉一致性放宽的先例风险**:§3.2 的阴影双轨打破了
   "parity 1.001"的严格口径;需要在 ADR 里把"允差项清单"写死,防止
   后续能力借道扩散、冻结政策名存实亡。
5. **桌面适配无先例可抄**:arco 密度/命中区按 Web 中后台设计(未核实到
   任何桌面端官方适配),桌面惯例(更小行高?右键菜单?键盘全覆盖)需
   自行定标准——这是设计工作,不是工程工作,单人团队易低估。
6. **token 漂移**:arco 活跃演进(web-react 2.66.x),pin 版本 + 转译脚本
   diff 审查可控,但组件级 token"就近取值"部分(§4.2)无自动追踪,升版
   靠人工比对。
7. **范围失控**:65 个组件全量复刻不现实;§3.1 的"永不做/远期"清单要
   进 README 当承诺边界,防止 issue 驱动的范围蔓延。

## 7. 出处

联网(2026-07-18 核实):
- arco 主仓(MIT、65 组件目录、README):<https://github.com/arco-design/arco-design>(许可证与 components 目录经 GitHub API 实查)
- global.less token 原文:<https://github.com/arco-design/arco-design/blob/main/components/style/theme/global.less>(经 jsdelivr 原文拉取)
- 色板算法:<https://github.com/arco-design/color>(MIT;[README.zh-CN](https://github.com/arco-design/color/blob/main/README.zh-CN.md)、[palette.js](https://github.com/arco-design/color/blob/main/src/palette.js)、[palette-dark.js](https://github.com/arco-design/color/blob/main/src/palette-dark.js) 源码全文)
- npm 包与 token 定制:<https://www.npmjs.com/package/@arco-design/web-react>;Design Lab 主题平台 <https://arco.design/themes>;主题包实例 <https://www.npmjs.com/package/@arco-themes/react-ocean-design>
- 图标:<https://arco.design/react/en-US/components/icon>;IconBox <https://arco.design/iconbox>
- 定位(中后台):<https://www.zhihu.com/question/494828193>;<https://arco.design/>
- Vue/移动端实现:<https://github.com/arco-design/arco-design-vue>;<https://github.com/arco-design/arco-design-mobile>

本仓库代码证据:`crates/sv-ui/src/lib.rs:125-142`(Style)、`:191-202`
(ElementKind)、`crates/sv-shell/src/paint.rs:85-102`(Painter 5 动词)、
`docs/CSS-SUPPORT.md`(box-shadow/渐变/font-family ⏳,transition/@media C2)、
`docs/DESIGN.md` ADR-3b/ADR-8/§5 R1–R5、调研 20–25。

**未核清单**(检索到但未能证实,不作为依据):arco 官方 token 文档页正文
(SPA 抓取失败,token 以 global.less 源码为准)、内置图标确切数量、
line-height 阶梯 token、任何桌面端(Electron/原生)官方适配案例、
Design Lab 组件级 token 的完整清单规模("千余个"为官方宣传口径)。
