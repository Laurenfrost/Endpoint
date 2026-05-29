<script>
  // VS Code 风格底部状态栏。
  // 左:文本统计(编码 / 字符数 / 清洗 / 顶层条目)
  // 右:阶段标签 + 进度条 + 百分比 + 详情
  import { pipeline } from "../stores/pipeline.svelte.js";
  import { progress, stageLabel } from "../stores/progress.svelte.js";
</script>

<footer
  class="flex h-6 shrink-0 select-none items-center gap-3.5 border-t bg-statusbar px-2.5 text-[11px] text-statusbar-foreground"
  aria-label="状态栏"
>
  <div class="flex min-w-0 items-center gap-2">
    <span class="font-semibold tracking-wide">Endpoint</span>
    {#if pipeline.dto}
      <span class="opacity-40">·</span>
      <span class="inline-flex items-center gap-1 whitespace-nowrap" title="编码">
        <span class="inline-block size-1.5 rounded-full bg-current opacity-85"></span>
        {pipeline.dto.source_encoding}
      </span>
      <span class="whitespace-nowrap" title="字符数">
        {pipeline.dto.source_text.length.toLocaleString()} 字符
      </span>
      <span class="whitespace-nowrap" title="清洗标注">
        {pipeline.dto.cleaning.length} 清洗
      </span>
      <span class="whitespace-nowrap" title="顶层条目">
        {pipeline.dto.book.entries.length} 条目
      </span>
    {:else}
      <span class="opacity-40">·</span>
      <span class="italic opacity-75">未加载文件</span>
    {/if}
  </div>

  <div class="ml-auto flex shrink-0 items-center gap-2">
    {#if progress.stage}
      <span class="font-medium">{stageLabel(progress.stage)}</span>
      <span class="inline-block h-1.5 w-[120px] overflow-hidden rounded-full bg-white/25">
        <span
          class="block h-full bg-current transition-[width] duration-150"
          style="width: {progress.percent}%"
        ></span>
      </span>
      <span class="min-w-9 text-right font-mono">{progress.percent}%</span>
      {#if progress.detail}
        <span class="max-w-[280px] truncate opacity-85" title={progress.detail}>{progress.detail}</span>
      {/if}
    {:else}
      <span class="italic opacity-75">就绪</span>
    {/if}
  </div>
</footer>
