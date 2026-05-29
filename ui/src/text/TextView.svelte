<script>
  // 文本区:VirtualText(主显示)+ OverviewRuler(右缘标尺)。
  // 始终展示原始 decoded source,各阶段通过 annotations store 叠加不同高亮层。
  import { pipeline } from "../stores/pipeline.svelte.js";
  import VirtualText from "./VirtualText.svelte";
  import OverviewRuler from "./OverviewRuler.svelte";
</script>

<section class="flex min-h-0 flex-1 bg-background">
  {#if !pipeline.dto}
    <div class="flex-1 self-center p-10 text-center text-muted-foreground">
      <p>未加载文件。</p>
      <p class="text-xs opacity-70">在左侧「文本选择」阶段选择 txt 并加载。</p>
    </div>
  {:else}
    <VirtualText />
    <OverviewRuler />
  {/if}
</section>
