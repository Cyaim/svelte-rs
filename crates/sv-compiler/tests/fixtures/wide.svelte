<script>
// 尽量把 codegen 的形状面铺开:runes 三件套、普通变量(预克隆)、
// 事件、双向绑定、块结构、样式四态。**这份 fixture 只服务金样,
// 不追求好看的 UI**——改它等于宣布"codegen 输出该变了"。
let count = $state(0i32);
let name = $state(String::from("世界"));
let double = $derived(count * 2);
let items = $state(vec![(1i32, String::from("甲")), (2, String::from("乙"))]);
let agree = $state(false);
let note = $state(String::new());
let scroll_y = $state(0f32);
let label = String::from("普通变量,会被预克隆");
$effect(|| {
    let _ = count;
});
</script>

<view class="page" style="gap: 8">
  <text font-size="26">你好 {name},计数 {count},双倍 {double}</text>

  <button class="btn" onclick={|| count += 1}>加一</button>
  <button class="btn" onclick={|| count = 0}
          onkeydown={|e| { let _ = e; }}
          onkeyup={|e| { let _ = e; }}
          aria-label={format!("重置(当前 {count})")}>重置</button>

  <view style="direction: row; gap: 4">
    <checkbox bind:checked={agree} />
    <text>{label}</text>
  </view>

  <input placeholder="单行" bind:value={name} oninput={|v| { let _ = v; }}
         onsubmit={|v| { let _ = v; }} />
  <textarea rows="3" placeholder="多行" bind:value={note} />

  {#if count > 3}
    <text fg="#ff3e00">超过 3 了</text>
  {:else if count > 1}
    <text>刚过 1</text>
  {:else}
    <text>还早</text>
  {/if}

  {#each items as it (it.0)}
    <text>{it.1}</text>
  {/each}

  {#each items as pair, i}
    <text>{i}: {pair.1}</text>
  {:else}
    <text>空列表</text>
  {/each}

  {#key count}
    <text>随 count 重建</text>
  {/key}

  <view style="overflow-y: scroll; overflow-x: hidden; height: 60"
        bind:scrolly={scroll_y} onscroll={|x, y| { let _ = (x, y); }}>
    <text>可滚内容</text>
  </view>

  {#await async move { 42i32 }}
    <text>算着呢</text>
  {:then v}
    <text>算完了 {v}</text>
  {/await}

  <view transition:fade={200}>
    <text>淡入</text>
  </view>
</view>

<style>
.page {
  padding: 16;
  background: #ffffff;
}
.btn {
  padding: 8;
  border-radius: 6;
  background: #3c78ff;
  color: #ffffff;
  &:hover  { background: #2f66e6; }
  &:active { opacity: 0.8; }
  &:focus  { border: 2 solid #0a2f8f; }
}
text {
  color: #223344;
}
</style>
