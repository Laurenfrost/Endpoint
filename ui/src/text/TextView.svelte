<script>
  // 文本区:VirtualText(主显示)+ OverviewRuler(右缘标尺)。
  // 始终展示原始 decoded source,各阶段通过 annotations store 叠加不同高亮层。
  import { pipeline } from "../stores/pipeline.svelte.js";
  import VirtualText from "./VirtualText.svelte";
  import OverviewRuler from "./OverviewRuler.svelte";
</script>

<section class="text-view">
  {#if !pipeline.dto}
    <div class="empty">
      <p>未加载文件。</p>
      <p class="small">在左侧「文本选择」阶段选择 txt 并加载。</p>
    </div>
  {:else}
    <VirtualText />
    <OverviewRuler />
  {/if}
</section>

<style>
  .text-view {
    flex: 1;
    display: flex;
    min-height: 0;
    background: #fff;
  }
  .empty {
    flex: 1;
    color: #52606d;
    padding: 40px;
    text-align: center;
    align-self: center;
  }
  .empty .small { font-size: 12px; opacity: 0.7; }
</style>
