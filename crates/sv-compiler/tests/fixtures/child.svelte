<script>
// props 三态:必填、带默认、$bindable;外加 snippet 渲染
$props { title: String, step: i32 = 1, value: $bindable(i32), children: sv_ui::Snippet }
</script>

<view class="row">
  <text>{title}</text>
  <button class="btn" onclick={|| value += step}>+{step}</button>
  {@render children()}
</view>

<style>
.row { direction: row; gap: 8; }
.btn { padding: 6; radius: 4; bg: #7048e8; fg: #ffffff; }
</style>
