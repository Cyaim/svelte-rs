<script>
// Arco Divider(A1:横向纯线 / 线-字-线 / 纵向短线;非交互)。
//
// 组件级取值出处:assets/divider-token.less(@arco-design/web-react 2.66.16):
//   线厚 1px(L12),线色 = color-neutral-3 → gray-3(L14);
//   横向上下 margin = spacing-8(20px,L4/L5);线-字间距 = spacing-7(16px,L6);
//   文字 = text-1 色(L15)+ body-3 字号(L9);纵向左右 margin = spacing-6
//   (12px,L3),线高固化 14px(arco 是 1em,无 em 单位按 body-3 上下文取值)。
//   单边框(border-bottom)不支持 → 1px 高填充 view 等价;font-weight 500
//   (L10)单字重表达不了。
//
// 结构上刻意**不用 {#if} 分支**:if 块会引入一个参与布局的包装 View
// (column/start,见 sv_ui::if_block),flex 拉伸穿不过它,横线会塌成零宽
// (已登记 open-issues)。改为恒渲染"线-字-线"扁平结构 + 条件类切形态:
//   横向纯线 = 文字置空(零宽 text)+ 两段 flex-grow 线无缝拼合;
//   纵向     = 根自己变成 1×14 竖线,三个子件全部清零。
//
// ⚠ 横向形态依赖父容器交叉轴拉伸:放在 align-items: stretch 的纵向容器里
//   才有宽度(本渲染栈 align-items 缺省是 start,不是 CSS 的 stretch)。
$props {
    text: String = String::new(), // 空 = 纯线;非空 = 线-字-线(居中)
    vertical: bool = false,       // 纵向短线(忽略 text)
}
let has_text = !text.is_empty() && !vertical;
let mid = if vertical { String::new() } else { text };
</script>

<view class="dv" class:dv-v={vertical} class:with-gap={has_text}>
  <view class="dv-line" class:dv-zero={vertical} />
  <text class="dv-text">{mid}</text>
  <view class="dv-line" class:dv-zero={vertical} />
</view>

<style>
/*ARCO_TOKENS*/

/* 横向根:行向容器,上下 margin;纯线态 gap=0,两段线拼成一条 */
.dv { direction: row; align-items: center; margin: 20px 0; }

/* 带字态:线-字间距 = spacing-7 */
.with-gap { gap: var(--spacing-7); }

/* 线段:flex-grow 均分 → 文字居中;纯线态两段无缝相接 */
.dv-line { height: 1px; bg: var(--gray-3); flex-grow: 1; }

.dv-text { fg: var(--color-text-1); font-size: var(--font-size-body-3); }

/* 纵向:根自己就是 1×14 竖线,左右 margin = spacing-6;子件由 dv-zero 清零 */
.dv-v { direction: row; width: 1px; height: 14px; bg: var(--gray-3); margin: 0 12px; }
.dv-zero { width: 0; height: 0; flex-grow: 0; }
</style>
