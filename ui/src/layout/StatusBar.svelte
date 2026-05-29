<script>
  // VS Code 风格底部状态栏。
  // 左:文本统计(编码 / 字符数 / 清洗 / 顶层条目)
  // 右:阶段标签 + 进度条 + 百分比 + 详情
  import { pipeline } from "../stores/pipeline.svelte.js";
  import { progress, stageLabel } from "../stores/progress.svelte.js";
</script>

<footer class="status-bar" aria-label="状态栏">
  <div class="left">
    <span class="brand">Endpoint</span>
    {#if pipeline.dto}
      <span class="sep">·</span>
      <span class="item" title="编码">
        <span class="dot enc"></span>{pipeline.dto.source_encoding}
      </span>
      <span class="item" title="字符数">
        {pipeline.dto.source_text.length.toLocaleString()} 字符
      </span>
      <span class="item" title="清洗标注">
        {pipeline.dto.cleaning.length} 清洗
      </span>
      <span class="item" title="顶层条目">
        {pipeline.dto.book.entries.length} 条目
      </span>
    {:else}
      <span class="sep">·</span>
      <span class="item idle">未加载文件</span>
    {/if}
  </div>

  <div class="right">
    {#if progress.stage}
      <span class="stage">{stageLabel(progress.stage)}</span>
      <span class="bar" class:busy={progress.busy}>
        <span class="fill" style="width: {progress.percent}%"></span>
      </span>
      <span class="pct">{progress.percent}%</span>
      {#if progress.detail}
        <span class="detail" title={progress.detail}>{progress.detail}</span>
      {/if}
    {:else}
      <span class="idle">就绪</span>
    {/if}
  </div>
</footer>

<style>
  .status-bar {
    height: 24px;
    background: #1f6feb;
    color: #fff;
    display: flex;
    align-items: center;
    padding: 0 10px;
    gap: 14px;
    font-size: 11px;
    flex-shrink: 0;
    border-top: 1px solid #1858c4;
    user-select: none;
  }
  .left, .right {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .right {
    margin-left: auto;
    flex-shrink: 0;
  }
  .brand {
    font-weight: 600;
    letter-spacing: 0.3px;
  }
  .sep {
    color: rgba(255, 255, 255, 0.4);
  }
  .item {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    white-space: nowrap;
  }
  .item.idle {
    color: rgba(255, 255, 255, 0.7);
  }
  .dot.enc {
    display: inline-block;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #fff;
    opacity: 0.85;
  }
  .stage {
    color: #fff;
    font-weight: 500;
  }
  .bar {
    display: inline-block;
    width: 120px;
    height: 6px;
    background: rgba(255, 255, 255, 0.25);
    border-radius: 3px;
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    background: #fff;
    transition: width 120ms ease;
  }
  .pct {
    font-family: Consolas, "Cascadia Mono", monospace;
    min-width: 36px;
    text-align: right;
  }
  .detail {
    color: rgba(255, 255, 255, 0.85);
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .idle {
    color: rgba(255, 255, 255, 0.75);
    font-style: italic;
  }
</style>
