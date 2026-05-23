<script>
  // 概览标尺:右缘竖条,按 byteOffset / sourceBytes 比例画色块。
  // 与正文高亮、侧边栏共用同一份 annotations。点击 → 跳转。
  import { onMount } from "svelte";
  import { pipeline } from "../stores/pipeline.svelte.js";
  import { annotations, jumpToByteOffset } from "../stores/annotations.svelte.js";

  const WIDTH = 14;
  const MIN_BLOCK_PX = 2;

  /** @type {HTMLCanvasElement} */
  let canvas;
  let height = $state(600);

  const totalBytes = $derived(pipeline.byteIndex?.totalBytes ?? 0);

  function draw() {
    if (!canvas || totalBytes === 0) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = WIDTH * dpr;
    canvas.height = height * dpr;
    canvas.style.width = WIDTH + "px";
    canvas.style.height = height + "px";
    const ctx = canvas.getContext("2d");
    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, WIDTH, height);
    ctx.fillStyle = "#f5f7fa";
    ctx.fillRect(0, 0, WIDTH, height);

    for (const layer of annotations.layers) {
      ctx.fillStyle = layer.color;
      for (const it of layer.items) {
        const y = Math.floor((it.span.start / totalBytes) * height);
        const len = Math.max(
          MIN_BLOCK_PX,
          Math.ceil(((it.span.end - it.span.start) / totalBytes) * height)
        );
        ctx.fillRect(0, y, WIDTH, len);
      }
    }
  }

  $effect(() => {
    // 依赖:layers / height / totalBytes
    void annotations.layers;
    void height;
    void totalBytes;
    draw();
  });

  function onClick(evt) {
    if (totalBytes === 0) return;
    const rect = canvas.getBoundingClientRect();
    const y = evt.clientY - rect.top;
    const ratio = Math.max(0, Math.min(1, y / rect.height));
    jumpToByteOffset(Math.floor(ratio * totalBytes));
  }

  /** @type {HTMLDivElement} */
  let container;
  onMount(() => {
    const ro = new ResizeObserver(() => {
      if (container) height = container.clientHeight;
    });
    if (container) {
      ro.observe(container);
      height = container.clientHeight;
    }
    return () => ro.disconnect();
  });
</script>

<div class="ruler" bind:this={container}>
  <canvas bind:this={canvas} onclick={onClick} aria-label="概览标尺"></canvas>
</div>

<style>
  .ruler {
    width: 14px;
    background: #f5f7fa;
    border-left: 1px solid #cbd2d9;
    flex-shrink: 0;
    overflow: hidden;
    cursor: crosshair;
  }
  canvas {
    display: block;
  }
</style>
