<script>
  // 虚拟滚动文本视图。整本两百万字一次性塞 DOM 必崩,本组件:
  //
  // 1. 加载时按 `\n` 一次性分行,记每行 char/byte 区间(O(n))。
  // 2. 估算每行可视高度 = ceil(line_chars / WRAP_COL) × ROW_HEIGHT。
  //    WRAP_COL 按容器宽度 / 单字宽度 动态算;ROW_HEIGHT 取 font-size × line-height。
  // 3. 用 cumHeights 做二分,O(log n) 找当前 scrollTop 对应的可视行区间。
  // 4. 只渲染可视区域 + buffer 行,绝对定位到 cumHeights[i]。
  // 5. 每行内根据当前激活的所有 layers,把命中的 annotation 切片作为
  //    `<span class="hl-...">` 内嵌渲染。
  //
  // 已知限制:行高靠估算,长段落实际渲染可能略高于估算,导致下方行被覆盖少许。
  //   2M 字符的网文绝大多数段落 ≤ 100 字符,实际偏差肉眼难察。
  //   2.5 之后若需要精确度可加 ResizeObserver 二次校正。
  import { onMount } from "svelte";
  import { pipeline } from "../stores/pipeline.svelte.js";
  import { annotations } from "../stores/annotations.svelte.js";

  const ROW_HEIGHT = 28;        // px,= 16 * 1.7
  const BUFFER_ROWS = 8;        // 视口上下各多渲染几行,平滑滚动
  const FONT_SIZE_PX = 16;      // 与 .content 的 font-size 一致

  /** @type {HTMLDivElement} */
  let scroller;
  let scrollTop = $state(0);
  let viewportHeight = $state(600);
  let containerWidth = $state(700);

  // 一次性按 \n 切行,计算 char/byte 区间。
  // line[i] = { byteStart, byteEnd, charStart, charEnd }
  const lines = $derived.by(() => {
    const ix = pipeline.byteIndex;
    const text = pipeline.dto?.source_text;
    if (!ix || !text) return [];
    const out = [];
    let charStart = 0;
    for (let i = 0; i < text.length; i++) {
      if (text.charCodeAt(i) === 0x0a /* '\n' */) {
        out.push({
          charStart,
          charEnd: i, // 不含 \n
          byteStart: ix.charToByte(charStart),
          byteEnd: ix.charToByte(i),
        });
        charStart = i + 1;
      }
    }
    if (charStart <= text.length) {
      out.push({
        charStart,
        charEnd: text.length,
        byteStart: ix.charToByte(charStart),
        byteEnd: ix.charToByte(text.length),
      });
    }
    return out;
  });

  // 每行字符数对应的可视高度估算。WRAP_COL 由容器宽度推算。
  const wrapCol = $derived(Math.max(20, Math.floor((containerWidth - 48) / FONT_SIZE_PX)));
  // cumHeights[i] = 前 i 行的总像素高;长度 = lines.length + 1。
  const cumHeights = $derived.by(() => {
    const arr = new Uint32Array(lines.length + 1);
    let acc = 0;
    for (let i = 0; i < lines.length; i++) {
      const chars = lines[i].charEnd - lines[i].charStart;
      const rows = Math.max(1, Math.ceil(chars / wrapCol));
      acc += rows * ROW_HEIGHT;
      arr[i + 1] = acc;
    }
    return arr;
  });
  const totalHeight = $derived(cumHeights[cumHeights.length - 1] || 0);

  // 二分查找:cumHeights[i] <= y < cumHeights[i+1] 的最大 i。
  function findLineAtY(y) {
    if (lines.length === 0) return 0;
    let lo = 0, hi = lines.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >>> 1;
      if (cumHeights[mid] <= y) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  // 当前可视范围(扩展 buffer)
  const visible = $derived.by(() => {
    if (lines.length === 0) return { first: 0, last: -1 };
    const firstRaw = findLineAtY(scrollTop);
    const lastRaw = findLineAtY(scrollTop + viewportHeight);
    return {
      first: Math.max(0, firstRaw - BUFFER_ROWS),
      last: Math.min(lines.length - 1, lastRaw + BUFFER_ROWS),
    };
  });

  // 每层 annotations 按 char.start 排序后的索引(O(n log n) 一次);
  // 用于在每行渲染时 O(log n + k) 找出与该行相交的 ann。
  const layersChar = $derived.by(() => {
    const ix = pipeline.byteIndex;
    if (!ix) return [];
    return annotations.layers.map((layer) => {
      const items = layer.items
        .map((it) => ({
          charStart: ix.byteToChar(it.span.start),
          charEnd: ix.byteToChar(it.span.end),
          data: it.data,
        }))
        .filter((a) => a.charEnd > a.charStart)
        .sort((a, b) => a.charStart - b.charStart);
      return { id: layer.id, className: layer.className, items };
    });
  });

  // 单行渲染:返回 [{ text, hl: className|null }] 段列表。
  // 多层叠加用"先按起点排序、再线性合并"——同位置多层时取第一个 className。
  function partsForLine(line, text) {
    const inter = [];
    for (const layer of layersChar) {
      const arr = layer.items;
      // 二分找第一个 ann.charEnd > line.charStart
      let lo = 0, hi = arr.length;
      while (lo < hi) {
        const mid = (lo + hi) >>> 1;
        if (arr[mid].charEnd <= line.charStart) lo = mid + 1;
        else hi = mid;
      }
      for (let i = lo; i < arr.length && arr[i].charStart < line.charEnd; i++) {
        const a = arr[i];
        const s = Math.max(a.charStart, line.charStart);
        const e = Math.min(a.charEnd, line.charEnd);
        if (e > s) inter.push({ s, e, cls: layer.className });
      }
    }
    if (inter.length === 0) {
      return [{ text: text.slice(line.charStart, line.charEnd), hl: null }];
    }
    inter.sort((a, b) => a.s - b.s || b.e - a.e);
    const parts = [];
    let cursor = line.charStart;
    for (const it of inter) {
      if (it.s < cursor) continue; // 被前一段覆盖
      if (it.s > cursor) parts.push({ text: text.slice(cursor, it.s), hl: null });
      parts.push({ text: text.slice(it.s, it.e), hl: it.cls });
      cursor = it.e;
    }
    if (cursor < line.charEnd) parts.push({ text: text.slice(cursor, line.charEnd), hl: null });
    return parts;
  }

  function onScroll() {
    if (scroller) scrollTop = scroller.scrollTop;
  }

  function measure() {
    if (!scroller) return;
    viewportHeight = scroller.clientHeight;
    containerWidth = scroller.clientWidth;
  }

  onMount(() => {
    measure();
    const ro = new ResizeObserver(measure);
    if (scroller) ro.observe(scroller);
    return () => ro.disconnect();
  });

  // jumpTo:当 annotations.jumpTo.version 变化时滚动到对应 byte offset。
  let jumpVersion = $state(0);
  $effect(() => {
    if (!scroller || !pipeline.byteIndex) return;
    const jv = annotations.jumpTo.version;
    if (jv === jumpVersion) return;
    jumpVersion = jv;
    const charIdx = pipeline.byteIndex.byteToChar(annotations.jumpTo.offset);
    // 找该字符所在行
    let lo = 0, hi = lines.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >>> 1;
      if (lines[mid].charStart <= charIdx) lo = mid;
      else hi = mid - 1;
    }
    const targetY = Math.max(0, cumHeights[lo] - 60); // 顶部留点 padding
    scroller.scrollTo({ top: targetY, behavior: "smooth" });
  });

