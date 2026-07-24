<script>
// Arco Link(A1 降级版:button 叶承载——arco 是内联 <a>,此处按块级叶子参与
// flex 布局,导航语义一律走 on_click;无下划线(单边框不支持)、无图标、
// 无 focus 光环(box-shadow 缺席)、无 hoverable=false 变体)。
//
// 组件级取值出处:assets/link-token.less(@arco-design/web-react 2.66.16):
//   几何 字号 body-3(L3);padding 纵 1px(L30)横 spacing-2 = 4px(L7);
//        圆角 radius-small(L32)
//   配色 default 文字 link-6(L9),hover/active 文字不变(L10/L11),
//        hover 底 fill-2(L5)、active 底 fill-3(L6);success/warning/danger
//        文字 = 状态-6(L14/L24/L19),底色同上
//   禁用 default → link-3(L12)、success → success-3(L17)、danger →
//        danger-3(L22)、warning → warning-2(L27,arco 原文就是 light-2
//        不是 light-3,照抄不"修正")+ not-allowed,无 hover/active 底
$props {
    label: String,
    status: String = String::from("default"), // default | success | warning | danger
    disabled: bool = false,
    on_click: std::rc::Rc<dyn Fn()> = std::rc::Rc::new(|| {}),
}
</script>

<button
  class="link"
  class:link-default={!disabled && status == "default"}
  class:link-success={!disabled && status == "success"}
  class:link-warning={!disabled && status == "warning"}
  class:link-danger={!disabled && status == "danger"}
  class:link-default-off={disabled && status == "default"}
  class:link-success-off={disabled && status == "success"}
  class:link-warning-off={disabled && status == "warning"}
  class:link-danger-off={disabled && status == "danger"}
  onclick={move || if !disabled { on_click() }}
>{label}</button>

<style>
/*ARCO_TOKENS*/

/* 几何:line-height(L4)表达不了,高度 = 文本行高 + 纵 padding 1px */
.link { padding: 1px 4px; font-size: var(--font-size-body-3); radius: var(--border-radius-small); cursor: pointer; }

.link-default { fg: var(--link-6);
  &:hover { bg: var(--color-fill-2); }
  &:active { bg: var(--color-fill-3); } }
.link-success { fg: var(--success-6);
  &:hover { bg: var(--color-fill-2); }
  &:active { bg: var(--color-fill-3); } }
.link-warning { fg: var(--warning-6);
  &:hover { bg: var(--color-fill-2); }
  &:active { bg: var(--color-fill-3); } }
.link-danger { fg: var(--danger-6);
  &:hover { bg: var(--color-fill-2); }
  &:active { bg: var(--color-fill-3); } }

/* 禁用:换浅档 + not-allowed,无 hover/active 底 */
.link-default-off { fg: var(--link-3); cursor: not-allowed; }
.link-success-off { fg: var(--success-3); cursor: not-allowed; }
.link-warning-off { fg: var(--warning-2); cursor: not-allowed; }
.link-danger-off { fg: var(--danger-3); cursor: not-allowed; }
</style>
