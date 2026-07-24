<script>
// Arco Alert(A1 降级版:无状态图标 / 无关闭按钮 / 无 banner / 无 action 区;
// 非交互组件,无伪类无 disabled)。
//
// 组件级取值出处:assets/alert-token.less(@arco-design/web-react 2.66.16):
//   容器 圆角 radius-small(L8);padding 无标题 9px/16px、有标题 16px 全向
//        (L13-L16)—— 1px transparent 边框(L3 及各态 border)画不了,
//        并入 padding 保几何:10px/17px 与 17px 全向
//   四态底色 = 状态-light-1 → 亮色即色板第 1 档(info L30 / warning L37 /
//        error L44 / success L51;info→primary=arcoblue,error→danger=red)
//   标题 title-1(16px,L19)text-1 色;正文 body-3(14px,L20),
//        无标题 text-1、有标题降 text-2(L34-L35 等);标题-正文 gap =
//        spacing-2(4px,L11);font-weight 500(L18)表达不了,层级靠字号
$props {
    title: String = String::new(), // 空 = 不渲染标题行
    content: String,
    status: String = String::from("info"), // info | success | warning | error(arco Alert 语义)
}
// 条件全用 Copy 的 bool:String 进多个闭包会 move 冲突(sv-compiler 预克隆
// 只覆盖单闭包,已登记 open-issues),title 本体只进标题分支
let has_title = !title.is_empty();
</script>

<view
  class="alert st-info"
  class:st-success={status == "success"}
  class:st-warning={status == "warning"}
  class:st-error={status == "error"}
  class:pad-title={has_title}
>
  <view class="alert-body">
    {#if has_title}
      <text class="alert-title">{title}</text>
    {/if}
    <text class="alert-content" class:content-with-title={has_title}>{content}</text>
  </view>
</view>

<style>
/*ARCO_TOKENS*/

/* 容器:padding 已含 1px 边框折算(9+1 / 16+1) */
.alert { direction: row; radius: var(--border-radius-small); padding: 10px 17px; }

/* 带标题:padding 16px 全向 + 1px 折算 */
.pad-title { padding: 17px; }

/* 四态底色 = 状态-light-1(亮色即色板第 1 档) */
.st-info { bg: var(--primary-1); }
.st-success { bg: var(--success-1); }
.st-warning { bg: var(--warning-1); }
.st-error { bg: var(--danger-1); }

/* 文字区:纵排,标题-正文 gap 4px,撑满余宽 */
.alert-body { direction: column; gap: 4px; flex-grow: 1; }

.alert-title { font-size: var(--font-size-title-1); fg: var(--color-text-1); }
.alert-content { font-size: var(--font-size-body-3); fg: var(--color-text-1); }

/* 有标题时正文降 text-2 */
.content-with-title { fg: var(--color-text-2); }
</style>
