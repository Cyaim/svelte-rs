# sv-arco

Arco Design 风格组件库(调研 26 的落地),`.svelte` 单文件组件形态,
设计令牌来自 [`sv-arco-tokens`](../sv-arco-tokens/)。

**状态:A1 静态件七件全部落地(离屏 PNG 视觉验收,arco-gallery 可复现)。**

## 组件清单与波次

按调研 26 §3.1 / §5 的分波推进(✅ 已落地 / ⏳ 排队):

- **A1 静态件 ✅**:Button(全矩阵)、Tag(14 色 × 4 档)、Badge
  (standalone:count/dot)、Divider(纯线/带字/纵向)、Alert(四状态 ×
  有/无标题)、Typography 子集(字号阶梯 + 色档)、Link(四状态 + 禁用)
- **A2 表单件**:Input、Checkbox、Radio、Switch、InputNumber
- **A4 容器布局件**:Card、Grid、List、Menu(平铺)、Tabs、Progress、
  Pagination、Steps、Skeleton
- **A5 弹层件**:Tooltip、Dropdown、Select、Modal、Message、Popconfirm
- **A6 数据重件**:Table、Tree、DatePicker 族

**承诺边界(不做/远期)**:Affix / BackTop / Anchor(文档流滚动锚点语义,
桌面无对应)、Watermark、Carousel / Image / Upload / Avatar-图片态(图片
子系统无排期)、ColorPicker / VerificationCode / Transfer / Mentions /
Comment / PageHeader(低频,后评)。

## 降级口径(A1 阶段,预期管理)

- **无图标**:`fill_path` 动词已在,但 SVG 图标编译期管线(A3)未接;
  可关闭 Tag 的 ✕、loading 转圈等先用字形字符或缺省。
- **无阴影**:box-shadow 渲染动词未落地(CSS-SUPPORT ⏳)。
- **单字重**:标题 500/600 字重等 fontique 字重选择接线,当前全 regular。
- **无过渡动效**:transition(C2)未落地,hover/active 是瞬时切换(切换
  本身生效——条件类上的 `:active`/`:focus` 曾被 codegen 静默丢弃,本批
  已修,见 CHANGELOG)。
- **1px 透明边框未补偿几何**:arco 各变体带 1px 透明边框(border-box),
  Button 非 outline 变体与 Tag 未把这 1px 折进 padding(Alert 折了),
  故同 label 的 outline 比 primary 宽 2px、Tag 横向少 1px/侧——**亚感知
  级色差,视觉可忽略**,已登记 open-issues。
- **disabled 无 `:disabled` 伪类**(C2):以条件类切换整套配色实现,
  点击回调同时短路。

组件级取值(尺寸/配色矩阵)按 §4.2 就近抄自对应组件的 `token.less`,
原文 vendored 于 `assets/`(如 `button-token.less`),出处注释写在各
`.svelte` 里。逐组件的裁决与降级细节(如 Badge 只做 standalone、
Typography 的 secondary 取 text-2、Link 的 warning 禁用色是 light-2、
Tag 的 `color="gray"` 走"arco 无 color 的默认外观"fill-2/text-1 而非
arco 自带的 gray 预设 gray-2/gray-6)见各组件头注释;编译器层的绕行
(if 块包装节点挡拉伸 → Divider 恒渲染+条件类清零;plain prop 进多同级
闭包 move 冲突 → 每分支克隆副本)已登记 `docs/plans/open-issues.md`。

## 组件形态约束(消费者须知)

- **Divider 横向形态要放在 `align-items: stretch` 的纵向容器里**才有宽度
  (本渲染栈 align-items 缺省是 start,没有百分比宽度);Alert 同理靠父
  容器给宽。arco-gallery 里有可抄的用法。
- variant/status/size/color/kind 这类枚举 prop 都是 `String`,**拼错静默
  失效**(不 panic 不报错):默认形态由静态类承载的组件(Tag/Alert/Badge/
  Typography、以及 Button 的 size)会落到默认;而 Button 的 variant/status
  与 Link 的 status——默认外观由条件类携带——拼错会落到**无变体裸基类**
  (透明底、无字色),不是 secondary/link 默认形态。对照各组件注释的合法值表。

## 许可与署名

视觉规范与 design token 派生自 ByteDance **Arco Design**(MIT,原文见
[`LICENSE-ARCO`](./LICENSE-ARCO);来源版本 pin 见 sv-arco-tokens README)。
本 crate 为**非官方**实现,与 ByteDance 无关联、未获其背书。
