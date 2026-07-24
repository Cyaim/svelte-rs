<script>
// Arco Button(A1 降级版:无图标/无阴影/无过渡;伪类只有 :hover/:active,
// disabled 用条件类整套换色 + 回调短路)。
//
// 组件级取值出处:assets/button-token.less(@arco-design/web-react 2.66.16):
//   尺寸  高 = size-mini/small/default/large(24/28/32/36),
//         横 padding = 11/15/15/19px,字号 = body-1/body-3×3(12/14/14/14),
//         圆角 = radius-small(2px),边框 = 1px
//   配色  primary:  bg P6 → hover P5 → active P7,文字 #fff,禁用 bg P3
//         secondary:bg fill-2 → fill-3 → fill-4,文字 text-2,禁用 bg fill-1 文字 text-4
//                   (状态色下 bg = 状态-1/2/3,文字 = 状态-6,禁用文字 状态-3)
//         outline:  边框+文字 P6 → P5 → P7,bg 透明,禁用 P3
//         text:     文字 P6 恒定,bg 透明 → fill-2 → fill-3,禁用文字 P3
//   (P = 状态对应色板:default→primary,warning/danger/success→同名)
$props {
    label: String,
    variant: String = String::from("secondary"), // primary | secondary | outline | text
    status: String = String::from("default"),    // default | warning | danger | success
    size: String = String::from("default"),      // mini | small | default | large
    disabled: bool = false,
    on_click: std::rc::Rc<dyn Fn()> = std::rc::Rc::new(|| {}),
}
</script>

<button
  class="btn sz-default"
  class:sz-mini={size == "mini"}
  class:sz-small={size == "small"}
  class:sz-large={size == "large"}
  class:btn-primary={!disabled && variant == "primary" && status == "default"}
  class:btn-primary-warning={!disabled && variant == "primary" && status == "warning"}
  class:btn-primary-danger={!disabled && variant == "primary" && status == "danger"}
  class:btn-primary-success={!disabled && variant == "primary" && status == "success"}
  class:btn-primary-off={disabled && variant == "primary" && status == "default"}
  class:btn-primary-warning-off={disabled && variant == "primary" && status == "warning"}
  class:btn-primary-danger-off={disabled && variant == "primary" && status == "danger"}
  class:btn-primary-success-off={disabled && variant == "primary" && status == "success"}
  class:btn-secondary={!disabled && variant == "secondary" && status == "default"}
  class:btn-secondary-warning={!disabled && variant == "secondary" && status == "warning"}
  class:btn-secondary-danger={!disabled && variant == "secondary" && status == "danger"}
  class:btn-secondary-success={!disabled && variant == "secondary" && status == "success"}
  class:btn-secondary-off={disabled && variant == "secondary" && status == "default"}
  class:btn-secondary-warning-off={disabled && variant == "secondary" && status == "warning"}
  class:btn-secondary-danger-off={disabled && variant == "secondary" && status == "danger"}
  class:btn-secondary-success-off={disabled && variant == "secondary" && status == "success"}
  class:btn-outline={!disabled && variant == "outline" && status == "default"}
  class:btn-outline-warning={!disabled && variant == "outline" && status == "warning"}
  class:btn-outline-danger={!disabled && variant == "outline" && status == "danger"}
  class:btn-outline-success={!disabled && variant == "outline" && status == "success"}
  class:btn-outline-off={disabled && variant == "outline" && status == "default"}
  class:btn-outline-warning-off={disabled && variant == "outline" && status == "warning"}
  class:btn-outline-danger-off={disabled && variant == "outline" && status == "danger"}
  class:btn-outline-success-off={disabled && variant == "outline" && status == "success"}
  class:btn-text={!disabled && variant == "text" && status == "default"}
  class:btn-text-warning={!disabled && variant == "text" && status == "warning"}
  class:btn-text-danger={!disabled && variant == "text" && status == "danger"}
  class:btn-text-success={!disabled && variant == "text" && status == "success"}
  class:btn-text-off={disabled && variant == "text" && status == "default"}
  class:btn-text-warning-off={disabled && variant == "text" && status == "warning"}
  class:btn-text-danger-off={disabled && variant == "text" && status == "danger"}
  class:btn-text-success-off={disabled && variant == "text" && status == "success"}
  onclick={move || if !disabled { on_click() }}
>{label}</button>

<style>
/*ARCO_TOKENS*/

.btn { radius: var(--border-radius-small); cursor: pointer; }

.sz-mini { height: var(--size-mini); padding: 0 11px; font-size: var(--font-size-body-1); }
.sz-small { height: var(--size-small); padding: 0 15px; font-size: var(--font-size-body-3); }
.sz-default { height: var(--size-default); padding: 0 15px; font-size: var(--font-size-body-3); }
.sz-large { height: var(--size-large); padding: 0 19px; font-size: var(--font-size-body-3); }

