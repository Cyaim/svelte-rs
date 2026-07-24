<script>
// Arco Badge(A1 降级版:只做 standalone 两态——count 胶囊 + dot 圆点;
// 不做绝对定位角标,overlay 锚定是 R3 域。非交互:无伪类/无 disabled/无回调)。
//
// 组件级取值出处:assets/badge-token.less(@arco-design/web-react 2.66.16):
//   胶囊 高 = size-5(20px,L3),横 padding = spacing-3(6px,L4),
//        字号 = body-1(12px,L7),白字(L9),默认底 = danger-6(L11);
//        min-width = 高(单字符呈正圆,arco badge.less 惯例),radius = 高/2
//   圆点 6×6(L12),radius = 3
//   彩色:@badge-{c}-color-dot-bg(L19-L30):red→danger-6、green→success-6、
//        arcoblue→primary-6、gray→gray-4(注意第 4 档),其余 色板-6
//   count 数字 font-weight 500(L18)单字重表达不了;count<=0 且非 dot 不渲染
$props {
    count: i64 = 0,
    max_count: i64 = 99,
    dot: bool = false,
    color: String = String::from("danger"), // danger(默认红)| 12 彩色/灰,见类表
}
let display = if count > max_count { format!("{max_count}+") } else { count.to_string() };
// dot 分支用独立副本:同一 plain 变量进多个同级块闭包会 move 冲突
// (sv-compiler 预克隆只覆盖单闭包,已登记 open-issues),每分支一份绕开
let color_dot = color.clone();
</script>

{#if dot}
  <view class="badge-dot"
    class:bg-red={color_dot == "red"}
    class:bg-orangered={color_dot == "orangered"}
    class:bg-orange={color_dot == "orange"}
    class:bg-gold={color_dot == "gold"}
    class:bg-lime={color_dot == "lime"}
    class:bg-green={color_dot == "green"}
    class:bg-cyan={color_dot == "cyan"}
    class:bg-arcoblue={color_dot == "arcoblue"}
    class:bg-purple={color_dot == "purple"}
    class:bg-pinkpurple={color_dot == "pinkpurple"}
    class:bg-magenta={color_dot == "magenta"}
    class:bg-gray={color_dot == "gray"}
  />
{:else if count > 0}
  <view class="badge-count"
    class:bg-red={color == "red"}
    class:bg-orangered={color == "orangered"}
    class:bg-orange={color == "orange"}
    class:bg-gold={color == "gold"}
    class:bg-lime={color == "lime"}
    class:bg-green={color == "green"}
    class:bg-cyan={color == "cyan"}
    class:bg-arcoblue={color == "arcoblue"}
    class:bg-purple={color == "purple"}
    class:bg-pinkpurple={color == "pinkpurple"}
    class:bg-magenta={color == "magenta"}
    class:bg-gray={color == "gray"}
  >
    <text class="badge-count-text">{display}</text>
  </view>
{/if}

<style>
/*ARCO_TOKENS*/

/* 胶囊:高/最小宽 20(L3),横 padding 6px(L4),radius = 高/2 正圆端,
   默认底 danger-6(L11);垂直水平双轴居中替代 arco 的 line-height 居中。 */
.badge-count {
  height: var(--size-5);
  min-width: var(--size-5);
  padding: 0 6px;
  radius: 10px;
  bg: var(--danger-6);
  direction: row;
  align-items: center;
  justify-content: center;
}

/* count 文本:白字(L9),12px(L7) */
.badge-count-text { fg: var(--color-white); font-size: var(--font-size-body-1); }

/* 圆点:6×6(L12),radius = 3 */
.badge-dot { width: 6px; height: 6px; radius: 3px; bg: var(--danger-6); }

/* 条件色类(dot / count 共用,只覆盖 bg;danger 为静态默认无独立类)。
   red 与默认同值(L19 把 red 映射到 danger-6);gray 用第 4 档(L30)。 */
.bg-red { bg: var(--danger-6); }
.bg-orangered { bg: var(--orangered-6); }
.bg-orange { bg: var(--orange-6); }
.bg-gold { bg: var(--gold-6); }
.bg-lime { bg: var(--lime-6); }
.bg-green { bg: var(--success-6); }
.bg-cyan { bg: var(--cyan-6); }
.bg-arcoblue { bg: var(--primary-6); }
.bg-purple { bg: var(--purple-6); }
.bg-pinkpurple { bg: var(--pinkpurple-6); }
.bg-magenta { bg: var(--magenta-6); }
.bg-gray { bg: var(--gray-4); }
</style>
