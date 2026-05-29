<script>
  // 概览标尺:右缘竖条,按 byteOffset / sourceBytes 比例画色块。
  // 与正文高亮、侧边栏共用同一份 annotations。点击 → 跳转。
  import { onMount } from "svelte";
  import { pipeline } from "../stores/pipeline.svelte.js";
  import { annotations, jumpToByteOffset } from "../stores/annotations.svelte.js";
  import { mode } from "mode-watcher";

  const WIDTH = 14;
  const MIN_BLOCK_PX = 2;

  /** @type {HTMLCanvasElement} */
  let canvas;
  let height = $state(600);

  const totalBytes = $derived(pipeline.byteIndex?.totalBytes ?? 0);

  // 把 layer.color 里的 CSS 变量(var(--xxx))解析成真实色值,canvas ctx 不接受 var()。
  function resolveColor(c) {
    if (!c) return "transparent";
    if (typeof c === "string" && c.startsWith("var(")) {
      const m = c.match(/var\((--[^)]+)\)/);
      if (m) {
        const v = getComputedStyle(document.documentElement).getPropertyValue(m[1]).trim();
        return v || "transparent";
      }
    }
    return c;
  }

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
    const bg = getComputedStyle(document.documentElement).getPropertyValue("--muted").trim() || "#f5f7fa";
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, WIDTH, height);

    for (const layer of annotations.layers) {
      ctx.fillStyle = resolveColor(layer.color);
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
    // 依赖:layers / height / totalBytes / mode(模式切换时重绘以取新色)
    void annotations.layers;
    void height;
    void totalBytes;
    void mode.current;
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

<div bind:this={container} class="w-3.5 shrink-0 cursor-crosshair overflow-hidden border-l bg-muted">
  <canvas bind:this={canvas} onclick={onClick} aria-label="概览标尺" class="block"></canvas>
</div>
