<script>
let name = $state(String::new());
let last_submit = $state(String::new());
let history = $state(Vec::<String>::new());
</script>

<view style="padding:24; gap:12">
  <text font-size="26">文本输入 / IME / 剪贴板 手测台</text>
  <text font-size="13" fg="#666677">中文输入法:候选窗应贴光标;Ctrl+A/C/X/V;Enter 提交;Tab 换焦点</text>

  <input placeholder="在这里输入(支持中文 IME)" bind:value={name}
         onsubmit={|v| {
             if !v.trim().is_empty() {
                 history.update(|h| h.push(v.to_string()));
                 last_submit = v.to_string();
                 name = String::new();
             }
         }} />

  <text>实时值(bind:value):{name}</text>

  <view style="direction:row; gap:8">
    <button style="padding:8; radius:6; bg:#3c78ff; fg:#fff"
            onclick={|| name = "预填文本(外部写入)".to_string()}>外部写入</button>
    <button style="padding:8; radius:6; bg:#ff3e00; fg:#fff"
            onclick={|| name = String::new()}>清空</button>
  </view>

  {#if !last_submit.is_empty()}
    <text fg="#0a7d32">上次提交:{last_submit}</text>
  {/if}

  {#each history as item, i}
    <text font-size="14" fg="#444455">{i + 1}. {item}</text>
  {:else}
    <text font-size="14" fg="#999999">提交历史为空,Enter 试试</text>
  {/each}
</view>
