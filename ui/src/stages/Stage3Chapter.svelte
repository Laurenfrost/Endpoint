<script>
  // 阶段 3:章节分析 —— 卷-章树 + 蓝色 heading 高亮 + 绿色 volume 高亮 + 跳转。
  // 标尺多层叠加:红(清洗)+ 蓝(章)+ 绿(卷)。
  import ChevronRight from "@lucide/svelte/icons/chevron-right";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import { pipeline } from "../stores/pipeline.svelte.js";
  import {
    setLayers,
    clearLayers,
    jumpToByteOffset,
  } from "../stores/annotations.svelte.js";
  import { onDestroy } from "svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { cn } from "$lib/utils.js";

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
      { id: "cleaning", color: "var(--hl-cleaning)", className: "hl-cleaning", items: cleanings },
      { id: "heading", color: "var(--hl-heading)", className: "hl-heading", items: headings },
      { id: "volume", color: "var(--hl-volume)", className: "hl-volume", items: volumes },
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

<div class="flex flex-col gap-2 p-3">
  <h2 class="text-sm font-semibold">3. 章节分析</h2>

  {#if !pipeline.dto}
    <p class="text-xs text-muted-foreground">请先在阶段 1 加载文件。</p>
  {:else}
    <div class="flex flex-wrap gap-1.5">
      <Badge variant="success">{stats.vols} 卷</Badge>
      <Badge variant="info">{stats.chs} 章</Badge>
      {#if stats.fallback > 0}
        <Badge variant="warning" title="未由规则命中、由兜底逻辑产生的章">
          {stats.fallback} 兜底
        </Badge>
      {/if}
    </div>

    <div class="flex gap-1.5">
      <Button variant="outline" size="sm" class="h-7 text-[11px]" onclick={expandAll}>全展开</Button>
      <Button variant="outline" size="sm" class="h-7 text-[11px]" onclick={collapseAll}>全折叠</Button>
    </div>

    <ul class="m-0 flex list-none flex-col gap-0.5 p-0">
      {#each entries as e, i (i)}
        {#if e.type === "volume"}
          {@const isFolded = collapsed[i]}
          <li>
            <div class="flex items-center gap-0.5">
              <button
                type="button"
                class="flex size-5 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-accent"
                onclick={() => toggle(i)}
                aria-label={isFolded ? "展开" : "折叠"}
              >
                {#if isFolded}
                  <ChevronRight class="size-3.5" />
                {:else}
                  <ChevronDown class="size-3.5" />
                {/if}
              </button>
              <button
                type="button"
                class={cn(
                  "flex min-w-0 flex-1 items-center gap-1.5 rounded border border-transparent px-1.5 py-1 text-left text-xs font-semibold transition-colors",
                  "text-emerald-600 dark:text-emerald-400 hover:bg-emerald-500/10",
                  selectedKey === `v${i}` && "border-emerald-500 bg-emerald-500/15",
                )}
                onclick={() => go(e.heading_span, `v${i}`)}
                title={`${e.title} · ${e.chapters.length} 章 · ${ORIGIN_LABEL[e.origin] ?? e.origin}`}
              >
                <span class="min-w-0 flex-1 truncate">{e.title}</span>
                <span class="rounded-full bg-emerald-500/15 px-1.5 py-0.5 text-[9px] font-normal">
                  {e.chapters.length}
                </span>
              </button>
            </div>
            {#if !isFolded}
              <ul class="m-0 ml-4 flex list-none flex-col gap-0.5 p-0">
                {#each e.chapters as c, j (j)}
                  <li>
                    <button
                      type="button"
                      class={cn(
                        "flex w-full items-center gap-1.5 rounded border border-transparent px-1.5 py-1 text-left text-xs transition-colors hover:bg-accent",
                        selectedKey === `v${i}c${j}` && "border-primary bg-primary/10",
                        c.origin === "fallback" && "text-amber-700 dark:text-amber-400",
                      )}
                      onclick={() => go(c.heading_span, `v${i}c${j}`)}
                      title={`${c.title} · ${ORIGIN_LABEL[c.origin] ?? c.origin}${c.matched_rule_id ? ` · ${c.matched_rule_id}` : ""}`}
                    >
                      <span class="min-w-0 flex-1 truncate">{c.title}</span>
                      {#if c.origin !== "regex_match"}
                        <span class="rounded bg-amber-500/15 px-1 py-0 text-[9px] text-amber-700 dark:text-amber-300">
                          {ORIGIN_LABEL[c.origin] ?? c.origin}
                        </span>
                      {/if}
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          </li>
        {:else}
          <li>
            <button
              type="button"
              class={cn(
                "flex w-full items-center gap-1.5 rounded border border-transparent px-1.5 py-1 text-left text-xs italic text-muted-foreground transition-colors hover:bg-accent",
                selectedKey === `c${i}` && "border-primary bg-primary/10",
                e.origin === "fallback" && "text-amber-700 dark:text-amber-400",
              )}
              onclick={() => go(e.heading_span, `c${i}`)}
              title={`${e.title} · ${ORIGIN_LABEL[e.origin] ?? e.origin}`}
            >
              <span class="min-w-0 flex-1 truncate">{e.title}</span>
              {#if e.origin !== "regex_match"}
                <span class="rounded bg-amber-500/15 px-1 py-0 text-[9px] text-amber-700 dark:text-amber-300">
                  {ORIGIN_LABEL[e.origin] ?? e.origin}
                </span>
              {/if}
            </button>
          </li>
        {/if}
      {/each}
    </ul>
  {/if}
</div>
