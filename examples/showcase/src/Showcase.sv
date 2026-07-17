<script>
let count = $state(0i32);
let double = $derived(count * 2);
let agree = $state(false);
let hovers = $state(0i32);
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

  <Card title={String::from("异步 · 过渡 · 表单 · 悬停")}>
    <view class="row">
      <checkbox bind:checked={agree} />
      <text>同意协议(bind:checked 双向)</text>
    </view>
    {#if agree}
      <text in:fade={300} fg="#1a7f37">已同意 —— 这行带 300ms 淡入</text>
    {/if}
    {#await async move { let base = 6 * 7; base }}
      <text class="muted">后台计算中…</text>
    {:then v}
      <text>异步答案:{v}</text>
    {/await}
    <text class="muted" onpointerenter={|| hovers += 1}>悬停过 {hovers} 次的区域</text>
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
/* CSS 兼容层演示:标准属性名、px 单位、rgb()/颜色名、:hover 伪类 */
.page { padding: 24px; gap: 12px; }
.h1 { font-size: 28px; }
.row { flex-direction: row; gap: 8px; }
.muted { color: #666677; }
.tiny { font-size: 12px; color: #9999aa; }
.btn { padding: 8px; border-radius: 6px; background-color: rgb(255, 62, 0); color: white; }
.btn:hover { background-color: orange; }
.btn-alt { padding: 8px; border-radius: 6px; background-color: #3c78ff; color: white; }
.btn-alt:hover { opacity: 0.8; }
</style>