</script>

<div class="scroller" bind:this={scroller} onscroll={onScroll}>
  <div class="canvas" style="height: {totalHeight}px;">
    {#each Array.from({ length: Math.max(0, visible.last - visible.first + 1) }, (_, k) => visible.first + k) as i (i)}
      {@const line = lines[i]}
      {@const parts = partsForLine(line, pipeline.dto.source_text)}
      <div class="line" style="top: {cumHeights[i]}px;">
        {#each parts as p, j}
          {#if p.hl}
            <span class={p.hl}>{p.text}</span>
          {:else}
            {p.text}
          {/if}
        {/each}
      </div>
    {/each}
  </div>
</div>

<style>
  .scroller {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    background: var(--background);
    position: relative;
  }
  .canvas {
    position: relative;
    width: 100%;
  }
  .line {
    position: absolute;
    left: 0;
    right: 0;
    padding: 0 24px;
    font-family: "PingFang SC", "Microsoft YaHei", "Source Han Serif",
      "Noto Serif CJK SC", serif;
    font-size: 16px;
    line-height: 1.75;
    color: var(--foreground);
    white-space: pre-wrap;
    word-break: break-all;
  }
  /* 高亮颜色:红=cleaning / 橙=auto 水印 / 黄=suspect 水印 / 蓝=章 / 绿=卷
     在 dark 模式下加深底色让高亮更显眼。 */
  :global(.hl-cleaning) {
    background: rgba(220, 53, 69, 0.22);
    border-bottom: 1px solid rgba(220, 53, 69, 0.7);
    border-radius: 2px;
  }
  :global(.dark .hl-cleaning) { background: rgba(220, 53, 69, 0.32); }

  :global(.hl-watermark-auto) {
    background: rgba(255, 140, 0, 0.22);
    border-bottom: 1px solid rgba(255, 140, 0, 0.85);
    border-radius: 2px;
  }
  :global(.dark .hl-watermark-auto) { background: rgba(255, 140, 0, 0.32); }

  :global(.hl-watermark-suspect) {
    background: rgba(245, 196, 0, 0.28);
    border-bottom: 1px dashed rgba(180, 142, 0, 0.85);
    border-radius: 2px;
  }
  :global(.dark .hl-watermark-suspect) { background: rgba(245, 196, 0, 0.4); }

  :global(.hl-heading) {
    background: rgba(31, 111, 235, 0.18);
    border-bottom: 1px solid rgba(31, 111, 235, 0.7);
    border-radius: 2px;
    font-weight: 600;
  }
  :global(.dark .hl-heading) { background: rgba(96, 165, 250, 0.28); }

  :global(.hl-volume) {
    background: rgba(46, 125, 50, 0.22);
    border-bottom: 1px solid rgba(46, 125, 50, 0.8);
    border-radius: 2px;
    font-weight: 700;
  }
  :global(.dark .hl-volume) { background: rgba(76, 175, 80, 0.3); }
</style>
