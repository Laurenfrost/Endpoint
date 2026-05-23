<script>
  // 阶段 2:文本处理 —— 清洗标注的红色高亮 + 侧边栏列表 + 跳转。
  // 黄色"灰区"列表保持空占位,由阶段三的水印检测填充。
  import { pipeline } from "../stores/pipeline.svelte.js";
  import {
    setLayers,
    clearLayers,
    jumpToByteOffset,
  } from "../stores/annotations.svelte.js";
  import { onMount, onDestroy } from "svelte";

  const KIND_LABEL = {
    blank_line_compression: "空行压缩",
    fullwidth_space: "全角空格",
    control_char: "控制字符",
    trailing_whitespace: "行尾空白",
  };

  const items = $derived(pipeline.dto?.cleaning ?? []);
  const counts = $derived.by(() => {
    const m = {};
    for (const c of items) m[c.kind] = (m[c.kind] ?? 0) + 1;
    return m;
  });

  // 把清洗标注注入 annotations store 作为红色层。
  $effect(() => {
    if (!pipeline.dto) {
      clearLayers();
      return;
    }
    setLayers([
      {
        id: "cleaning",
        color: "rgba(220, 53, 69, 0.55)",
        className: "hl-cleaning",
        items: items.map((c, idx) => ({ span: c.span, data: { ...c, idx } })),
      },
    ]);
  });

  onDestroy(() => clearLayers());

  // 选中态:点击后 sidebar 项亮起,VirtualText 滚到位。
  let selectedIdx = $state(-1);
  function jumpTo(idx) {
    selectedIdx = idx;
    jumpToByteOffset(items[idx].span.start);
  }

  // 上下文预览:截 span 前后各 8 个字符。
  function preview(span) {
    const t = pipeline.dto.source_text;
    const ix = pipeline.byteIndex;
    const cs = ix.byteToChar(span.start);
    const ce = ix.byteToChar(span.end);
    const before = t.slice(Math.max(0, cs - 8), cs);
    const target = t.slice(cs, ce);
    const after = t.slice(ce, Math.min(t.length, ce + 8));
    return { before, target, after };
  }

  // 列表用懒渲染,避免一次性塞数万条 DOM。
  const PAGE_SIZE = 200;
  let visibleCount = $state(PAGE_SIZE);
  function loadMore() {
    visibleCount = Math.min(items.length, visibleCount + PAGE_SIZE);
  }
</script>

<div class="panel">
  <h2>2. 文本处理 <span class="count">{items.length} 条</span></h2>

  {#if !pipeline.dto}
    <p class="hint">请先在阶段 1 加载文件。</p>
  {:else}
    <div class="kinds">
      {#each Object.entries(counts) as [k, n]}
        <span class="chip">{KIND_LABEL[k] ?? k} <strong>{n}</strong></span>
      {/each}
    </div>

    {#if items.length === 0}
      <p class="hint">无清洗标注 —— 文本本身已经很干净。</p>
    {:else}
      <ul class="list">
        {#each items.slice(0, visibleCount) as c, idx (idx)}
          {@const p = preview(c.span)}
          <li>
            <button
              class="item"
              class:active={idx === selectedIdx}
              onclick={() => jumpTo(idx)}
            >
              <span class="kind">{KIND_LABEL[c.kind] ?? c.kind}</span>
              <span class="ctx">
                <span class="dim">{p.before}</span><mark>{p.target}</mark
                ><span class="dim">{p.after}</span>
              </span>
              <span class="offset">[{c.span.start}–{c.span.end}]</span>
            </button>
          </li>
        {/each}
      </ul>

      {#if visibleCount < items.length}
        <button class="load-more" onclick={loadMore}>
          再加载 {Math.min(PAGE_SIZE, items.length - visibleCount)} 条
          (已显示 {visibleCount} / {items.length})
        </button>
      {/if}
    {/if}

    <div class="reserved">
      <h3>灰区 / 可疑(占位)</h3>
      <p class="hint">阶段三的水印检测会在这里填入"中等可疑度"的黄色标注。</p>
    </div>
  {/if}
</div>

<style>
  .panel { padding: 12px 14px; }
  h2 { font-size: 14px; margin: 0 0 10px 0; display: flex; align-items: center; gap: 6px; }
  h2 .count { font-size: 11px; color: #52606d; font-weight: normal; }
  h3 { font-size: 11px; margin: 12px 0 4px 0; color: #52606d; text-transform: uppercase; letter-spacing: 0.5px; }
  .hint { font-size: 12px; color: #52606d; }
  .kinds { display: flex; flex-wrap: wrap; gap: 4px; margin-bottom: 10px; }
  .chip {
    background: #fff;
    border: 1px solid #cbd2d9;
    border-radius: 10px;
    padding: 2px 8px;
    font-size: 10px;
    color: #52606d;
  }
  .chip strong { color: #c62828; margin-left: 2px; }
  .list { list-style: none; padding: 0; margin: 0; }
  .list li { margin-bottom: 1px; }
  .item {
    display: grid;
    grid-template-columns: auto 1fr auto;
    gap: 6px;
    width: 100%;
    text-align: left;
    background: #fff;
    border: 1px solid transparent;
    border-radius: 3px;
    padding: 4px 6px;
    font-size: 11px;
    cursor: pointer;
    color: #1f2933;
    align-items: center;
  }
  .item:hover { background: #eef1f5; }
  .item.active { background: #fff4f5; border-color: #ef9a9a; }
  .kind {
    background: #ffebee;
    color: #c62828;
    padding: 1px 5px;
    border-radius: 2px;
    font-size: 9px;
    white-space: nowrap;
  }
  .ctx {
    font-family: "PingFang SC", "Microsoft YaHei", serif;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ctx .dim { color: #9aa5b1; }
  .ctx mark {
    background: rgba(220, 53, 69, 0.4);
    color: #1f2933;
    padding: 0 1px;
    border-radius: 2px;
  }
  .offset {
    font-family: Consolas, monospace;
    font-size: 9px;
    color: #9aa5b1;
  }
  .load-more {
    margin-top: 8px;
    width: 100%;
    padding: 4px;
    font-size: 11px;
    background: #fff;
    border: 1px dashed #cbd2d9;
    color: #52606d;
    cursor: pointer;
    border-radius: 3px;
  }
  .load-more:hover { background: #eef1f5; }
  .reserved {
    margin-top: 16px;
    padding-top: 10px;
    border-top: 1px dashed #cbd2d9;
  }
</style>
