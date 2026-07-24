<script>
$props {
    // 非 Copy 的 String prop —— 同时进 if 条件 / await future / key / each list
    // 与各分支/body。修复前每种都 E0382,本 crate 编不过即证明修复回退。
    label: String = String::new(),
}
let items = $state(vec![String::from("a")]);
</script>

<view>
  {#if label.is_empty()}
    <text>{label}</text>
  {:else if label.len() > 3}
    <text>{label}</text>
  {:else}
    <text>{label}</text>
  {/if}

  {#await load(label.clone())}
    <text>{label}</text>
  {:then _v}
    <text>{label}</text>
  {:catch _e}
    <text>{label}</text>
  {/await}

  {#key label.len()}
    <text>{label}</text>
  {/key}

  {#each make(label.clone()) as it}
    <text>{it}: {label}</text>
  {/each}
</view>
