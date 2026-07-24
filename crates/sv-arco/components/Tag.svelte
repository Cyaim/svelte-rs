<script>
// Arco Tag(A1 降级版:非 checkable / 不可关闭 / 无图标 / 无 bordered;
// 无交互 → 无 :hover/:active/disabled)。
//
// 组件级取值出处:assets/tag-token.less(@arco-design/web-react 2.66.16):
//   尺寸  高 = size-5/6/7/8(20/24/28/32),对应 small/default/medium/large
//         (注意 default=24 与 medium=28 是不同档),横 padding = 8px 全档一致,
//         字号 = 12/12/14/14,圆角 = radius-small(2px)
//   配色  gray(默认)= arco 不传 color 的默认 Tag:bg fill-2,文字 text-1。
//         ⚠ 这刻意**不等于** arco 自带的 gray 预设(@tag-gray-* = gray-2 底/
//         gray-6 字);我们把 color="gray" 映射到"无 color 默认外观",取更
//         常用的那套。亮色下底色巧合同为 #F2F3F5,只文字色不同。
//         彩色系:bg 色板-1,文字 色板-6(yellow 为通式推导补齐——arco Tag
//         预设 13 色不含 yellow,我们合法值收满 14 色板名)
//   样式全挂 view 容器上,text 叶靠 fg/font-size 继承取色
$props {
    label: String,
    color: String = String::from("gray"),   // 14 色板名(gray = 默认形态)
    size: String = String::from("default"), // small | default | medium | large
}
</script>

<view
  class="tag sz-default tag-default"
  class:sz-small={size == "small"}
  class:sz-medium={size == "medium"}
  class:sz-large={size == "large"}
  class:tag-red={color == "red"}
  class:tag-orangered={color == "orangered"}
  class:tag-orange={color == "orange"}
  class:tag-gold={color == "gold"}
  class:tag-yellow={color == "yellow"}
  class:tag-lime={color == "lime"}
  class:tag-green={color == "green"}
  class:tag-cyan={color == "cyan"}
  class:tag-blue={color == "blue"}
  class:tag-arcoblue={color == "arcoblue"}
  class:tag-purple={color == "purple"}
  class:tag-pinkpurple={color == "pinkpurple"}
  class:tag-magenta={color == "magenta"}
>
  <text>{label}</text>
</view>

<style>
/*ARCO_TOKENS*/

/* 圆角 = radius-small(L9);padding 8px/0(L5-L6);垂直居中用 align-items
   (arco 用 line-height = 高-2px,无 line-height,等效降级);非 bordered
   形态的 1px 透明边框(L3/L30)不画。 */
.tag { radius: var(--border-radius-small); padding: 0 8px; align-items: center; justify-content: center; }

/* 高 = size-5/6/7/8(L11-L14),字号 12/12/14/14(L16-L19)。
   注意高度是 --size-5..8(4×N 阶梯),不是 --size-small/default(按钮档)。 */
.sz-small { height: var(--size-5); font-size: var(--font-size-body-1); }
.sz-default { height: var(--size-6); font-size: var(--font-size-body-1); }
.sz-medium { height: var(--size-7); font-size: var(--font-size-body-3); }
.sz-large { height: var(--size-8); font-size: var(--font-size-body-3); }

/* 默认(gray)形态:@tag-default-color-bg/text(L26/L29) */
.tag-default { bg: var(--color-fill-2); fg: var(--color-text-1); }

/* 彩色系:bg = 色板-1,fg = 色板-6(各 @tag-{c}-color-bg/text 行号见 spec) */
.tag-red { bg: var(--red-1); fg: var(--red-6); }
.tag-orangered { bg: var(--orangered-1); fg: var(--orangered-6); }
.tag-orange { bg: var(--orange-1); fg: var(--orange-6); }
.tag-gold { bg: var(--gold-1); fg: var(--gold-6); }
.tag-yellow { bg: var(--yellow-1); fg: var(--yellow-6); }
.tag-lime { bg: var(--lime-1); fg: var(--lime-6); }
.tag-green { bg: var(--green-1); fg: var(--green-6); }
.tag-cyan { bg: var(--cyan-1); fg: var(--cyan-6); }
.tag-blue { bg: var(--blue-1); fg: var(--blue-6); }
.tag-arcoblue { bg: var(--arcoblue-1); fg: var(--arcoblue-6); }
.tag-purple { bg: var(--purple-1); fg: var(--purple-6); }
.tag-pinkpurple { bg: var(--pinkpurple-1); fg: var(--pinkpurple-6); }
.tag-magenta { bg: var(--magenta-1); fg: var(--magenta-6); }
</style>
