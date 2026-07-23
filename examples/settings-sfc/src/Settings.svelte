<script>
let username = $state(String::from("psyche"));
let notify = $state(true);
let telemetry = $state(false);
let volume = $state(60i32);
let scroll_y = $state(0.0f32);
</script>

<view style="padding:16; gap:10; height: 380">
  <view style="direction:row; justify-content: space-between; align-items: center">
    <text font-size="24">设置(R2 验收:滚动 + flex + 折行)</text>
    <text font-size="12" fg="#999999">滚动位置 {format!("{:.0}", scroll_y)}px</text>
  </view>

  <view style="overflow: scroll; height: 320; gap:14; padding: 4" bind:scrolly={scroll_y}>
    <view style="gap:6">
      <text font-size="18">账户</text>
      <view style="direction:row; align-items: center; gap:8">
        <text style="min-width: 80">用户名</text>
        <input placeholder="输入用户名" bind:value={username} />
      </view>
      <text font-size="13" fg="#666677">当前登录:{username}。这一段说明文字特意写得比较长,用来验收调研 23 的文本折行——容器宽度之内应当按词与 CJK 规则断行,标点不落行首,超长的不可断内容也会被安全地强制折断而不是撑破布局。</text>
    </view>

    <view style="gap:6">
      <text font-size="18">通知</text>
      <view style="direction:row; align-items: center; gap:8">
        <checkbox bind:checked={notify} />
        <text>接收系统通知</text>
      </view>
      <view style="direction:row; align-items: center; gap:8">
        <checkbox bind:checked={telemetry} />
        <text>发送匿名使用统计</text>
      </view>
      {#if telemetry}
        <text font-size="12" fg="#0a7d32">感谢!统计数据仅用于改进产品。</text>
      {/if}
    </view>

    <view style="gap:6">
      <text font-size="18">音量:{volume}%</text>
      <view style="direction:row; gap:8">
        <button style="padding:6; radius:6; bg:#3c78ff; fg:#fff" onclick={|| volume = (volume - 10).max(0)}>-10</button>
        <button style="padding:6; radius:6; bg:#3c78ff; fg:#fff" onclick={|| volume = (volume + 10).min(100)}>+10</button>
      </view>
    </view>

    <view style="gap:6">
      <text font-size="18">关于</text>
      <text font-size="13" fg="#666677" style="max-width: 420">sv 是一个 Svelte 风格的 Rust 跨平台桌面 UI 库探索原型。渲染层当前为 CPU/vello 双后端,布局引擎为 taffy 0.12,文本栈为 Parley 0.11 + fontique(shaping/折行/光标几何一并接管)。本面板内容特意超出一屏高度:用滚轮滚动(鼠标滚轮带缓动)、或直接拖右侧那根合成绘制的滚动条。</text>
      <text font-size="13" fg="#666677">空间占位段落甲:为了让内容确定超过一屏,这里再放几段文字。</text>
      <text font-size="13" fg="#666677">空间占位段落乙:滚动到底部时,滚动链会把多余的滚动量交给祖先容器。</text>
      <text font-size="13" fg="#666677">空间占位段落丙:Tab 键可以在输入框、复选框与按钮之间移动焦点。</text>
      <text font-size="13" fg="#666677">底部:你看到这行说明滚动生效了。</text>
    </view>
  </view>
</view>
