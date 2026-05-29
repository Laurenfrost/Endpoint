<script>
  // 阶段 2:文本处理 —— 清洗(红)+ 水印 auto(橙)+ 水印 suspect(黄)。
  //
  // 数据流(阶段三 3.3 镜像后):
  //   - pipeline.cleaning 中 kind 是 5 种格式整理变体之一 → cleaning 列表 + 红层
  //   - pipeline.cleaning 中 kind 是 watermark_* 变体之一 → 来自 auto 水印镜像 → 橙层
  //   - pipeline.watermark 全量(auto + suspect)→ 水印列表;suspect → 黄层
  import ChevronRight from "@lucide/svelte/icons/chevron-right";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import X from "@lucide/svelte/icons/x";
  import { pipeline } from "../stores/pipeline.svelte.js";
  import {
    setLayers,
    clearLayers,
    jumpToByteOffset,
  } from "../stores/annotations.svelte.js";
  import { setBusy } from "../stores/progress.svelte.js";
  import { setPipeline } from "../stores/pipeline.svelte.js";
  import { loadAndAnalyze, adjudicateWatermarks, induceWatermarkRule, saveInducedRule } from "../ipc.js";
  import { llm } from "../stores/llm.svelte.js";
  import {
    decisions,
    getDecision,
    toggleDecision,
    bulkSet,
    bulkClear,
    clearAllDecisions,
    decisionCount,
  } from "../stores/decisions.svelte.js";
  import { onDestroy } from "svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Checkbox } from "$lib/components/ui/checkbox/index.js";
  import { Slider } from "$lib/components/ui/slider/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import * as Alert from "$lib/components/ui/alert/index.js";
  import { cn } from "$lib/utils.js";

  const KIND_LABEL = {
    blank_line_compression: "空行压缩",
    leading_fullwidth_space: "段首全角缩进",
    inline_fullwidth_space: "行内全角连排",
    control_char: "控制字符",
    trailing_whitespace: "行尾空白",
    watermark_keyword: "水印:关键词",
    watermark_repetition: "水印:重复",
    watermark_non_cjk: "水印:非中文",
  };

  const SIGNAL_LABEL = {
    repetition: "行频",
    non_cjk_ratio: "非中文",
    keyword_regex: "关键词",
    llm_adjudication: "LLM 仲裁",
  };

  // —— v2 清洗策略面板 ——
  const CLEANING_DEFAULT = {
    blank_line_compression: false,
    leading_fullwidth_space: false,
    inline_fullwidth_space: false,
    control_char: false,
    trailing_whitespace: false,
  };
  let cfg = $state({ ...CLEANING_DEFAULT });
  let cleaningLastApplied = $state({ ...CLEANING_DEFAULT });
  const cleaningDirty = $derived(
    Object.keys(CLEANING_DEFAULT).some((k) => cfg[k] !== cleaningLastApplied[k])
  );
  let strategyOpen = $state(false);

  // —— v2.1 水印阈值/权重面板 ——
  const WM_DEFAULT = {
    auto_threshold: 0.70,
    suspect_threshold: 0.42,
    w_repeat: 0.40,
    w_non_cjk: 0.20,
    w_keyword: 0.40,
    repeat_count_min: 5,
    min_line_chars: 10,
    enabled: true,
  };
  let wmCfg = $state({ ...WM_DEFAULT });
  let wmLastApplied = $state({ ...WM_DEFAULT });
  const wmDirty = $derived(
    Object.keys(WM_DEFAULT).some((k) => wmCfg[k] !== wmLastApplied[k])
  );
  let wmThresholdOpen = $state(false);

  const anyDirty = $derived(cleaningDirty || wmDirty);
  let reanalyzing = $state(false);
  let reanalyzeError = $state("");

  async function reanalyze() {
    if (!pipeline.sourcePath) {
      reanalyzeError = "无源文件路径,无法重新分析";
      return;
    }
    const dropping = decisionCount();
    if (dropping > 0) {
      const ok = window.confirm(
        `重新分析会清空已有的 ${dropping} 条决策(接受 / 拒绝),确定继续?`,
      );
      if (!ok) return;
    }

    reanalyzing = true;
    reanalyzeError = "";
    setBusy(true);
    try {
      const dto = await loadAndAnalyze(
        pipeline.sourcePath,
        null,
        { ...cfg },
        { ...wmCfg },
      );
      setPipeline(dto, pipeline.sourcePath);
      cleaningLastApplied = { ...cfg };
      wmLastApplied = { ...wmCfg };
      clearAllDecisions();
    } catch (e) {
      reanalyzeError = String(e);
    } finally {
      reanalyzing = false;
      setBusy(false);
    }
  }

  function getCleaningDecision(span) { return getDecision("cleaning", span); }
  function getWmDecision(span) { return getDecision("watermark", span); }
  function toggleCleaningDecision(span, want) { toggleDecision("cleaning", span, want); }
  function toggleWmDecision(span, want) { toggleDecision("watermark", span, want); }
  function bulkApproveCleaning() { bulkSet("cleaning", cleaningItems.map((c) => c.span), "approved"); }
  function bulkRejectCleaning()  { bulkSet("cleaning", cleaningItems.map((c) => c.span), "rejected"); }
  function bulkClearCleaning()   { bulkClear("cleaning", cleaningItems.map((c) => c.span)); }
  function bulkApproveWm() { bulkSet("watermark", wmFiltered.map((w) => w.span), "approved"); }
  function bulkRejectWm()  { bulkSet("watermark", wmFiltered.map((w) => w.span), "rejected"); }
  function bulkClearWm()   { bulkClear("watermark", wmFiltered.map((w) => w.span)); }

  const WM_FIELDS = [
    { key: "auto_threshold", label: "auto 阈值", kind: "float", min: 0, max: 1, step: 0.05, hint: "≥ 此值自动删除(默认 0.70)" },
    { key: "suspect_threshold", label: "suspect 阈值", kind: "float", min: 0, max: 1, step: 0.01, hint: "≥ 此值进灰区(默认 0.42)" },
    { key: "w_repeat", label: "行频权重", kind: "float", min: 0, max: 1, step: 0.05, hint: "默认 0.40" },
    { key: "w_non_cjk", label: "非中文权重", kind: "float", min: 0, max: 1, step: 0.05, hint: "默认 0.20" },
    { key: "w_keyword", label: "关键词权重", kind: "float", min: 0, max: 1, step: 0.05, hint: "默认 0.40" },
    { key: "repeat_count_min", label: "行频最小次数", kind: "int", min: 1, max: 100, step: 1, hint: "默认 5" },
    { key: "min_line_chars", label: "短行豁免字符数", kind: "int", min: 1, max: 50, step: 1, hint: "默认 10" },
  ];

  const cleaningItems = $derived(
    (pipeline.dto?.cleaning ?? []).filter((c) => !c.kind.startsWith("watermark_"))
  );
  const cleaningCounts = $derived.by(() => {
    const m = {};
    for (const c of cleaningItems) m[c.kind] = (m[c.kind] ?? 0) + 1;
    return m;
  });
  const autoMirrorSpans = $derived(
    (pipeline.dto?.cleaning ?? [])
      .filter((c) => c.kind.startsWith("watermark_"))
      .map((c) => c.span)
  );

  const watermarkItems = $derived(pipeline.dto?.watermark ?? []);
  const wmCounts = $derived.by(() => {
    let auto = 0, suspect = 0;
    for (const w of watermarkItems) {
      if (w.verdict === "auto") auto += 1;
      else if (w.verdict === "suspect") suspect += 1;
    }
    return { auto, suspect, total: watermarkItems.length };
  });
  const suspectSpans = $derived(
    watermarkItems.filter((w) => w.verdict === "suspect").map((w) => w.span)
  );

  function spanMatches(targetMap, scope, span) {
    return targetMap[`${scope}:${span.start}-${span.end}`];
  }
  const effectiveCleaningRedSpans = $derived(
    cleaningItems
      .filter((c) => spanMatches(decisions.map, "cleaning", c.span) !== "rejected")
      .map((c) => c.span)
  );
  const effectiveAutoOrangeSpans = $derived.by(() => {
    const out = [];
    for (const span of autoMirrorSpans) {
      if (spanMatches(decisions.map, "watermark", span) !== "rejected") {
        out.push(span);
      }
    }
    for (const w of watermarkItems) {
      if (w.verdict === "suspect" && spanMatches(decisions.map, "watermark", w.span) === "approved") {
        out.push(w.span);
      }
    }
    return out;
  });
  const effectiveSuspectYellowSpans = $derived(
    suspectSpans.filter((span) => spanMatches(decisions.map, "watermark", span) !== "approved")
  );

  $effect(() => {
    if (!pipeline.dto) {
      clearLayers();
      return;
    }
    setLayers([
      {
        id: "cleaning_format",
        color: "var(--hl-cleaning)",
        className: "hl-cleaning",
        items: effectiveCleaningRedSpans.map((span) => ({ span })),
      },
      {
        id: "watermark_auto",
        color: "rgba(255, 140, 0, 0.55)",
        className: "hl-watermark-auto",
        items: effectiveAutoOrangeSpans.map((span) => ({ span })),
      },
      {
        id: "watermark_suspect",
        color: "rgba(245, 196, 0, 0.55)",
        className: "hl-watermark-suspect",
        items: effectiveSuspectYellowSpans.map((span) => ({ span })),
      },
    ]);
  });

  onDestroy(() => clearLayers());

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

  let wmTab = $state("all");
  let wmSelected = $state(-1);
  const WM_PAGE = 200;
  let wmVisible = $state(WM_PAGE);
  const wmFiltered = $derived.by(() => {
    if (wmTab === "auto") return watermarkItems.filter((w) => w.verdict === "auto");
    if (wmTab === "suspect") return watermarkItems.filter((w) => w.verdict === "suspect");
    return watermarkItems;
  });
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

  function formatScore(s) { return (s * 100).toFixed(0) + "%"; }

  let adjudicating = $state(false);
  let adjudicateError = $state("");
  let adjudicatingSpan = $state("");

  function applyAdjudicationResult(result) {
    if (!result || !pipeline.dto) return;
    for (const uw of result.updated_watermarks ?? []) {
      const idx = pipeline.dto.watermark.findIndex(
        (w) => w.span.start === uw.span.start && w.span.end === uw.span.end
      );
      if (idx >= 0) pipeline.dto.watermark[idx] = uw;
    }
    for (const nc of result.new_cleaning ?? []) {
      let pos = pipeline.dto.cleaning.findIndex((c) => c.span.start > nc.span.start);
      if (pos === -1) pos = pipeline.dto.cleaning.length;
      pipeline.dto.cleaning.splice(pos, 0, nc);
    }
  }

  async function onAdjudicateAll() {
    const suspects = watermarkItems.filter((w) => w.verdict === "suspect");
    if (suspects.length === 0) return;
    adjudicating = true;
    adjudicateError = "";
    try {
      const result = await adjudicateWatermarks(suspects.map((w) => w.span));
      applyAdjudicationResult(result);
    } catch (e) {
      adjudicateError = String(e);
    } finally {
      adjudicating = false;
    }
  }

  let inducing = $state(false);
  let inducedRule = $state(null);
  let induceError = $state("");
  let savingRule = $state(false);
  let savedMsg = $state("");

  function getRejectedWmSpans() {
    const spans = [];
    for (const [key, val] of Object.entries(decisions.map)) {
      if (key.startsWith("watermark:") && val === "rejected") {
        const raw = key.slice("watermark:".length);
        const dash = raw.indexOf("-");
        if (dash > 0) {
          spans.push({ start: parseInt(raw.slice(0, dash)), end: parseInt(raw.slice(dash + 1)) });
        }
      }
    }
    return spans;
  }

  const rejectedWmCount = $derived(
    Object.entries(decisions.map).filter(([k, v]) => k.startsWith("watermark:") && v === "rejected").length
  );

  async function onInduceRule() {
    const spans = getRejectedWmSpans();
    if (spans.length === 0) return;
    inducing = true;
    induceError = "";
    inducedRule = null;
    savedMsg = "";
    try {
      const rule = await induceWatermarkRule(spans);
      if (rule) inducedRule = rule;
      else induceError = "LLM 未能归纳出规则(样本可能不足或模式不明显)";
    } catch (e) {
      induceError = String(e);
    } finally {
      inducing = false;
    }
  }

  async function onSaveRule() {
    if (!inducedRule) return;
    savingRule = true;
    savedMsg = "";
    try {
      await saveInducedRule(inducedRule);
      savedMsg = "规则已保存,下次重新分析将自动应用";
    } catch (e) {
      induceError = String(e);
    } finally {
      savingRule = false;
    }
  }

  async function onAdjudicateOne(span) {
    const key = `${span.start}-${span.end}`;
    adjudicatingSpan = key;
    adjudicateError = "";
    try {
      const result = await adjudicateWatermarks([span]);
      applyAdjudicationResult(result);
    } catch (e) {
      adjudicateError = String(e);
    } finally {
      adjudicatingSpan = "";
    }
  }

  // 给 Slider 包装(它是 array 类型,我们用 single 模式)
  function sliderValue(key) {
    return wmCfg[key];
  }
