<script>
let count = $state(0i32);
let double = $derived(count * 2);
let items = $state(vec![
    (1i32, String::from("学 Rust")),
    (2, String::from("写 UI 库")),
    (3, String::from("发布")),
]);
let next_id = $state(4i32);

$inspect(count);
</script>

<view class="page">
  <text class="h1">sv 特性橱窗</text>

  {#snippet stat(label: String, n: i32)}
    <view class="row">
      <text class="muted">{label}</text>
      <text>{n}</text>
    </view>
  {/snippet}

  <Card title={String::from("双向绑定 $bindable + snippet")}>
    <Stepper bind:value={count} step={2} />
    {@render stat(String::from("外部读同一信号:"), count)}
    {@render stat(String::from("$derived 双倍:"), double)}
    {#if count > 5}
      <text fg="#ff3e00">超过 5 了!</text>
    {/if}
  </Card>

  <Card title={String::from("keyed 列表(重排保状态)")}>
    <view class="row">
      <button class="btn" onclick={|| {
          items.update(|v| v.push((next_id.get_untracked(), format!("任务 {}", next_id))));
          next_id += 1;
      }}>添加</button>
      <button class="btn-alt" onclick={|| items.update(|v| v.reverse())}>反转顺序</button>
    </view>
    {#if items.is_empty()}
      <text class="muted">没有任务了</text>
    {/if}
    {#each items as it (it.0)}
      <TaskRow id={it.0} label={it.1}
               on_remove={std::rc::Rc::new(move || items.update(|v| v.retain(|x| x.0 != it.0)))} />
    {/each}
  </Card>

  <Card title={String::from("{@const} · {#key} · {@debug}")}>
    {@const summary = format!("{} 项任务 · 计数 {}", items.len(), count)}
    <text class="muted">{summary}</text>
    {#key count}
      <text class="tiny">count 变化时我销毁重建(key 块)</text>
    {/key}
    {@debug count, double}
  </Card>
</view>

<style>
.page { padding: 24; gap: 12; }
.h1 { font-size: 28; }
.row { direction: row; gap: 8; }
.muted { fg: #666677; }
.tiny { font-size: 12; fg: #9999aa; }
.btn { padding: 8; radius: 6; bg: #ff3e00; fg: #ffffff; }
.btn-alt { padding: 8; radius: 6; bg: #3c78ff; fg: #ffffff; }
</style>
