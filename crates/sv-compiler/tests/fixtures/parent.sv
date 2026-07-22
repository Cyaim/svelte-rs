<script>
let n = $state(3i32);
let title = String::from("子组件");
</script>

<view style="gap: 6">
  <!-- 组件调用:必填 prop、省略带默认的 prop、bind: 双向、隐式 children snippet -->
  <Child title={title.clone()} bind:value={n}>
    <text>这是塞进去的 children</text>
  </Child>

  {#snippet extra(x: i32)}
    <text>片段参数 {x}</text>
  {/snippet}

  {@render extra(n)}
</view>
