<script>
let dialog_open = $state(false);
let menu_open = $state(false);
let picked = $state(String::from("(未选)"));
</script>

<view style="padding:20; gap:14">
  <text font-size="24">弹层演示:对话框 / 下拉菜单 / tooltip</text>

  <view style="direction:row; gap:10">
    <button style="padding:8; radius:6; bg:#ff3e00; fg:#fff"
            onclick={|| dialog_open = true}>打开对话框</button>

    <view>
      <button style="padding:8; radius:6; bg:#3c78ff; fg:#fff"
              onclick={|| menu_open = !menu_open}>下拉菜单</button>
      <overlay open={menu_open} anchor="below" gap="4"
               ondismiss={|| menu_open = false}
               style="padding:4; gap:2; bg:#ffffff; border:1px solid #d0d0dd; radius:8; min-width: 140">
        <button style="padding:6; radius:4; bg:#ffffff; fg:#222233"
                onclick={|| { picked = "新建".to_string(); menu_open = false; }}>新建</button>
        <button style="padding:6; radius:4; bg:#ffffff; fg:#222233"
                onclick={|| { picked = "打开".to_string(); menu_open = false; }}>打开</button>
        <button style="padding:6; radius:4; bg:#ffffff; fg:#222233"
                onclick={|| { picked = "保存".to_string(); menu_open = false; }}>保存</button>
      </overlay>
    </view>

    <button style="padding:8; radius:6; bg:#ddddea; fg:#222233"
            {@attach |doc: &sv_ui::Doc, id: sv_ui::ViewId| {
                sv_ui::tooltip(doc, id, 400, |d, root| {
                    d.update_style(root, |s| {
                        s.padding = 6.0.into();
                        s.bg = Some(sv_ui::Color::rgb(40, 40, 48));
                        s.corner_radius = 6.0;
                    });
                    let t = d.create_text("悬停 400ms 出现的提示");
                    d.append(root, t);
                    d.update_style(t, |s| s.fg = Some(sv_ui::Color::WHITE));
                });
            }}>悬停看提示</button>
  </view>

  <text>菜单选择:{picked}</text>
  <Dialog bind:open={dialog_open} title={"确认操作".to_string()} />
</view>
