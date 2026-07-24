<script>
// 非 Copy plain 变量(普通 let)同时进**引导闭包**(if 条件 / await future / key)
// 与**分支/body**闭包 —— 修复前每种都 E0382。见 codegen.rs 的 with_captured_plain:
// 每个同级 move 闭包(含 cond/fut/key 驱动)各得一份外层捕获份。
let label = String::from("hi");
let items = $state(vec![String::from("a")]);
</script>

<view>
  <!-- cond 与 then/else 都引 label:三者是 if_block 的同级 move 闭包 -->
  {#if label.is_empty()}
    <text>{label}</text>
  {:else if label.len() > 3}
    <text>{label}</text>
  {:else}
    <text>{label}</text>
  {/if}

  <!-- future 与 pending/then/catch 都引 label -->
  {#await load(label.clone())}
    <text>{label}</text>
  {:then _v}
    <text>{label}</text>
  {:catch _e}
    <text>{label}</text>
  {/await}

  <!-- key 与 body 都引 label -->
  {#key label.len()}
    <text>{label}</text>
  {/key}

  <!-- each 的 list 与行体都引 label(list_cl + row 各捕获份) -->
  {#each make(label.clone()) as it}
    <text>{it}: {label}</text>
  {/each}
</view>