.btn-primary { bg: var(--primary-6); fg: #ffffff;
  &:hover { bg: var(--primary-5); }
  &:active { bg: var(--primary-7); } }
.btn-primary-warning { bg: var(--warning-6); fg: #ffffff;
  &:hover { bg: var(--warning-5); }
  &:active { bg: var(--warning-7); } }
.btn-primary-danger { bg: var(--danger-6); fg: #ffffff;
  &:hover { bg: var(--danger-5); }
  &:active { bg: var(--danger-7); } }
.btn-primary-success { bg: var(--success-6); fg: #ffffff;
  &:hover { bg: var(--success-5); }
  &:active { bg: var(--success-7); } }
.btn-primary-off { bg: var(--primary-3); fg: #ffffff; cursor: not-allowed; }
.btn-primary-warning-off { bg: var(--warning-3); fg: #ffffff; cursor: not-allowed; }
.btn-primary-danger-off { bg: var(--danger-3); fg: #ffffff; cursor: not-allowed; }
.btn-primary-success-off { bg: var(--success-3); fg: #ffffff; cursor: not-allowed; }

.btn-secondary { bg: var(--color-fill-2); fg: var(--color-text-2);
  &:hover { bg: var(--color-fill-3); }
  &:active { bg: var(--color-fill-4); } }
.btn-secondary-warning { bg: var(--warning-1); fg: var(--warning-6);
  &:hover { bg: var(--warning-2); }
  &:active { bg: var(--warning-3); } }
.btn-secondary-danger { bg: var(--danger-1); fg: var(--danger-6);
  &:hover { bg: var(--danger-2); }
  &:active { bg: var(--danger-3); } }
.btn-secondary-success { bg: var(--success-1); fg: var(--success-6);
  &:hover { bg: var(--success-2); }
  &:active { bg: var(--success-3); } }
.btn-secondary-off { bg: var(--color-fill-1); fg: var(--color-text-4); cursor: not-allowed; }
.btn-secondary-warning-off { bg: var(--warning-1); fg: var(--warning-3); cursor: not-allowed; }
.btn-secondary-danger-off { bg: var(--danger-1); fg: var(--danger-3); cursor: not-allowed; }
.btn-secondary-success-off { bg: var(--success-1); fg: var(--success-3); cursor: not-allowed; }

.btn-outline { border: 1px solid var(--primary-6); fg: var(--primary-6);
  &:hover { border: 1px solid var(--primary-5); fg: var(--primary-5); }
  &:active { border: 1px solid var(--primary-7); fg: var(--primary-7); } }
.btn-outline-warning { border: 1px solid var(--warning-6); fg: var(--warning-6);
  &:hover { border: 1px solid var(--warning-5); fg: var(--warning-5); }
  &:active { border: 1px solid var(--warning-7); fg: var(--warning-7); } }
.btn-outline-danger { border: 1px solid var(--danger-6); fg: var(--danger-6);
  &:hover { border: 1px solid var(--danger-5); fg: var(--danger-5); }
  &:active { border: 1px solid var(--danger-7); fg: var(--danger-7); } }
.btn-outline-success { border: 1px solid var(--success-6); fg: var(--success-6);
  &:hover { border: 1px solid var(--success-5); fg: var(--success-5); }
  &:active { border: 1px solid var(--success-7); fg: var(--success-7); } }
.btn-outline-off { border: 1px solid var(--primary-3); fg: var(--primary-3); cursor: not-allowed; }
.btn-outline-warning-off { border: 1px solid var(--warning-3); fg: var(--warning-3); cursor: not-allowed; }
.btn-outline-danger-off { border: 1px solid var(--danger-3); fg: var(--danger-3); cursor: not-allowed; }
.btn-outline-success-off { border: 1px solid var(--success-3); fg: var(--success-3); cursor: not-allowed; }

.btn-text { fg: var(--primary-6);
  &:hover { bg: var(--color-fill-2); }
  &:active { bg: var(--color-fill-3); } }
.btn-text-warning { fg: var(--warning-6);
  &:hover { bg: var(--color-fill-2); }
  &:active { bg: var(--color-fill-3); } }
.btn-text-danger { fg: var(--danger-6);
  &:hover { bg: var(--color-fill-2); }
  &:active { bg: var(--color-fill-3); } }
.btn-text-success { fg: var(--success-6);
  &:hover { bg: var(--color-fill-2); }
  &:active { bg: var(--color-fill-3); } }
.btn-text-off { fg: var(--primary-3); cursor: not-allowed; }
.btn-text-warning-off { fg: var(--warning-3); cursor: not-allowed; }
.btn-text-danger-off { fg: var(--danger-3); cursor: not-allowed; }
.btn-text-success-off { fg: var(--success-3); cursor: not-allowed; }
</style>
