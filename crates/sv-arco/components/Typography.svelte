<script>
// Arco Typography 子集(A1 降级版:单 text 叶;无 bold/mark/code/underline/
// delete/blockquote/copy/edit/ellipsis——行内混排与图标/省略能力缺席,整块裁剪;
// 标题字重 500(token L12)单字重表达不了,层级只靠字号;margin 一律不带,
// 排版交给容器 gap)。
//
// 组件级取值出处:assets/typography-token.less(@arco-design/web-react 2.66.16):
//   字号档 title-1/2/3 = arco h6/h5/h4(16/20/24,L8/L7/L6),display-1 =
//   arco h1(36,L3);display-2/3(48/56)是我方 token 扩展档;arco h2(32)/
//   h3(28)在我方 token 无对应档,不收录。
//   色档 默认 text-1(L14);secondary = text-2(L17,token 原文如此);
//   primary/success/warning/danger = 功能色-6(L16/L18/L19/L20,arco 命名
//   error → 我方 danger)。
$props {
    text: String,
    kind: String = String::from("body"), // body|caption|title-1..3|display-1..3|secondary|primary|success|warning|danger
}
</script>

<text
  class="ty"
  class:kd-caption={kind == "caption"}
  class:kd-title-1={kind == "title-1"}
  class:kd-title-2={kind == "title-2"}
  class:kd-title-3={kind == "title-3"}
  class:kd-display-1={kind == "display-1"}
  class:kd-display-2={kind == "display-2"}
  class:kd-display-3={kind == "display-3"}
  class:kd-secondary={kind == "secondary"}
  class:kd-primary={kind == "primary"}
  class:kd-success={kind == "success"}
  class:kd-warning={kind == "warning"}
  class:kd-danger={kind == "danger"}
>{text}</text>

<style>
/*ARCO_TOKENS*/

/* body 基态:默认文本色 + 正文字号 */
.ty { fg: var(--color-text-1); font-size: var(--font-size-body-3); }

/* 字号档(色沿用基态 text-1,arco 标题不换色) */
.kd-caption { font-size: var(--font-size-caption); }
.kd-title-1 { font-size: var(--font-size-title-1); }
.kd-title-2 { font-size: var(--font-size-title-2); }
.kd-title-3 { font-size: var(--font-size-title-3); }
.kd-display-1 { font-size: var(--font-size-display-1); }
.kd-display-2 { font-size: var(--font-size-display-2); }
.kd-display-3 { font-size: var(--font-size-display-3); }

/* 色档(字号沿用基态 body-3) */
.kd-secondary { fg: var(--color-text-2); }
.kd-primary { fg: var(--primary-6); }
.kd-success { fg: var(--success-6); }
.kd-warning { fg: var(--warning-6); }
.kd-danger { fg: var(--danger-6); }
</style>
