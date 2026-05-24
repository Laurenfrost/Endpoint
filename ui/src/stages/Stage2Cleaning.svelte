<script>
  // 阶段 2:文本处理 —— 清洗(红)+ 水印 auto(橙)+ 水印 suspect(黄)。
  //
  // 数据流(阶段三 3.3 镜像后):
  //   - pipeline.cleaning 中 kind 是 4 种格式整理变体之一 → cleaning 列表 + 红层
  //   - pipeline.cleaning 中 kind 是 watermark_* 变体之一 → 来自 auto 水印镜像 → 橙层
  //                                                          (不进 cleaning 列表,只在水印列表里显示)
  //   - pipeline.watermark 全量(auto + suspect)→ 水印列表;suspect → 黄层
  //
  // 颜色与命名详见 `docs/stage3-design.md` 第六节 6.2。
  import { pipeline } from "../stores/pipeline.svelte.js";
  import {
    setLayers,
    clearLayers,
    jumpToByteOffset,
  } from "../stores/annotations.svelte.js";
  import { onDestroy } from "svelte";

  const KIND_LABEL = {
    blank_line_compression: "空行压缩",
    fullwidth_space: "全角空格",
    control_char: "控制字符",
    trailing_whitespace: "行尾空白",
  };

  const SIGNAL_LABEL = {
    repetition: "行频",
    non_cjk_ratio: "非中文",
    keyword_regex: "关键词",
  };

  // —— 数据派生 ——
  // 清洗列表只显示阶段二的 4 种格式整理变体(把 watermark_* 镜像剔除,后者在水印列表里看)。
  const cleaningItems = $derived(
    (pipeline.dto?.cleaning ?? []).filter((c) => !c.kind.startsWith("watermark_"))
  );
  const cleaningCounts = $derived.by(() => {
    const m = {};
    for (const c of cleaningItems) m[c.kind] = (m[c.kind] ?? 0) + 1;
    return m;
  });
  // auto 水印镜像在 cleaning 里的 span(仅作高亮源,不进列表)
  const autoMirrorSpans = $derived(
    (pipeline.dto?.cleaning ?? [])
      .filter((c) => c.kind.startsWith("watermark_"))
      .map((c) => ({ span: c.span }))
  );

  // 水印列表(全部 verdict)
  const watermarkItems = $derived(pipeline.dto?.watermark ?? []);
  const wmCounts = $derived.by(() => {
    let auto = 0, suspect = 0;
    for (const w of watermarkItems) {
      if (w.verdict === "auto") auto += 1;
      else if (w.verdict === "suspect") suspect += 1;
    }
    return { auto, suspect, total: watermarkItems.length };
  });
  // suspect span(用于黄色高亮层)
  const suspectSpans = $derived(
    watermarkItems
      .filter((w) => w.verdict === "suspect")
      .map((w) => ({ span: w.span }))
  );

  // 注入三层 annotations。
  $effect(() => {
    if (!pipeline.dto) {
      clearLayers();
      return;
    }
    setLayers([
      {
        id: "cleaning_format",
        color: "rgba(220, 53, 69, 0.55)",
        className: "hl-cleaning",
        items: cleaningItems.map((c) => ({ span: c.span })),
      },
      {
        id: "watermark_auto",
        color: "rgba(255, 140, 0, 0.55)",
        className: "hl-watermark-auto",
        items: autoMirrorSpans,
      },
      {
        id: "watermark_suspect",
        color: "rgba(245, 196, 0, 0.55)",
        className: "hl-watermark-suspect",
        items: suspectSpans,
      },
    ]);
  });

  onDestroy(() => clearLayers());

  // —— 上下文预览(共用) ——
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

  // —— 清洗列表(上半屏) ——
  let cleaningSelected = $state(-1);
  function jumpCleaning(idx) {
    cleaningSelected = idx;
    jumpToByteOffset(cleaningItems[idx].span.start);
  }
  const CLEAN_PAGE = 200;
  let cleaningVisible = $state(CLEAN_PAGE);
  function loadMoreCleaning() {
    cleaningVisible = Math.min(cleaningItems.length, cleaningVisible + CLEAN_PAGE);
  }

  // —— 水印列表(下半屏) ——
  let wmTab = $state("all"); // all | auto | suspect
  let wmSelected = $state(-1);
  const WM_PAGE = 200;
  let wmVisible = $state(WM_PAGE);
  const wmFiltered = $derived.by(() => {
    if (wmTab === "auto") return watermarkItems.filter((w) => w.verdict === "auto");
    if (wmTab === "suspect") return watermarkItems.filter((w) => w.verdict === "suspect");
    return watermarkItems;
  });
  // tab 切换时复位选中与可见量
  $effect(() => {
    void wmTab;
    wmSelected = -1;
    wmVisible = WM_PAGE;
  });
  function jumpWm(idx) {
    wmSelected = idx;
    jumpToByteOffset(wmFiltered[idx].span.start);
  }
  function loadMoreWm() {
    wmVisible = Math.min(wmFiltered.length, wmVisible + WM_PAGE);
  }

  function formatScore(s) {
    return (s * 100).toFixed(0) + "%";
  }
