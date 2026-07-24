# sv-arco

Arco Design 风格组件库(调研 26 的落地),`.svelte` 单文件组件形态,
设计令牌来自 [`sv-arco-tokens`](../sv-arco-tokens/)。

**状态:A1 波次起步 —— 一次一个组件,离屏 PNG 视觉把关。**

## 组件清单与波次

按调研 26 §3.1 / §5 的分波推进(✅ 已落地 / ⏳ 排队):

- **A1 静态件**:Button ⏳(首件)、Tag、Badge、Divider、Alert、Typography
  子集、Link
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
- **无过渡动效**:transition(C2)未落地,hover/active 是瞬时切换。
- **disabled 无 `:disabled` 伪类**(C2):以 prop 切换整套配色实现,
  点击回调同时短路。

组件级取值(尺寸/配色矩阵)按 §4.2 就近抄自对应组件的 `token.less`,
原文 vendored 于 `assets/`(如 `button-token.less`),出处注释写在各
`.svelte` 里。

## 许可与署名

视觉规范与 design token 派生自 ByteDance **Arco Design**(MIT,原文见
[`LICENSE-ARCO`](./LICENSE-ARCO);来源版本 pin 见 sv-arco-tokens README)。
本 crate 为**非官方**实现,与 ByteDance 无关联、未获其背书。
