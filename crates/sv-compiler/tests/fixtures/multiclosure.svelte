<script>
// 非 Copy plain 变量(普通 let),被多个同级块闭包各自引用 —— 修复前每种都 E0382。
// 见 codegen.rs 的 with_captured_plain:每个 move 闭包各得一份外层捕获份。
let label = String::from("hi");
let count = $state(0i32);
let items = $state(vec![String::from("a")]);
</script>

<view>
  <!-- if/else-if/else 三臂都引 label:then/else 是同一 if_block 的同级 move 闭包 -->
  {#if count > 2}
    <text>{label}</text>
  {:else if count > 0}
    <text>{label}</text>
  {:else}
    <text>{label}</text>
  {/if}

  <!-- await 三臂 pending/then/catch 都引 label -->
  {#await ready()}
    <text>{label}</text>
  {:then _v}
    <text>{label}</text>
  {:catch _e}
    <text>{label}</text>
  {/await}

  <!-- each 行体引 label -->
  {#each items as it}
    <text>{it}: {label}</text>
  {/each}
</view>
