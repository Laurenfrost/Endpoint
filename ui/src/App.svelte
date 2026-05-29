<script>
  // VS Code 式布局:活动栏 + 侧边栏 + 文本区,底部状态栏。
  // 窗口标题由 Windows 原生标题栏承载(tauri.conf.json 的 title 字段)。
  import { onMount } from "svelte";
  import { ModeWatcher } from "mode-watcher";
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

<ModeWatcher />

<div class="flex h-full flex-col bg-background text-foreground">
  <div class="flex min-h-0 flex-1">
    <ActivityBar />
    <Sidebar />
    <TextView />
  </div>
  <StatusBar />
</div>
