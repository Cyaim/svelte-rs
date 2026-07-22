<script>
let name = $state(String::new());
let last_submit = $state(String::new());
let history = $state(Vec::<String>::new());
let note = $state(String::from("第一行\n第二行(Enter 换行)"));
</script>

<view style="padding:24; gap:12">
  <text font-size="26">文本输入 / IME / 剪贴板 手测台</text>
  <text font-size="13" fg="#666677">中文输入法:候选窗应贴光标;Ctrl+A/C/X/V;Enter 提交;Tab 换焦点</text>
  <text font-size="13" fg="#666677">编辑:Ctrl+←/→ 词跳;Ctrl+Backspace 删词;Ctrl+Z / Ctrl+Y 撤销重做</text>
  <text font-size="13" fg="#666677">指针:拖拽选择;双击选词;三击全选</text>

  <input placeholder="在这里输入(支持中文 IME)" bind:value={name}
         onsubmit={|v| {
             if !v.trim().is_empty() {
                 history.update(|h| h.push(v.to_string()));
                 last_submit = v.to_string();
                 name = String::new();
             }
         }} />

  <text>实时值(bind:value):{name}</text>

  <text font-size="13" fg="#666677">多行 textarea:Enter 换行,↑/↓ 按视觉行走,超出 rows 自动上滚</text>
  <textarea rows="4" placeholder="多行输入…" bind:value={note} />
  <text font-size="13" fg="#999999">note 长度:{note.chars().count()}</text>

  <view style="direction:row; gap:8">
    <button class="act" onclick={|| name = "预填文本(外部写入)".to_string()}>外部写入</button>
    <button class="danger" onclick={|| name = String::new()}>清空</button>
  </view>
  <text font-size="13" fg="#666677">上面两个按钮带 :focus 样式 —— Tab 过去看边框(键盘用户的位置反馈)</text>

  {#if !last_submit.is_empty()}
    <text fg="#0a7d32">上次提交:{last_submit}</text>
  {/if}

  {#each history as item, i}
    <text font-size="14" fg="#444455">{i + 1}. {item}</text>
  {:else}
    <text font-size="14" fg="#999999">提交历史为空,Enter 试试</text>
  {/each}
</view>

<style>
.act {
  padding: 8;
  border-radius: 6;
  background: #3c78ff;
  color: #fff;
  &:hover { background: #2f66e6; }
  &:focus { border: 2 solid #0a2f8f; }
}
.danger {
  padding: 8;
  border-radius: 6;
  background: #ff3e00;
  color: #fff;
  &:hover { background: #e63700; }
  &:focus { border: 2 solid #7a1d00; }
}
</style>
