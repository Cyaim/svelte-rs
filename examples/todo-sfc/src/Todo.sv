<script>
let todos = $state(Vec::<String>::new());
let next = $state(1i32);
let count = $derived(todos.len());

$inspect(count);
</script>

<view style="padding:24; gap:10">
  <text font-size="26">待办(.sv 特性演示)</text>
  {@const summary = format!("共 {} 项", count)}
  <text fg="#666677">{summary}</text>

  <view style="direction:row; gap:8">
    <button style="padding:8; radius:6; bg:#ff3e00; fg:#fff" onclick={|| {
        todos.update(|v| v.push(format!("事项 {}", next)));
        next += 1;
    }}>添加</button>
    <button style="padding:8; radius:6; bg:#3c78ff; fg:#fff" onclick={|| todos = Vec::new()}>清空</button>
  </view>

  {#each todos as label, i}
    <TodoItem
      label={label}
      index={i}
      on_remove={std::rc::Rc::new(move || todos.update(|v| { v.remove(i); }))}
    />
  {:else}
    <text fg="#999999">空空如也,点「添加」</text>
  {/each}

  {#key count}
    <text font-size="12" fg="#bbbbcc">数量变化时这行会销毁重建(key 块演示)</text>
  {/key}
</view>
