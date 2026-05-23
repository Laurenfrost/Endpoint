<script>
  // 阶段 3:章节分析 —— 卷-章树 + 蓝色 heading 高亮 + 绿色 volume 高亮 + 跳转。
  // 标尺多层叠加:红(清洗)+ 蓝(章)+ 绿(卷)。
  import { pipeline } from "../stores/pipeline.svelte.js";
  import {
    setLayers,
    clearLayers,
    jumpToByteOffset,
  } from "../stores/annotations.svelte.js";
  import { onDestroy } from "svelte";

  const ORIGIN_LABEL = {
    regex_match: "规则",
    structural: "结构补",
    llm_adjudicated: "LLM",
    fallback: "兜底",
  };

  const entries = $derived(pipeline.dto?.book.entries ?? []);

  // 折叠状态:默认全展开。volIdx -> 是否折叠。
  let collapsed = $state({});
  function toggle(i) {
    collapsed[i] = !collapsed[i];
  }
  function expandAll() {
    collapsed = {};
  }
  function collapseAll() {
    const m = {};
    entries.forEach((e, i) => {
      if (e.type === "volume") m[i] = true;
    });
    collapsed = m;
  }

  // 注入三层 annotations:cleaning(红,半透明)+ heading(蓝)+ volume(绿)。
  $effect(() => {
    if (!pipeline.dto) {
      clearLayers();
      return;
    }
    const cleanings = pipeline.dto.cleaning.map((c) => ({ span: c.span }));
    const headings = [];
    const volumes = [];
    for (const e of entries) {
      if (e.type === "volume") {
        volumes.push({ span: e.heading_span });
        for (const c of e.chapters) {
          if (c.heading_span.start < c.heading_span.end) {
            headings.push({ span: c.heading_span });
          }
        }
      } else {
        if (e.heading_span.start < e.heading_span.end) {
          headings.push({ span: e.heading_span });
        }
      }
    }
    setLayers([
      {
        id: "cleaning",
        color: "rgba(220, 53, 69, 0.4)",
        className: "hl-cleaning",
        items: cleanings,
      },
      {
        id: "heading",
        color: "rgba(31, 111, 235, 0.75)",
        className: "hl-heading",
        items: headings,
      },
      {
        id: "volume",
        color: "rgba(46, 125, 50, 0.85)",
        className: "hl-volume",
        items: volumes,
      },
    ]);
  });

  onDestroy(() => clearLayers());

  let selectedKey = $state("");
  function go(span, key) {
    selectedKey = key;
    jumpToByteOffset(span.start);
  }

  // 统计
  const stats = $derived.by(() => {
    let vols = 0, chs = 0, fallback = 0;
    for (const e of entries) {
      if (e.type === "volume") {
        vols += 1;
        for (const c of e.chapters) {
          chs += 1;
          if (c.origin === "fallback") fallback += 1;
        }
      } else {
        chs += 1;
        if (e.origin === "fallback") fallback += 1;
      }
    }
    return { vols, chs, fallback };
  });
</script>

