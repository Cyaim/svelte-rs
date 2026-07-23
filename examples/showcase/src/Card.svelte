<script>
$props { title: String, children: sv_ui::Snippet }
</script>

<view class="card">
  <text class="card-title">{title}</text>
  {@render children()}
</view>

<style>
.card { padding: 16; gap: 8; radius: 10; bg: #f0f0f6; }
.card-title { font-size: 20; fg: #223344; }
</style>
