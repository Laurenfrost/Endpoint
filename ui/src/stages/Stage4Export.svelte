<script>
  // 阶段 4:样式预览与导出。封面 + CSS + 字体嵌入 + 元数据。
  import CheckCircle2 from "@lucide/svelte/icons/check-circle-2";
  import AlertCircle from "@lucide/svelte/icons/alert-circle";
  import { pipeline } from "../stores/pipeline.svelte.js";
  import { progress, setBusy } from "../stores/progress.svelte.js";
  import { openSettings } from "../stores/stage.svelte.js";
  import {
    pickOutputFile, pickCoverFile, pickFontFile,
    buildEpub, listThemes, loadTheme, generateTextCover,
    getLlmConfig, suggestMetadata,
    getKepubifyConfig,
  } from "../ipc.js";
  import { llm, applyLlmConfig } from "../stores/llm.svelte.js";
  import { serializeForIpc, decisionCount } from "../stores/decisions.svelte.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Textarea } from "$lib/components/ui/textarea/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import { Checkbox } from "$lib/components/ui/checkbox/index.js";
  import * as Alert from "$lib/components/ui/alert/index.js";
  import { cn } from "$lib/utils.js";

  const THEME_LABELS = { easypub: "EasyPub", standard: "标准", classic: "古风", highcontrast: "高对比度" };

  let outputPath = $state("");
  let title = $state("");
  let author = $state("");
  let description = $state("");
  let subjectsText = $state("");
  let series = $state("");
  let seriesIndexText = $state("");
  let coverMode = $state("none");
  let coverPath = $state("");
  let coverDataUrl = $state("");
  let textCoverStyle = $state("default");
  let generatingCover = $state(false);
  let embedFonts = $state(false);
  let fontSource = $state("builtin");
  let customFontPath = $state("");
  let themes = $state([]);
  let selectedTheme = $state("");
  let cssText = $state("");
  let cssExpanded = $state(false);
  let loadingTheme = $state(false);
  let suggesting = $state(false);
  let suggestion = $state(null);
  let suggestionMsg = $state("");
  let kepubifyPath = $state("");
  let kepubifyEnabled = $state(false);
  let error = $state("");
  let result = $state("");

  $effect(() => {
    if (pipeline.sourcePath && !outputPath) {
      outputPath = pipeline.sourcePath.replace(/\.txt$/i, "") + ".epub";
    }
  });

  $effect(() => {
    if (pipeline.dto && themes.length === 0) {
      listThemes().then(list => {
        themes = list;
        if (list.includes("easypub")) {
          onSelectTheme("easypub");
        } else if (list.includes("standard")) {
          onSelectTheme("standard");
        }
      }).catch(() => {});
    }
  });

  $effect(() => {
    getLlmConfig().then(applyLlmConfig).catch(() => {});
  });

  $effect(() => {
    getKepubifyConfig().then(cfg => {
      kepubifyPath = cfg.path ?? "";
      kepubifyEnabled = !!cfg.enabled;
    }).catch(() => {});
  });

  async function onSelectTheme(name) {
    if (name === selectedTheme) return;
    loadingTheme = true;
    try {
      const css = await loadTheme(name);
      cssText = css;
      selectedTheme = name;
    } catch (e) {
      error = String(e);
    } finally {
      loadingTheme = false;
    }
  }

  function onCssInput() { selectedTheme = ""; }

  async function onPickOutput() {
    try {
      const p = await pickOutputFile(outputPath || null);
      if (typeof p === "string") outputPath = p;
    } catch (e) { error = String(e); }
  }

  async function onPickCover() {
    try {
      const res = await pickCoverFile();
      if (res && res.path) { coverPath = res.path; coverDataUrl = res.dataUrl; }
    } catch (e) { error = String(e); }
  }

  function onClearCover() { coverPath = ""; coverDataUrl = ""; }

  async function onGenerateTextCover() {
    if (!title || !author) return;
    generatingCover = true;
    error = "";
    try {
      const fontPath = embedFonts && fontSource === "custom" ? customFontPath : null;
      const res = await generateTextCover(title, author, textCoverStyle, fontPath);
      if (res && res.path) {
        coverPath = res.path;
        coverDataUrl = res.dataUrl;
      }
    } catch (e) {
      error = String(e);
    } finally {
      generatingCover = false;
    }
  }

  async function onPickCustomFont() {
    try {
      const p = await pickFontFile();
      if (typeof p === "string") customFontPath = p;
    } catch (e) { error = String(e); }
  }

  async function onSuggestMetadata() {
    suggesting = true;
    suggestion = null;
    suggestionMsg = "";
    try {
      const r = await suggestMetadata();
      if (!r) {
        suggestionMsg = llm.configured ? "LLM 无法从正文推断元数据" : "请先在 ⚙ 设置中配置 LLM";
      } else {
        suggestion = r;
      }
    } catch (e) {
      suggestionMsg = String(e);
    } finally {
      suggesting = false;
    }
  }

  function applySuggestionField(field) {
    if (field === "title" && suggestion?.title) title = suggestion.title;
    if (field === "author" && suggestion?.author) author = suggestion.author;
    if (field === "description" && suggestion?.description) description = suggestion.description;
    if (field === "subjects" && suggestion?.subjects?.length)
      subjectsText = suggestion.subjects.join("、");
    if (field === "series" && suggestion?.series) {
      series = suggestion.series;
      if (suggestion.series_index != null) seriesIndexText = String(suggestion.series_index);
    }
  }

  function applyAllSuggestions() {
    if (!suggestion) return;
    if (suggestion.title) title = suggestion.title;
    if (suggestion.author) author = suggestion.author;
    if (suggestion.description) description = suggestion.description;
    if (suggestion.subjects?.length) subjectsText = suggestion.subjects.join("、");
    if (suggestion.series) {
      series = suggestion.series;
      if (suggestion.series_index != null) seriesIndexText = String(suggestion.series_index);
    }
  }

  function dismissSuggestion() {
    suggestion = null;
    suggestionMsg = "";
  }

  async function onBuild() {
    if (!outputPath) return (error = "请先选择输出位置");
    if (!title || !author) return (error = "请填写书名与作者");
    if (embedFonts && fontSource === "custom" && !customFontPath)
      return (error = "已选自定义字体但未选择字体文件");
    error = "";
    result = "";
    setBusy(true);
    try {
      const decisions = serializeForIpc();
      const subjectsArr = subjectsText
        .split(/[,，、;；]/)
        .map(s => s.trim())
        .filter((s, i, arr) => s && arr.indexOf(s) === i);
      const idxParsed = parseInt(seriesIndexText, 10);
      const seriesIdx = Number.isFinite(idxParsed) && idxParsed > 0 ? idxParsed : null;
      const finalPath = await buildEpub({
        outputPath,
        title,
        author,
        kepubifyPath: (kepubifyEnabled && kepubifyPath) ? kepubifyPath : null,
        decisions: decisions.length > 0 ? decisions : null,
        coverPath: coverPath || null,
        cssOverride: cssText || null,
        embedFonts,
        fontPath: embedFonts && fontSource === "custom" ? customFontPath : null,
        description: description.trim() || null,
        subjects: subjectsArr.length > 0 ? subjectsArr : null,
        series: series.trim() || null,
        seriesIndex: series.trim() ? seriesIdx : null,
      });
      result = finalPath;
    } catch (e) {
      error = String(e);
    } finally {
      setBusy(false);
    }
  }
