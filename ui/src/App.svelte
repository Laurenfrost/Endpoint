<script>
  // VS Code 式三栏:活动栏(56px) + 侧边栏(320px) + 文本区(剩余空间)。
  // 顶栏挂结构化进度条;切换阶段时只换侧边栏内容与高亮色层,骨架不动。
  import { onMount } from "svelte";
  import ActivityBar from "./layout/ActivityBar.svelte";
  import Sidebar from "./layout/Sidebar.svelte";
  import TextView from "./text/TextView.svelte";
  import { progress, stageLabel, applyProgressEvent } from "./stores/progress.svelte.js";
  import { pipeline } from "./stores/pipeline.svelte.js";
  import { onProgress } from "./ipc.js";

  // 进度事件全局订阅一次,贯穿整个应用生命周期。
  let unlisten;
  onMount(() => {
    onProgress(applyProgressEvent).then((u) => (unlisten = u));
    return () => unlisten?.();
  });
</script>

<div class="shell">
  <header class="title-bar">
    <span class="brand">Endpoint <small>· 网文 txt → EPUB</small></span>
    {#if pipeline.dto}
      <span class="meta">
        {pipeline.dto.source_encoding} ·
        {pipeline.dto.source_text.length.toLocaleString()} chars ·
        {pipeline.dto.cleaning.length} 清洗 ·
        {pipeline.dto.book.entries.length} 顶层条目
      </span>
    {/if}
    <span class="progress" class:active={progress.busy || progress.percent > 0}>
      {#if progress.stage}
        <span class="stage">{stageLabel(progress.stage)}</span>
        <span class="bar">
          <span class="fill" style="width: {progress.percent}%"></span>
        </span>
        <span class="pct">{progress.percent}%</span>
        {#if progress.detail}<span class="detail">{progress.detail}</span>{/if}
      {:else}
        <span class="idle">就绪</span>
      {/if}
    </span>
  </header>

  <div class="body">
    <ActivityBar />
    <Sidebar />
    <TextView />
  </div>
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
  .title-bar {
    height: 32px;
    background: #1f2933;
    color: #cbd2d9;
    display: flex;
    align-items: center;
    padding: 0 12px;
    gap: 16px;
    font-size: 11px;
    flex-shrink: 0;
  }
  .brand { font-weight: 600; color: #fff; }
  .brand small { color: #9aa5b1; font-weight: normal; margin-left: 4px; }
  .meta {
    color: #9aa5b1;
    font-family: Consolas, "Cascadia Mono", monospace;
    font-size: 10px;
  }
  .progress {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 10px;
  }
  .progress .idle { color: #6c7682; }
  .progress .stage { color: #fff; }
  .progress .bar {
    display: inline-block;
    width: 120px;
    height: 6px;
    background: #3a4452;
    border-radius: 3px;
    overflow: hidden;
  }
  .progress .fill {
    display: block;
    height: 100%;
    background: #1f6feb;
    transition: width 120ms ease;
  }
  .progress .pct {
    font-family: Consolas, "Cascadia Mono", monospace;
    color: #fff;
    min-width: 32px;
    text-align: right;
  }
  .progress .detail { color: #9aa5b1; }

  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
</style>