</script>

<div class="panel">
  <h2>2. 文本处理</h2>

  {#if !pipeline.dto}
    <p class="hint">请先在阶段 1 加载文件。</p>
  {:else}
    <!-- ============ 上半屏:格式清洗(红色) ============ -->
    <section class="block">
      <h3>
        <span class="dot dot-red"></span>
        格式清洗 <span class="count">{cleaningItems.length} 条</span>
      </h3>

      {#if cleaningItems.length === 0}
        <p class="hint">无格式清洗 —— 文本已经很干净。</p>
      {:else}
        <div class="kinds">
          {#each Object.entries(cleaningCounts) as [k, n]}
            <span class="chip chip-red">{KIND_LABEL[k] ?? k} <strong>{n}</strong></span>
          {/each}
        </div>

        <ul class="list">
          {#each cleaningItems.slice(0, cleaningVisible) as c, idx (idx)}
            {@const p = preview(c.span)}
            <li>
              <button
                class="item item-cleaning"
                class:active={idx === cleaningSelected}
                onclick={() => jumpCleaning(idx)}
              >
                <span class="kind kind-cleaning">{KIND_LABEL[c.kind] ?? c.kind}</span>
                <span class="ctx">
                  <span class="dim">{p.before}</span><mark class="mk-cleaning">{p.target}</mark><span class="dim">{p.after}</span>
                </span>
                <span class="offset">[{c.span.start}–{c.span.end}]</span>
              </button>
            </li>
          {/each}
        </ul>

        {#if cleaningVisible < cleaningItems.length}
          <button class="load-more" onclick={loadMoreCleaning}>
            再加载 {Math.min(CLEAN_PAGE, cleaningItems.length - cleaningVisible)} 条
            (已显示 {cleaningVisible} / {cleaningItems.length})
          </button>
        {/if}
      {/if}
    </section>

    <!-- ============ 下半屏:水印(橙/黄)============ -->
    <section class="block">
      <h3>
        <span class="dot dot-orange"></span><span class="dot dot-yellow"></span>
        水印检测 <span class="count">auto {wmCounts.auto} · suspect {wmCounts.suspect}</span>
      </h3>

      {#if wmCounts.total === 0}
        <p class="hint">未识别出水印 —— 整本文本干净或规则未覆盖到。</p>
      {:else}
        <div class="tabs" role="tablist">
          <button class="tab" class:on={wmTab === "all"} onclick={() => (wmTab = "all")}>
            全部 <small>{wmCounts.total}</small>
          </button>
          <button class="tab" class:on={wmTab === "auto"} onclick={() => (wmTab = "auto")}>
            <span class="dot dot-orange"></span>自动 <small>{wmCounts.auto}</small>
          </button>
          <button class="tab" class:on={wmTab === "suspect"} onclick={() => (wmTab = "suspect")}>
            <span class="dot dot-yellow"></span>灰区 <small>{wmCounts.suspect}</small>
          </button>
        </div>

        {#if wmFiltered.length === 0}
          <p class="hint">当前 tab 下无数据。</p>
        {:else}
          <ul class="list">
            {#each wmFiltered.slice(0, wmVisible) as w, idx (`${w.span.start}-${w.span.end}-${idx}`)}
              {@const p = preview(w.span)}
              <li>
                <button
                  class="item item-watermark"
                  class:active={idx === wmSelected}
                  onclick={() => jumpWm(idx)}
                >
                  <span class="verdict-row">
                    <span class="verdict verdict-{w.verdict}">{w.verdict === "auto" ? "自动" : "灰区"}</span>
                    <span class="score">分 {formatScore(w.score)}</span>
                    <span class="offset">[{w.span.start}–{w.span.end}]</span>
                  </span>
                  <span class="ctx">
                    <span class="dim">{p.before}</span><mark class="mk-{w.verdict}">{p.target}</mark><span class="dim">{p.after}</span>
                  </span>
                  <div class="signals">
                    {#each w.signals as s}
                      <div class="signal">
                        <span class="sig-kind">{SIGNAL_LABEL[s.kind] ?? s.kind}</span>
                        <span class="sig-bar"><span class="sig-fill" style="width:{Math.round(s.score * 100)}%"></span></span>
                        {#if s.detail}<span class="sig-detail" title={s.detail}>{s.detail}</span>{/if}
                      </div>
                    {/each}
                  </div>
                </button>
              </li>
            {/each}
          </ul>

          {#if wmVisible < wmFiltered.length}
            <button class="load-more" onclick={loadMoreWm}>
              再加载 {Math.min(WM_PAGE, wmFiltered.length - wmVisible)} 条
              (已显示 {wmVisible} / {wmFiltered.length})
            </button>
          {/if}
        {/if}
      {/if}
    </section>
  {/if}
</div>

<style>
  .panel { padding: 12px 14px; }
  h2 { font-size: 14px; margin: 0 0 10px 0; }
  h3 {
    font-size: 11px;
    margin: 0 0 6px 0;
    color: #52606d;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  h3 .count { font-size: 10px; color: #9aa5b1; margin-left: auto; text-transform: none; letter-spacing: 0; font-weight: normal; }
  .hint { font-size: 12px; color: #52606d; }

  .block { margin-bottom: 18px; }
  .block + .block { padding-top: 14px; border-top: 1px dashed #cbd2d9; }

  .dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; }
  .dot-red    { background: rgba(220, 53, 69, 0.85); }
  .dot-orange { background: rgba(255, 140, 0, 0.85); }
  .dot-yellow { background: rgba(245, 196, 0, 0.95); }

  .kinds { display: flex; flex-wrap: wrap; gap: 4px; margin-bottom: 8px; }
  .chip {
    background: #fff;
    border: 1px solid #cbd2d9;
    border-radius: 10px;
    padding: 2px 8px;
    font-size: 10px;
    color: #52606d;
  }
  .chip-red strong { color: #c62828; margin-left: 2px; }

  .list { list-style: none; padding: 0; margin: 0; }
  .list li { margin-bottom: 1px; }

  .item {
    display: grid;
    width: 100%;
    text-align: left;
    background: #fff;
    border: 1px solid transparent;
    border-radius: 3px;
    padding: 4px 6px;
    font-size: 11px;
    cursor: pointer;
    color: #1f2933;
  }
  .item-cleaning {
    grid-template-columns: auto 1fr auto;
    gap: 6px;
    align-items: center;
  }
  .item-cleaning.active { background: #fff4f5; border-color: #ef9a9a; }
  .item-watermark {
    grid-template-rows: auto auto auto;
    gap: 3px;
    padding: 6px 8px;
  }
  .item-watermark.active { background: #fff8ee; border-color: #ffb84d; }
  .item:hover { background: #eef1f5; }
  .item-watermark.active:hover { background: #fff4e1; }

  .kind {
    padding: 1px 5px;
    border-radius: 2px;
    font-size: 9px;
    white-space: nowrap;
  }
  .kind-cleaning { background: #ffebee; color: #c62828; }

  .verdict-row { display: flex; gap: 6px; align-items: center; }
  .verdict {
    padding: 1px 6px;
    border-radius: 8px;
    font-size: 9px;
    font-weight: 600;
  }
  .verdict-auto    { background: #fff0d6; color: #b45309; }
  .verdict-suspect { background: #fff8d6; color: #8a6d00; }
  .score {
    font-family: Consolas, monospace;
    font-size: 9px;
    color: #52606d;
  }

  .ctx {
    font-family: "PingFang SC", "Microsoft YaHei", serif;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .ctx .dim { color: #9aa5b1; }
  .ctx mark { color: #1f2933; padding: 0 1px; border-radius: 2px; }
  .mk-cleaning { background: rgba(220, 53, 69, 0.4); }
  .mk-auto     { background: rgba(255, 140, 0, 0.4); }
  .mk-suspect  { background: rgba(245, 196, 0, 0.5); }

  .offset {
    font-family: Consolas, monospace;
    font-size: 9px;
    color: #9aa5b1;
    margin-left: auto;
  }

  .signals { display: flex; flex-direction: column; gap: 2px; margin-top: 2px; }
  .signal {
    display: grid;
    grid-template-columns: 40px 60px 1fr;
    gap: 6px;
    align-items: center;
    font-size: 9px;
    color: #52606d;
  }
  .sig-kind { font-weight: 600; color: #1f2933; }
  .sig-bar {
    display: inline-block;
    height: 4px;
    background: #e6e9ee;
    border-radius: 2px;
    overflow: hidden;
  }
  .sig-fill {
    display: block;
    height: 100%;
    background: rgba(255, 140, 0, 0.7);
  }
  .sig-detail {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tabs {
    display: flex;
    gap: 4px;
    margin-bottom: 8px;
  }
  .tab {
    background: #fff;
    border: 1px solid #cbd2d9;
    border-radius: 12px;
    padding: 3px 10px;
    font-size: 10px;
    color: #52606d;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .tab small { color: #9aa5b1; font-size: 9px; }
  .tab.on { background: #1f6feb; color: #fff; border-color: #1f6feb; }
  .tab.on small { color: rgba(255, 255, 255, 0.75); }

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
</style>
