<script>
  // VS Code 式布局:活动栏 + 侧边栏 + 文本区,底部状态栏。
  // 窗口标题由 Windows 原生标题栏承载(tauri.conf.json 的 title 字段)。
  import { onMount } from "svelte";
  import ActivityBar from "./layout/ActivityBar.svelte";
  import Sidebar from "./layout/Sidebar.svelte";
  import StatusBar from "./layout/StatusBar.svelte";
  import TextView from "./text/TextView.svelte";
  import { applyProgressEvent } from "./stores/progress.svelte.js";
  import { onProgress } from "./ipc.js";

  // 进度事件全局订阅一次,贯穿整个应用生命周期。
  let unlisten;
  onMount(() => {
    onProgress(applyProgressEvent).then((u) => (unlisten = u));
    return () => unlisten?.();
  });
</script>

<div class="shell">
  <div class="body">
    <ActivityBar />
    <Sidebar />
    <TextView />
  </div>
  <StatusBar />
</div>

<style>
  :global(html, body) {
    margin: 0;
    padding: 0;
    height: 100%;
    overflow: hidden;
    font-family: "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif;
    color: #1f2933;
    background: #f5f7fa;
  }
  :global(*) { box-sizing: border-box; }
  :global(#app) { height: 100%; }

  .shell {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
</style>
