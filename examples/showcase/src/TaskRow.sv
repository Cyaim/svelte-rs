<script>
$props { id: i32, label: String, on_remove: std::rc::Rc<dyn Fn()> }

let done = $state(false);
</script>

<view style="direction:row; gap:8">
  <button class="chk" onclick={|| done = !done}>{if done { "[x]" } else { "[ ]" }}</button>
  <text font-size="18"
        style:fg={if done { sv_ui::Color::rgb(160, 160, 170) } else { sv_ui::Color::BLACK }}
  >#{id} {label}</text>
  <button class="del" onclick={move || on_remove()}>删</button>
</view>

<style>
.chk { padding: 6; radius: 6; bg: #ddddea; fg: #223344; }
.del { padding: 6; radius: 6; bg: #ff3e00; fg: #ffffff; }
</style>