</script>

<div class="flex flex-col gap-3 p-4">
  <h2 class="text-sm font-semibold">4. 样式预览与导出</h2>

  {#if !pipeline.dto}
    <p class="text-xs text-muted-foreground">请先在阶段 1 加载文件。</p>
  {:else}
    <div class="flex flex-col gap-1.5">
      <Label for="output-path">输出 epub</Label>
      <div class="flex gap-1.5">
        <Input id="output-path" bind:value={outputPath} readonly placeholder="点击选择" />
        <Button variant="outline" size="sm" onclick={onPickOutput} disabled={progress.busy}>选择</Button>
      </div>
    </div>

    <div class="flex flex-col gap-2.5">
      <div class="flex flex-col gap-1.5">
        <Label for="title">书名 *</Label>
        <Input id="title" bind:value={title} disabled={progress.busy} />
      </div>
      <div class="flex flex-col gap-1.5">
        <Label for="author">作者 *</Label>
        <Input id="author" bind:value={author} disabled={progress.busy} />
      </div>
      <div class="flex flex-col gap-1.5">
        <Label for="desc">简介(写入 EPUB 的 <code class="text-[10px]">dc:description</code>)</Label>
        <Textarea id="desc" rows="3" bind:value={description} disabled={progress.busy}
          placeholder="留空则 EPUB 不写简介字段" />
      </div>
      <div class="flex flex-col gap-1.5">
        <Label for="subjects">分类标签(逗号或顿号分隔,例:玄幻、修真)</Label>
        <Input id="subjects" bind:value={subjectsText} disabled={progress.busy}
          placeholder="留空则 EPUB 不写 dc:subject" />
      </div>
      <div class="flex gap-2">
        <div class="flex flex-1 flex-col gap-1.5">
          <Label for="series">系列名</Label>
          <Input id="series" bind:value={series} disabled={progress.busy}
            placeholder="独立作品留空" />
        </div>
        <div class="flex w-24 flex-col gap-1.5">
          <Label for="series-idx">系列序号</Label>
          <Input id="series-idx" type="number" bind:value={seriesIndexText} disabled={progress.busy}
            min="1" placeholder="如 1, 2" />
        </div>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <Button
          variant="outline"
          size="sm"
          class="border-primary text-[11px] text-primary hover:bg-primary/10"
          onclick={onSuggestMetadata}
          disabled={progress.busy || suggesting || !pipeline.dto || !llm.configured}
          title={llm.configured
            ? (llm.searchConfigured
                ? "Pass A:LLM 训练知识 → Pass B(必要时):Brave 搜索补全"
                : "Pass A:LLM 训练知识(未配 Brave,冷门作品可能识别不到)")
            : "请先在 ⚙ 设置中配置 LLM"}
        >
          {suggesting ? "推断中…" : "从正文建议 ▸"}
        </Button>
        {#if llm.searchConfigured}
          <span class="text-[11px] font-medium text-emerald-700 dark:text-emerald-400">+ Brave 搜索兜底</span>
        {/if}
        {#if !llm.configured}
          <button type="button" class="text-[11px] text-primary hover:underline" onclick={openSettings}>去配置 →</button>
        {/if}
        {#if suggestionMsg}
          <span class="text-[11px] text-muted-foreground">{suggestionMsg}</span>
        {/if}
      </div>

      {#if suggestion}
        <div class="rounded-md border border-primary/30 bg-primary/5 p-2 text-xs">
          <div class="mb-1.5 flex items-center justify-between">
            <span class="font-semibold text-primary">LLM 建议</span>
            <div class="flex gap-1">
              <button type="button" class="text-[11px] text-primary hover:underline" onclick={applyAllSuggestions}>全部采用</button>
              <button type="button" class="text-[11px] text-muted-foreground hover:underline" onclick={dismissSuggestion}>关闭 ✕</button>
            </div>
          </div>
          {#each [
            ["title", "书名", suggestion.title],
            ["author", "作者", suggestion.author],
            ["description", "简介", suggestion.description],
            ["subjects", "分类", suggestion.subjects?.length ? suggestion.subjects.join("、") : ""],
            ["series", "系列", suggestion.series ? (suggestion.series + (suggestion.series_index != null ? ` · 第 ${suggestion.series_index} 部` : "")) : ""],
          ] as [field, label, value]}
            {#if value}
              <div class={cn("flex flex-wrap items-baseline gap-2 mb-1", field === "description" && "items-start")}>
                <span class="min-w-[60px] shrink-0 text-muted-foreground">{label}</span>
                <span class={cn("flex-1 break-all", field === "description" && "whitespace-pre-wrap")}>{value}</span>
                <button
                  type="button"
                  class="shrink-0 rounded border border-primary px-1.5 py-0.5 text-[10px] text-primary hover:bg-primary/10"
                  onclick={() => applySuggestionField(field)}
                >采用</button>
              </div>
            {/if}
          {/each}
          {#if suggestion.cover_keywords}
            <div class="flex flex-wrap items-baseline gap-2">
              <span class="min-w-[60px] shrink-0 text-muted-foreground">封面关键词</span>
              <span class="flex-1 italic text-muted-foreground">{suggestion.cover_keywords}(仅参考)</span>
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <!-- 封面 -->
    <div class="flex flex-col gap-1.5">
      <Label>封面(可选)</Label>
      <div class="flex flex-wrap gap-3">
        {#each [["none", "无封面"], ["file", "图片文件"], ["text", "文字封面"]] as [val, lbl]}
          <label class="flex cursor-pointer items-center gap-1.5 text-xs">
            <input type="radio" name="cover-mode" value={val}
              checked={coverMode === val}
              onchange={() => { coverMode = val; coverPath = ""; coverDataUrl = ""; }} />
            {lbl}
          </label>
        {/each}
      </div>

      {#if coverMode === "file"}
        <div class="flex flex-col gap-1.5 rounded-md border bg-muted/40 p-2">
          <div class="flex gap-1.5">
            <Input bind:value={coverPath} readonly placeholder="选择图片文件" />
            <Button variant="outline" size="sm" onclick={onPickCover} disabled={progress.busy}>选择</Button>
            {#if coverPath}
              <Button variant="outline" size="sm" onclick={onClearCover} disabled={progress.busy} title="清除">✕</Button>
            {/if}
          </div>
          {#if coverDataUrl}
            <img class="block max-h-44 max-w-full rounded border object-contain" src={coverDataUrl} alt="封面预览" />
          {/if}
        </div>
      {:else if coverMode === "text"}
        <div class="flex flex-col gap-1.5 rounded-md border bg-muted/40 p-2">
          <div class="flex flex-wrap items-center gap-2">
            <label class="flex cursor-pointer items-center gap-1 text-xs">
              <input type="radio" bind:group={textCoverStyle} value="default"
                disabled={progress.busy || generatingCover} />
              深蓝
            </label>
            <label class="flex cursor-pointer items-center gap-1 text-xs">
              <input type="radio" bind:group={textCoverStyle} value="gradient"
                disabled={progress.busy || generatingCover} />
              蓝紫
            </label>
            <Button
              variant="outline"
              size="sm"
              onclick={onGenerateTextCover}
              disabled={progress.busy || generatingCover || !title || !author}
            >{generatingCover ? "生成中…" : "生成预览"}</Button>
          </div>
          {#if !title || !author}
            <p class="text-[11px] text-muted-foreground">请先填写书名与作者，再生成封面</p>
          {/if}
          {#if coverDataUrl}
            <img class="block max-h-44 max-w-full rounded border object-contain" src={coverDataUrl} alt="封面预览" />
          {/if}
        </div>
      {/if}
    </div>

    <!-- 字体嵌入 -->
    <div class="flex flex-col gap-1.5">
      <label class="flex cursor-pointer items-center gap-2 text-xs">
        <Checkbox bind:checked={embedFonts} disabled={progress.busy} />
        嵌入中文字体(约 +16 MB)
      </label>
      {#if embedFonts}
        <div class="flex flex-col gap-1.5 rounded-md border bg-muted/40 p-2">
          <label class="flex cursor-pointer items-center gap-1.5 text-xs">
            <input type="radio" bind:group={fontSource} value="builtin" disabled={progress.busy} />
            霞鹜文楷(内置)
          </label>
          <label class="flex cursor-pointer items-center gap-1.5 text-xs">
            <input type="radio" bind:group={fontSource} value="custom" disabled={progress.busy} />
            自定义字体文件
          </label>
          {#if fontSource === "custom"}
            <div class="mt-1 flex gap-1.5">
              <Input bind:value={customFontPath} readonly placeholder="选择 .ttf / .otf" />
              <Button variant="outline" size="sm" onclick={onPickCustomFont} disabled={progress.busy}>选择</Button>
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <!-- CSS 主题 -->
    <div class="flex flex-col gap-1.5">
      <div class="flex items-center justify-between">
        <Label>样式主题</Label>
        <button type="button" class="text-[11px] text-primary hover:underline" onclick={() => cssExpanded = !cssExpanded}>
          {cssExpanded ? "收起 CSS ▲" : "编辑 CSS ▼"}
        </button>
      </div>
      <div class="flex flex-wrap items-center gap-1">
        {#each themes as t}
          <button
            type="button"
            class={cn(
              "rounded-full border px-2.5 py-1 text-[11px] transition-colors",
              selectedTheme === t
                ? "border-primary bg-primary text-primary-foreground"
                : "border-input bg-background text-muted-foreground hover:bg-accent",
            )}
            disabled={progress.busy || loadingTheme}
            onclick={() => onSelectTheme(t)}
          >{THEME_LABELS[t] ?? t}</button>
        {/each}
        {#if selectedTheme === ""}
          <span class="rounded-full border border-dashed px-2 py-1 text-[11px] text-muted-foreground">自定义</span>
        {/if}
      </div>
      {#if cssExpanded}
        <Textarea
          class="mt-1 font-mono text-[11px] leading-5"
          bind:value={cssText}
          oninput={onCssInput}
          disabled={progress.busy}
          spellcheck={false}
          rows={14}
        />
      {/if}
    </div>

    <!-- 生成 -->
    <Button class="w-full" onclick={onBuild} disabled={progress.busy}>
      {progress.busy ? "生成中…" : "生成 EPUB"}
    </Button>

    <div class="flex flex-col gap-1">
      {#if kepubifyEnabled && kepubifyPath}
        <p class="text-[11px] text-emerald-700 dark:text-emerald-400">✓ 将额外生成 .kepub.epub</p>
      {:else}
        <p class="text-[11px] text-muted-foreground">
          只生成标准 .epub。
          <button type="button" class="text-primary hover:underline" onclick={openSettings}>在设置中启用 kepubify →</button>
        </p>
      {/if}
      {#if decisionCount() > 0}
        <p class="text-[11px] text-muted-foreground">将随生成应用 <strong>{decisionCount()}</strong> 条用户决策。</p>
      {/if}
    </div>

    {#if error}
      <Alert.Root variant="destructive">
        <AlertCircle />
        <Alert.Description>{error}</Alert.Description>
      </Alert.Root>
    {/if}
    {#if result}
      <Alert.Root variant="info" class="border-emerald-500/50 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300 [&>svg]:text-emerald-700 dark:[&>svg]:text-emerald-300">
        <CheckCircle2 />
        <Alert.Description><span class="font-mono">{result}</span></Alert.Description>
      </Alert.Root>
    {/if}
  {/if}
</div>