</script>

<div class="flex flex-col gap-3 p-3">
  <h2 class="text-sm font-semibold">2. 文本处理</h2>

  {#if !pipeline.dto}
    <p class="text-xs text-muted-foreground">请先在阶段 1 加载文件。</p>
  {:else}
    <!-- ============ 清洗策略折叠面板 ============ -->
    <section class="overflow-hidden rounded-md border bg-card">
      <button
        type="button"
        class="flex w-full items-center gap-1.5 bg-muted/60 px-2.5 py-1.5 text-left text-[11px] hover:bg-muted"
        onclick={() => (strategyOpen = !strategyOpen)}
      >
        {#if strategyOpen}<ChevronDown class="size-3" />{:else}<ChevronRight class="size-3" />{/if}
        清洗策略
        <span class="ml-auto text-[10px] text-muted-foreground">
          {Object.values(cfg).filter(Boolean).length} / 5 启用
        </span>
        {#if cleaningDirty}
          <span class="size-1.5 rounded-full bg-amber-500 ring-2 ring-amber-500/25"></span>
        {/if}
      </button>
      {#if strategyOpen}
        <div class="flex flex-col gap-1.5 px-2.5 pb-2 pt-2">
          {#each Object.keys(CLEANING_DEFAULT) as k}
            <label class="flex cursor-pointer items-center gap-2 text-[11px]">
              <Checkbox bind:checked={cfg[k]} />
              <span class="min-w-[88px]">{KIND_LABEL[k] ?? k}</span>
              {#if k === "leading_fullwidth_space"}
                <span class="text-[10px] text-muted-foreground">中文段首习惯,默认保留</span>
              {/if}
            </label>
          {/each}
        </div>
      {/if}
    </section>

    <!-- ============ 水印阈值高级折叠面板 ============ -->
    <section class="overflow-hidden rounded-md border bg-card">
      <button
        type="button"
        class="flex w-full items-center gap-1.5 bg-muted/60 px-2.5 py-1.5 text-left text-[11px] hover:bg-muted"
        onclick={() => (wmThresholdOpen = !wmThresholdOpen)}
      >
        {#if wmThresholdOpen}<ChevronDown class="size-3" />{:else}<ChevronRight class="size-3" />{/if}
        水印阈值 <span class="text-[10px] text-muted-foreground">高级</span>
        {#if wmDirty}
          <span class="ml-auto size-1.5 rounded-full bg-amber-500 ring-2 ring-amber-500/25"></span>
        {/if}
      </button>
      {#if wmThresholdOpen}
        <div class="flex flex-col gap-2 px-2.5 pb-2 pt-2">
          <label class="flex cursor-pointer items-center gap-2 text-[11px]">
            <Checkbox bind:checked={wmCfg.enabled} />
            <span>启用水印检测</span>
            <span class="text-[10px] text-muted-foreground">关闭后跳过整段水印分析</span>
          </label>
          {#each WM_FIELDS as f}
            <div class="grid grid-cols-[100px_1fr_44px] items-center gap-1.5 text-[11px]">
              <span>{f.label}</span>
              <Slider
                value={sliderValue(f.key)}
                onValueChange={(v) => (wmCfg[f.key] = v)}
                min={f.min}
                max={f.max}
                step={f.step}
                disabled={!wmCfg.enabled}
              />
              <span class="text-right font-mono text-[10px]">
                {f.kind === "float" ? Number(wmCfg[f.key]).toFixed(2) : wmCfg[f.key]}
              </span>
              <span class="col-span-3 pl-[100px] text-[10px] text-muted-foreground">{f.hint}</span>
            </div>
          {/each}
        </div>
      {/if}
    </section>

    <!-- 重新分析 -->
    <div class="flex items-center gap-2">
      <Button
        class="flex-1"
        size="sm"
        variant={anyDirty ? "default" : "outline"}
        disabled={!anyDirty || reanalyzing}
        onclick={reanalyze}
      >
        {reanalyzing ? "分析中…" : anyDirty ? "重新分析" : "策略 / 阈值 无改动"}
      </Button>
    </div>
    {#if reanalyzeError}
      <Alert.Root variant="destructive"><Alert.Description>{reanalyzeError}</Alert.Description></Alert.Root>
    {/if}

    <!-- ============ 格式清洗 ============ -->
    <section class="flex flex-col gap-2 border-t border-dashed pt-3">
      <h3 class="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
        <span class="inline-block size-2 rounded-full bg-red-500/85"></span>
        格式清洗
        <span class="ml-auto text-[10px] font-normal normal-case text-muted-foreground/80">
          {cleaningItems.length} 条
        </span>
      </h3>

      {#if cleaningItems.length === 0}
        <p class="text-xs text-muted-foreground">无格式清洗 —— 文本已经很干净。</p>
      {:else}
        <div class="flex flex-wrap gap-1">
          {#each Object.entries(cleaningCounts) as [k, n]}
            <span class="rounded-full border bg-background px-2 py-0.5 text-[10px] text-muted-foreground">
              {KIND_LABEL[k] ?? k} <strong class="ml-0.5 text-red-700 dark:text-red-400">{n}</strong>
            </span>
          {/each}
        </div>

        <div class="flex gap-1">
          <Button variant="outline" size="sm" class="h-7 border-emerald-500/30 text-[10px] text-emerald-700 hover:bg-emerald-500/10 dark:text-emerald-400" onclick={bulkApproveCleaning}>✓ 全接受</Button>
          <Button variant="outline" size="sm" class="h-7 border-red-500/30 text-[10px] text-red-700 hover:bg-red-500/10 dark:text-red-400" onclick={bulkRejectCleaning}>✗ 全拒绝</Button>
          <Button variant="outline" size="sm" class="h-7 text-[10px]" onclick={bulkClearCleaning}>重置</Button>
        </div>

        <ul class="m-0 flex list-none flex-col gap-0.5 p-0">
          {#each cleaningItems.slice(0, cleaningVisible) as c, idx (idx)}
            {@const p = preview(c.span)}
            {@const d = getCleaningDecision(c.span)}
            <li>
              <div
                class={cn(
                  "grid grid-cols-[auto_1fr_auto_auto] select-none items-center gap-1.5 rounded border bg-card px-1.5 py-1 text-[11px] cursor-pointer transition-colors hover:bg-accent/60",
                  idx === cleaningSelected && "border-red-300 bg-red-500/5",
                  d === "approved" && "shadow-[inset_3px_0_0_oklch(0.7_0.17_148)]",
                  d === "rejected" && "shadow-[inset_3px_0_0_var(--muted-foreground)] opacity-55",
                )}
                role="button"
                tabindex="0"
                onclick={() => jumpCleaning(idx)}
                onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); jumpCleaning(idx); }}}
              >
                <span class="whitespace-nowrap rounded bg-red-500/15 px-1.5 py-0.5 text-[9px] text-red-700 dark:text-red-300">
                  {KIND_LABEL[c.kind] ?? c.kind}
                </span>
                <span class="min-w-0 truncate font-sans">
                  <span class="text-muted-foreground/60">{p.before}</span><mark class="bg-red-500/40 px-0.5 text-inherit rounded-sm">{p.target}</mark><span class="text-muted-foreground/60">{p.after}</span>
                </span>
                <span class="font-mono text-[9px] text-muted-foreground/70">[{c.span.start}–{c.span.end}]</span>
                <span class="inline-flex gap-0.5" role="presentation" onclick={(e) => e.stopPropagation()}>
                  <button
                    type="button"
                    class={cn(
                      "size-[18px] rounded border text-[11px] font-bold leading-none transition-colors",
                      d === "approved"
                        ? "border-emerald-600 bg-emerald-600 text-white"
                        : "border-input bg-background text-muted-foreground hover:bg-accent",
                    )}
                    onclick={() => toggleCleaningDecision(c.span, "approved")}
                    title="确认要删除(显式锁定)"
                  >✓</button>
                  <button
                    type="button"
                    class={cn(
                      "size-[18px] rounded border text-[11px] font-bold leading-none transition-colors",
                      d === "rejected"
                        ? "border-red-600 bg-red-600 text-white"
                        : "border-input bg-background text-muted-foreground hover:bg-accent",
                    )}
                    onclick={() => toggleCleaningDecision(c.span, "rejected")}
                    title="拒绝删除,保留该 span"
                  >✗</button>
                </span>
              </div>
            </li>
          {/each}
        </ul>

        {#if cleaningVisible < cleaningItems.length}
          <Button variant="outline" size="sm" class="w-full border-dashed text-[11px]" onclick={loadMoreCleaning}>
            再加载 {Math.min(CLEAN_PAGE, cleaningItems.length - cleaningVisible)} 条
            (已显示 {cleaningVisible} / {cleaningItems.length})
          </Button>
        {/if}
      {/if}
    </section>

    <!-- ============ 水印 ============ -->
    <section class="flex flex-col gap-2 border-t border-dashed pt-3">
      <h3 class="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
        <span class="inline-block size-2 rounded-full bg-orange-500/85"></span>
        <span class="inline-block size-2 rounded-full bg-yellow-400"></span>
        水印检测
        <span class="ml-auto text-[10px] font-normal normal-case text-muted-foreground/80">
          auto {wmCounts.auto} · suspect {wmCounts.suspect}
        </span>
      </h3>

      {#if wmCounts.total === 0}
        <p class="text-xs text-muted-foreground">未识别出水印 —— 整本文本干净或规则未覆盖到。</p>
      {:else}
        <div class="flex flex-wrap gap-1" role="tablist">
          <button
            type="button"
            class={cn(
              "flex items-center gap-1 rounded-full border px-2.5 py-0.5 text-[10px]",
              wmTab === "all" ? "border-primary bg-primary text-primary-foreground" : "border-input bg-background text-muted-foreground hover:bg-accent",
            )}
            onclick={() => (wmTab = "all")}
          >全部 <small class="opacity-75">{wmCounts.total}</small></button>
          <button
            type="button"
            class={cn(
              "flex items-center gap-1 rounded-full border px-2.5 py-0.5 text-[10px]",
              wmTab === "auto" ? "border-primary bg-primary text-primary-foreground" : "border-input bg-background text-muted-foreground hover:bg-accent",
            )}
            onclick={() => (wmTab = "auto")}
          ><span class="size-1.5 rounded-full bg-orange-500"></span>自动 <small class="opacity-75">{wmCounts.auto}</small></button>
          <button
            type="button"
            class={cn(
              "flex items-center gap-1 rounded-full border px-2.5 py-0.5 text-[10px]",
              wmTab === "suspect" ? "border-primary bg-primary text-primary-foreground" : "border-input bg-background text-muted-foreground hover:bg-accent",
            )}
            onclick={() => (wmTab = "suspect")}
          ><span class="size-1.5 rounded-full bg-yellow-400"></span>灰区 <small class="opacity-75">{wmCounts.suspect}</small></button>
          {#if llm.configured && wmCounts.suspect > 0}
            <button
              type="button"
              class="ml-auto flex items-center gap-1 rounded-full border border-violet-500/30 bg-violet-500/15 px-2.5 py-0.5 text-[10px] text-violet-700 hover:bg-violet-500/25 disabled:opacity-55 dark:text-violet-300"
              disabled={adjudicating}
              onclick={onAdjudicateAll}
              title="把所有灰区候选批量发给 LLM 仲裁"
            >{adjudicating ? "仲裁中…" : "询问 LLM ▸"}</button>
          {/if}
        </div>
        {#if adjudicateError}
          <p class="text-[10px] text-destructive">{adjudicateError}</p>
        {/if}

        {#if wmFiltered.length === 0}
          <p class="text-xs text-muted-foreground">当前 tab 下无数据。</p>
        {:else}
          <div class="flex gap-1">
            <Button variant="outline" size="sm" class="h-7 border-emerald-500/30 text-[10px] text-emerald-700 hover:bg-emerald-500/10 dark:text-emerald-400" onclick={bulkApproveWm}>✓ 全接受</Button>
            <Button variant="outline" size="sm" class="h-7 border-red-500/30 text-[10px] text-red-700 hover:bg-red-500/10 dark:text-red-400" onclick={bulkRejectWm}>✗ 全拒绝</Button>
            <Button variant="outline" size="sm" class="h-7 text-[10px]" onclick={bulkClearWm}>重置</Button>
          </div>

          <ul class="m-0 flex list-none flex-col gap-0.5 p-0">
            {#each wmFiltered.slice(0, wmVisible) as w, idx (`${w.span.start}-${w.span.end}-${idx}`)}
              {@const p = preview(w.span)}
              {@const d = getWmDecision(w.span)}
              <li>
                <div
                  class={cn(
                    "flex select-none cursor-pointer flex-col gap-1 rounded border bg-card p-1.5 text-[11px] transition-colors hover:bg-accent/60",
                    idx === wmSelected && "border-amber-300 bg-amber-500/5",
                    d === "approved" && "shadow-[inset_3px_0_0_oklch(0.7_0.17_148)]",
                    d === "rejected" && "shadow-[inset_3px_0_0_var(--muted-foreground)] opacity-55",
                  )}
                  role="button"
                  tabindex="0"
                  onclick={() => jumpWm(idx)}
                  onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); jumpWm(idx); }}}
                >
                  <div class="flex items-center gap-1.5">
                    <span class={cn(
                      "rounded-full px-1.5 py-0.5 text-[9px] font-semibold",
                      w.verdict === "auto" ? "bg-orange-500/20 text-orange-700 dark:text-orange-300" : "bg-yellow-400/25 text-yellow-700 dark:text-yellow-300",
                    )}>
                      {w.verdict === "auto" ? "自动" : "灰区"}
                    </span>
                    <span class="font-mono text-[9px] text-muted-foreground">分 {formatScore(w.score)}</span>
                    <span class="font-mono text-[9px] text-muted-foreground/70">[{w.span.start}–{w.span.end}]</span>
                    <span class="ml-auto inline-flex gap-0.5" role="presentation" onclick={(e) => e.stopPropagation()}>
                      {#if llm.configured && w.verdict === "suspect"}
                        {@const spanKey = `${w.span.start}-${w.span.end}`}
                        <button
                          type="button"
                          class="size-[18px] rounded border border-violet-500/30 bg-background text-[10px] font-bold leading-none text-violet-700 hover:bg-violet-500/15 disabled:opacity-55 dark:text-violet-300"
                          disabled={adjudicatingSpan === spanKey || adjudicating}
                          onclick={() => onAdjudicateOne(w.span)}
                          title="询问 LLM 判断这行是否为水印"
                        >{adjudicatingSpan === spanKey ? "…" : "?"}</button>
                      {/if}
                      <button
                        type="button"
                        class={cn(
                          "size-[18px] rounded border text-[11px] font-bold leading-none transition-colors",
                          d === "approved" ? "border-emerald-600 bg-emerald-600 text-white" : "border-input bg-background text-muted-foreground hover:bg-accent",
                        )}
                        onclick={() => toggleWmDecision(w.span, "approved")}
                        title={w.verdict === "auto" ? "确认删(锁定默认)" : "升级:从灰区扣除该行"}
                      >✓</button>
                      <button
                        type="button"
                        class={cn(
                          "size-[18px] rounded border text-[11px] font-bold leading-none transition-colors",
                          d === "rejected" ? "border-red-600 bg-red-600 text-white" : "border-input bg-background text-muted-foreground hover:bg-accent",
                        )}
                        onclick={() => toggleWmDecision(w.span, "rejected")}
                        title={w.verdict === "auto" ? "拒绝删:auto 行将保留在 EPUB" : "锁定保留(默认就是保留)"}
                      >✗</button>
                    </span>
                  </div>
                  <span class="overflow-hidden truncate font-sans text-[11px]">
                    <span class="text-muted-foreground/60">{p.before}</span><mark class={cn(
                      "px-0.5 text-inherit rounded-sm",
                      w.verdict === "auto" ? "bg-orange-500/40" : "bg-yellow-400/50",
                    )}>{p.target}</mark><span class="text-muted-foreground/60">{p.after}</span>
                  </span>
                  <div class="flex flex-col gap-0.5">
                    {#each w.signals as s}
                      <div class="grid grid-cols-[40px_60px_1fr] items-center gap-1.5 text-[9px] text-muted-foreground">
                        <span class="font-semibold text-foreground">{SIGNAL_LABEL[s.kind] ?? s.kind}</span>
                        <span class="inline-block h-1 overflow-hidden rounded-sm bg-muted">
                          <span class="block h-full bg-orange-500/70" style="width:{Math.round(s.score * 100)}%"></span>
                        </span>
                        {#if s.detail}<span class="truncate" title={s.detail}>{s.detail}</span>{/if}
                      </div>
                    {/each}
                  </div>
                </div>
              </li>
            {/each}
          </ul>

          {#if wmVisible < wmFiltered.length}
            <Button variant="outline" size="sm" class="w-full border-dashed text-[11px]" onclick={loadMoreWm}>
              再加载 {Math.min(WM_PAGE, wmFiltered.length - wmVisible)} 条
              (已显示 {wmVisible} / {wmFiltered.length})
            </Button>
          {/if}
        {/if}

        {#if llm.configured}
          <div class="mt-2 flex items-center gap-2 border-t border-dashed pt-2">
            <Button
              size="sm"
              variant="outline"
              class="border-violet-500/30 bg-violet-500/10 text-[10px] text-violet-700 hover:bg-violet-500/20 dark:text-violet-300"
              disabled={rejectedWmCount === 0 || inducing}
              onclick={onInduceRule}
              title={rejectedWmCount === 0 ? "先拒绝(✗)一些灰区候选行" : `从 ${rejectedWmCount} 条拒绝行归纳规则`}
            >
              {inducing ? "归纳中…" : "归纳规则 ▸"}
              {#if rejectedWmCount > 0}<small class="text-violet-500">({rejectedWmCount} 条)</small>{/if}
            </Button>
            {#if induceError}<span class="text-[10px] text-destructive">{induceError}</span>{/if}
          </div>

          {#if inducedRule}
            <div class="mt-2 rounded-md border border-violet-500/30 bg-violet-500/5 p-2 text-[11px]">
              <div class="mb-1.5 flex items-center justify-between">
                <span class="text-[10px] font-semibold uppercase tracking-wide text-violet-700 dark:text-violet-300">LLM 归纳规则</span>
                <button
                  type="button"
                  class="text-muted-foreground hover:text-foreground"
                  onclick={() => { inducedRule = null; savedMsg = ""; }}
                ><X class="size-3" /></button>
              </div>
              <div class="mb-1 break-all rounded border bg-background px-1.5 py-1 font-mono text-[10px]">
                {inducedRule.pattern}
              </div>
              {#if inducedRule.description}
                <div class="mb-1.5 text-[10px] text-muted-foreground">{inducedRule.description}</div>
              {/if}
              {#if savedMsg}
                <div class="text-[10px] text-emerald-700 dark:text-emerald-400">{savedMsg}</div>
              {:else}
                <Button
                  size="sm"
                  class="bg-violet-600 text-[10px] text-white hover:bg-violet-700"
                  disabled={savingRule}
                  onclick={onSaveRule}
                >{savingRule ? "保存中…" : "保存规则"}</Button>
              {/if}
            </div>
          {/if}
        {/if}
      {/if}
    </section>
  {/if}
</div>
