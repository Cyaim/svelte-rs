<script>
$props {
    open: $bindable(bool),
    title: String,
}
</script>

<view>
  <overlay open={open} anchor="center" modal ondismiss={|| open = false}
           style="padding:16; gap:12; bg:#ffffff; border:2px solid #d0d0dd; radius:10; min-width: 260">
    <text font-size="20">{title}</text>
    <text fg="#666677">这是一个模态对话框:底层不可点,Tab 焦点被困在框内。</text>
    <view style="direction:row; gap:8; justify-content: end">
      <button style="padding:8; radius:6; bg:#ddddea; fg:#222233"
              onclick={|| open = false}>取消</button>
      <button style="padding:8; radius:6; bg:#ff3e00; fg:#fff"
              onclick={|| open = false}>确定</button>
    </view>
  </overlay>
</view>
