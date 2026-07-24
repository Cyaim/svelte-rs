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

  <!-- 元素级:同一非 Copy 的 `label` 喂同级多个 move 闭包(每个 effect/
       bind_style_patch 都按值捕获)。修复前第二个起 E0382,故本段专测元素层。
       站点齐活:value / aria-label / style: / checked / @attach。 -->
  <input value={label} aria-label={label}
         style:width={(label.len() * 8) as f32}
         oninput={|_v: &str| { let _ = label.len(); }} />
  <checkbox checked={!label.is_empty()} aria-label={label} />
  <view aria-label={label}
        onclick={|| { let _ = label.len(); }}
        {@attach |doc: &sv_ui::Doc, id: sv_ui::ViewId| {
            let _keep = label.len();
            let _ = (doc, id);
        }}></view>
</view>
