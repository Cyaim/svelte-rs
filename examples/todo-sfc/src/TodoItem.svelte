<script>
$props {
    label: String,
    index: usize,
    on_remove: std::rc::Rc<dyn Fn()>,
    accent: sv_ui::Color = sv_ui::Color::rgb(255, 62, 0),
}

let done = $state(false);
</script>

<view style="direction:row; gap:8">
  <button style="padding:6; radius:6; bg:#ddddea; fg:#222233"
          onclick={|| done = !done}>{if done { "[x]" } else { "[ ]" }}</button>
  <text font-size="18"
        style:fg={if done { sv_ui::Color::rgb(160, 160, 170) } else { sv_ui::Color::BLACK }}
  >{index + 1}. {label}</text>
  <button style="padding:6; radius:6; fg:#fff" style:bg={accent}
          onclick={move || on_remove()}>删除</button>
</view>
{#if done}
  <text font-size="12" fg="#999999">已完成:{label}(勾选可撤销)</text>
{/if}