<div class="panel">
  <h2>3. 章节分析</h2>

  {#if !pipeline.dto}
    <p class="hint">请先在阶段 1 加载文件。</p>
  {:else}
    <div class="stats">
      <span class="chip vol">{stats.vols} 卷</span>
      <span class="chip ch">{stats.chs} 章</span>
      {#if stats.fallback > 0}
        <span class="chip fb" title="未由规则命中、由兜底逻辑产生的章">
          {stats.fallback} 兜底
        </span>
      {/if}
    </div>

    <div class="toolbar">
      <button onclick={expandAll}>全展开</button>
      <button onclick={collapseAll}>全折叠</button>
    </div>

    <ul class="tree">
      {#each entries as e, i (i)}
        {#if e.type === "volume"}
          {@const isFolded = collapsed[i]}
          <li class="volume">
            <div class="vol-row">
              <button
                class="caret"
                onclick={() => toggle(i)}
                aria-label={isFolded ? "展开" : "折叠"}
              >
                {isFolded ? "▶" : "▼"}
              </button>
              <button
                class="title vol-title"
                class:active={selectedKey === `v${i}`}
                onclick={() => go(e.heading_span, `v${i}`)}
                title={`${e.title} · ${e.chapters.length} 章 · ${ORIGIN_LABEL[e.origin] ?? e.origin}`}
              >
                <span class="text">{e.title}</span>
                <span class="badge">{e.chapters.length}</span>
              </button>
            </div>
            {#if !isFolded}
              <ul class="chapters">
                {#each e.chapters as c, j (j)}
                  <li>
                    <button
                      class="title ch-title"
                      class:active={selectedKey === `v${i}c${j}`}
                      class:fallback={c.origin === "fallback"}
                      onclick={() => go(c.heading_span, `v${i}c${j}`)}
                      title={`${c.title} · ${ORIGIN_LABEL[c.origin] ?? c.origin}${c.matched_rule_id ? ` · ${c.matched_rule_id}` : ""}`}
                    >
                      <span class="text">{c.title}</span>
                      {#if c.origin !== "regex_match"}
                        <span class="origin-badge">{ORIGIN_LABEL[c.origin] ?? c.origin}</span>
                      {/if}
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          </li>
        {:else}
          <li class="chapter">
            <button
              class="title ch-title top"
              class:active={selectedKey === `c${i}`}
              class:fallback={e.origin === "fallback"}
              onclick={() => go(e.heading_span, `c${i}`)}
              title={`${e.title} · ${ORIGIN_LABEL[e.origin] ?? e.origin}`}
            >
              <span class="text">{e.title}</span>
              {#if e.origin !== "regex_match"}
                <span class="origin-badge">{ORIGIN_LABEL[e.origin] ?? e.origin}</span>
              {/if}
            </button>
          </li>
        {/if}
      {/each}
    </ul>
  {/if}
</div>

<style>
  .panel { padding: 12px 14px; }
  h2 { font-size: 14px; margin: 0 0 10px 0; }
  .hint { font-size: 12px; color: #52606d; }
  .stats { display: flex; gap: 4px; margin-bottom: 8px; }
  .chip {
    background: #fff;
    border: 1px solid #cbd2d9;
    border-radius: 10px;
    padding: 2px 8px;
    font-size: 10px;
  }
  .chip.vol { color: #2e7d32; border-color: #a5d6a7; }
  .chip.ch { color: #1f6feb; border-color: #b3c7e6; }
  .chip.fb { color: #8a5400; border-color: #f5c074; background: #fff4e5; }
  .toolbar { display: flex; gap: 4px; margin-bottom: 8px; }
  .toolbar button {
    font-size: 10px;
    padding: 2px 8px;
    background: #fff;
    border: 1px solid #cbd2d9;
    border-radius: 3px;
    cursor: pointer;
    color: #52606d;
  }
  .toolbar button:hover { background: #eef1f5; }
  .tree, .chapters {
    list-style: none;
    padding: 0;
    margin: 0;
  }
  .chapters { padding-left: 16px; }
  .vol-row { display: flex; align-items: center; gap: 2px; }
  .caret {
    background: transparent;
    border: none;
    width: 14px;
    font-size: 9px;
    color: #52606d;
    cursor: pointer;
    padding: 0;
  }
  .title {
    flex: 1;
    text-align: left;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 3px;
    padding: 3px 6px;
    font-size: 12px;
    cursor: pointer;
    color: #1f2933;
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .title:hover { background: #eef1f5; }
  .title.active { background: #e3eefb; border-color: #1f6feb; }
  .title .text {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .vol-title { font-weight: 600; color: #2e7d32; }
  .vol-title.active { background: #e8f5e9; border-color: #2e7d32; }
  .vol-title .badge {
    background: #e8f5e9;
    color: #2e7d32;
    font-size: 9px;
    padding: 1px 5px;
    border-radius: 8px;
    font-weight: normal;
  }
  .ch-title.top { font-style: italic; color: #52606d; }
  .ch-title.fallback .text { color: #8a5400; }
  .origin-badge {
    background: #fff4e5;
    color: #8a5400;
    font-size: 9px;
    padding: 0 4px;
    border-radius: 2px;
  }
</style>
