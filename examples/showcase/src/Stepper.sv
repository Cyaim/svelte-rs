<script>
$props { value: $bindable(i32), step: i32 = 1 }
</script>

<view style="direction:row; gap:8">
  <button class="btn" onclick={|| value -= step}>-{step}</button>
  <text font-size="20">{value}</text>
  <button class="btn" onclick={|| value += step}>+{step}</button>
</view>

<style>
/* 与 Showcase.sv 的 .btn 同名——scoped,互不影响 */
.btn { padding: 8; radius: 6; bg: #7048e8; fg: #ffffff; }
</style>
