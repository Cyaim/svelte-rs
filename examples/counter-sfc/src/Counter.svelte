<script>
let count = $state(0i32);
let double = $derived(count * 2);

$effect(|| {
    println!("count 变为 {}", count);
});
</script>

<view style="padding:24; gap:12">
  <text font-size="28">sv 计数器(.sv 编译器路线)</text>
  <text font-size="20">Count: {count} · 双倍 = {double}</text>
  <view style="direction:row; gap:8">
    <button style="padding:10; radius:8; bg:#ff3e00; fg:#fff" onclick={|| count += 1}>+1</button>
    <button style="padding:10; radius:8; bg:#ff3e00; fg:#fff" onclick={|| count -= 1}>-1</button>
    <button style="padding:10; radius:8; bg:#ff3e00; fg:#fff" onclick={|| count = 0}>归零</button>
  </view>
  {#if count > 5}
    <text fg="#ff3e00">超过 5 了!</text>
  {:else if count < 0}
    <text fg="#3c78ff">负数啦</text>
  {/if}
</view>
