<script>
  // 阶段 4:样式预览与导出。封面 + CSS + 字体嵌入 + 元数据。
  // LLM / 搜索 / kepubify 配置已搬到 ⚙ 设置面板;本阶段只读取持久化的 kepubify 配置并用于本次生成。
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

  const THEME_LABELS = { easypub: "EasyPub", standard: "标准", classic: "古风", highcontrast: "高对比度" };

  let outputPath = $state("");
  let title = $state("");
  let author = $state("");
  // 扩展元数据
  let description = $state("");
  let subjectsText = $state("");
  let series = $state("");
  let seriesIndexText = $state("");
  // 封面
  let coverMode = $state("none");       // "none" | "file" | "text"
  let coverPath = $state("");
  let coverDataUrl = $state("");
  let textCoverStyle = $state("default");
  let generatingCover = $state(false);
  // 字体
  let embedFonts = $state(false);
  let fontSource = $state("builtin");
  let customFontPath = $state("");
  // CSS 主题
  let themes = $state([]);
  let selectedTheme = $state("");
  let cssText = $state("");
  let cssExpanded = $state(false);
  let loadingTheme = $state(false);
  // LLM 元数据建议
  let suggesting = $state(false);
  let suggestion = $state(null);
  let suggestionMsg = $state("");
  // kepubify(只读,从持久化配置加载)
  let kepubifyPath = $state("");
  let kepubifyEnabled = $state(false);
  // 结果
  let error = $state("");
  let result = $state("");

  // 输出路径默认推导自源文件
  $effect(() => {
    if (pipeline.sourcePath && !outputPath) {
      outputPath = pipeline.sourcePath.replace(/\.txt$/i, "") + ".epub";
    }
  });

  // 加载主题列表(仅一次)
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

  // 进入阶段时同步 LLM 状态(供"从正文建议"按钮显示状态)
  $effect(() => {
    getLlmConfig().then(applyLlmConfig).catch(() => {});
  });

  // 进入阶段时读取持久化的 kepubify 配置
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

  function onCssInput() {
    selectedTheme = "";
  }

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

<div class="panel">
  <h2>4. 样式预览与导出</h2>

  {#if !pipeline.dto}
    <p class="hint">请先在阶段 1 加载文件。</p>
  {:else}
    <label>
      <span>输出 epub</span>
      <div class="row">
        <input type="text" bind:value={outputPath} readonly placeholder="点击选择" />
        <button onclick={onPickOutput} disabled={progress.busy}>选择</button>
      </div>
    </label>

    <div class="meta-group">
      <label>
        <span>书名 *</span>
        <input type="text" bind:value={title} disabled={progress.busy} />
      </label>
      <label>
        <span>作者 *</span>
        <input type="text" bind:value={author} disabled={progress.busy} />
      </label>
      <label>
        <span>简介(写入 EPUB 的 <code>dc:description</code>)</span>
        <textarea rows="3" bind:value={description} disabled={progress.busy}
          placeholder="留空则 EPUB 不写简介字段"></textarea>
      </label>
      <label>
        <span>分类标签(逗号或顿号分隔,例:玄幻、修真)</span>
        <input type="text" bind:value={subjectsText} disabled={progress.busy}
          placeholder="留空则 EPUB 不写 dc:subject" />
      </label>
      <div class="row series-row">
        <label class="series-name">
          <span>系列名</span>
          <input type="text" bind:value={series} disabled={progress.busy}
            placeholder="独立作品留空" />
        </label>
        <label class="series-idx">
          <span>系列序号</span>
          <input type="number" bind:value={seriesIndexText} disabled={progress.busy}
            min="1" placeholder="如 1, 2" />
        </label>
      </div>

      <div class="suggest-row">
        <button
          class="suggest-btn"
          onclick={onSuggestMetadata}
          disabled={progress.busy || suggesting || !pipeline.dto || !llm.configured}
          title={llm.configured
            ? (llm.searchConfigured
                ? "Pass A:LLM 训练知识 → Pass B(必要时):Brave 搜索补全"
                : "Pass A:LLM 训练知识(未配 Brave,冷门作品可能识别不到)")
            : "请先在 ⚙ 设置中配置 LLM"}
        >
          {suggesting ? "推断中..." : "从正文建议 ▸"}
        </button>
        {#if llm.searchConfigured}
          <span class="hint search-on">+ Brave 搜索兜底</span>
        {/if}
        {#if !llm.configured}
          <button class="link-btn" onclick={openSettings}>去配置 →</button>
        {/if}
        {#if suggestionMsg}<span class="hint suggest-hint">{suggestionMsg}</span>{/if}
      </div>

      {#if suggestion}
        <div class="suggestion-panel">
          <div class="suggestion-header">
            <span class="suggestion-title">LLM 建议</span>
            <button class="link-btn" onclick={applyAllSuggestions}>全部采用</button>
            <button class="link-btn" onclick={dismissSuggestion}>关闭 ✕</button>
          </div>
          {#if suggestion.title}
            <div class="suggestion-row">
              <span class="sug-label">书名</span>
              <span class="sug-value">{suggestion.title}</span>
              <button class="sug-apply" onclick={() => applySuggestionField("title")}>采用</button>
            </div>
          {/if}
          {#if suggestion.author}
            <div class="suggestion-row">
              <span class="sug-label">作者</span>
              <span class="sug-value">{suggestion.author}</span>
              <button class="sug-apply" onclick={() => applySuggestionField("author")}>采用</button>
            </div>
          {/if}
          {#if suggestion.description}
            <div class="suggestion-row suggestion-desc">
              <span class="sug-label">简介</span>
              <span class="sug-value">{suggestion.description}</span>
              <button class="sug-apply" onclick={() => applySuggestionField("description")}>采用</button>
            </div>
          {/if}
          {#if suggestion.subjects?.length}
            <div class="suggestion-row">
              <span class="sug-label">分类</span>
              <span class="sug-value">{suggestion.subjects.join("、")}</span>
              <button class="sug-apply" onclick={() => applySuggestionField("subjects")}>采用</button>
            </div>
          {/if}
          {#if suggestion.series}
            <div class="suggestion-row">
              <span class="sug-label">系列</span>
              <span class="sug-value">
                {suggestion.series}
                {#if suggestion.series_index != null} · 第 {suggestion.series_index} 部{/if}
              </span>
              <button class="sug-apply" onclick={() => applySuggestionField("series")}>采用</button>
            </div>
          {/if}
          {#if suggestion.cover_keywords}
            <div class="suggestion-row">
              <span class="sug-label">封面关键词</span>
              <span class="sug-value sug-hint">{suggestion.cover_keywords}(仅参考)</span>
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <!-- 封面 -->
    <div class="section">
      <div class="section-header">
        <span class="section-title">封面(可选)</span>
      </div>
      <div class="cover-mode-row">
        <label class="radio-label">
          <input type="radio" name="cover-mode" value="none"
            checked={coverMode === "none"}
            onchange={() => { coverMode = "none"; coverPath = ""; coverDataUrl = ""; }} />
          无封面
        </label>
        <label class="radio-label">
          <input type="radio" name="cover-mode" value="file"
            checked={coverMode === "file"}
            onchange={() => { coverMode = "file"; coverPath = ""; coverDataUrl = ""; }} />
          图片文件
        </label>
        <label class="radio-label">
          <input type="radio" name="cover-mode" value="text"
            checked={coverMode === "text"}
            onchange={() => { coverMode = "text"; coverPath = ""; coverDataUrl = ""; }} />
          文字封面
        </label>
      </div>

      {#if coverMode === "file"}
        <div class="inset">
          <div class="row">
            <input type="text" bind:value={coverPath} readonly placeholder="选择图片文件" />
            <button onclick={onPickCover} disabled={progress.busy}>选择</button>
            {#if coverPath}
              <button onclick={onClearCover} disabled={progress.busy} title="清除">✕</button>
            {/if}
          </div>
          {#if coverDataUrl}
            <img class="cover-preview" src={coverDataUrl} alt="封面预览" />
          {/if}
        </div>
      {:else if coverMode === "text"}
        <div class="inset">
          <div class="row" style="align-items:center;gap:8px;flex-wrap:wrap;">
            <label class="radio-label" style="margin:0">
              <input type="radio" bind:group={textCoverStyle} value="default"
                disabled={progress.busy || generatingCover} />
              深蓝
            </label>
            <label class="radio-label" style="margin:0">
              <input type="radio" bind:group={textCoverStyle} value="gradient"
                disabled={progress.busy || generatingCover} />
              蓝紫
            </label>
            <button
              onclick={onGenerateTextCover}
              disabled={progress.busy || generatingCover || !title || !author}
            >
              {generatingCover ? "生成中..." : "生成预览"}
            </button>
          </div>
          {#if !title || !author}
            <p class="hint" style="margin:4px 0 0">请先填写书名与作者，再生成封面</p>
          {/if}
          {#if coverDataUrl}
            <img class="cover-preview" src={coverDataUrl} alt="封面预览" />
          {/if}
        </div>
      {/if}
    </div>

    <!-- 字体嵌入 -->
    <div class="section">
      <label class="checkbox-label">
        <input type="checkbox" bind:checked={embedFonts} disabled={progress.busy} />
        <span>嵌入中文字体(约 +16 MB)</span>
      </label>
      {#if embedFonts}
        <div class="inset">
          <label class="radio-label">
            <input type="radio" bind:group={fontSource} value="builtin" disabled={progress.busy} />
            霞鹜文楷(内置)
          </label>
          <label class="radio-label">
            <input type="radio" bind:group={fontSource} value="custom" disabled={progress.busy} />
            自定义字体文件
          </label>
          {#if fontSource === "custom"}
            <div class="row" style="margin-top:4px;">
              <input type="text" bind:value={customFontPath} readonly placeholder="选择 .ttf / .otf" />
              <button onclick={onPickCustomFont} disabled={progress.busy}>选择</button>
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <!-- CSS 主题 -->
    <div class="section">
      <div class="section-header">
        <span class="section-title">样式主题</span>
        <button class="link-btn" onclick={() => cssExpanded = !cssExpanded}>
          {cssExpanded ? "收起 CSS ▲" : "编辑 CSS ▼"}
        </button>
      </div>
      <div class="theme-row">
        {#each themes as t}
          <button
            class="theme-btn"
            class:active={selectedTheme === t}
            disabled={progress.busy || loadingTheme}
            onclick={() => onSelectTheme(t)}
          >
            {THEME_LABELS[t] ?? t}
          </button>
        {/each}
        {#if selectedTheme === ""}
          <span class="custom-tag">自定义</span>
        {/if}
      </div>
      {#if cssExpanded}
        <textarea
          class="css-editor"
          bind:value={cssText}
          oninput={onCssInput}
          disabled={progress.busy}
          spellcheck={false}
          rows={14}
        ></textarea>
      {/if}
    </div>

    <!-- 生成 -->
    <button class="primary" onclick={onBuild} disabled={progress.busy}>
      {progress.busy ? "生成中..." : "生成 EPUB"}
    </button>

    <!-- 状态提示 -->
    <div class="status-hints">
      {#if kepubifyEnabled && kepubifyPath}
        <p class="hint ok-hint">✓ 将额外生成 .kepub.epub</p>
      {:else}
        <p class="hint">
          只生成标准 .epub。
          <button class="link-btn" onclick={openSettings}>在设置中启用 kepubify →</button>
        </p>
      {/if}
      {#if decisionCount() > 0}
        <p class="hint">将随生成应用 <strong>{decisionCount()}</strong> 条用户决策。</p>
      {/if}
    </div>

    {#if error}<div class="error">{error}</div>{/if}
    {#if result}<div class="ok">✓ {result}</div>{/if}
  {/if}
</div>

<style>
  .panel { padding: 16px; }
  h2 { font-size: 14px; margin: 0 0 12px 0; color: #1f2933; }
  label { display: block; margin-bottom: 12px; font-size: 12px; color: #52606d; }
  label span { display: block; margin-bottom: 4px; }
  .row { display: flex; gap: 4px; }
  .row input { flex: 1; }
  input[type=text] {
    width: 100%;
    padding: 5px 8px;
    border: 1px solid #cbd2d9;
    border-radius: 4px;
    font-size: 12px;
    background: #fff;
    color: #1f2933;
    box-sizing: border-box;
  }
  button {
    padding: 5px 10px;
    font-size: 12px;
    border: 1px solid #cbd2d9;
    background: #fff;
    border-radius: 4px;
    cursor: pointer;
    color: #1f2933;
    white-space: nowrap;
  }
  button:hover:not(:disabled) { background: #eef1f5; }
  button:disabled { opacity: 0.5; cursor: not-allowed; }
  button.primary {
    background: #1f6feb;
    color: #fff;
    border-color: #1f6feb;
    padding: 7px 14px;
    margin-top: 4px;
    width: 100%;
  }
  button.primary:hover:not(:disabled) { background: #1858c4; }
  .hint { font-size: 11px; color: #52606d; }
  .ok-hint { color: #2e7d32; }
  .cover-mode-row {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    margin-bottom: 8px;
  }
  .cover-preview {
    display: block;
    margin-top: 6px;
    max-width: 100%;
    max-height: 180px;
    object-fit: contain;
    border: 1px solid #cbd2d9;
    border-radius: 4px;
  }
  .section {
    margin-bottom: 12px;
    font-size: 12px;
    color: #52606d;
  }
  .inset {
    margin-top: 8px;
    padding: 8px 10px;
    background: #f5f7fa;
    border: 1px solid #e4e7eb;
    border-radius: 4px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .checkbox-label, .radio-label {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 0;
    cursor: pointer;
    user-select: none;
    font-size: 12px;
    color: #52606d;
  }
  .checkbox-label input[type=checkbox],
  .radio-label input[type=radio] { margin: 0; width: auto; }
  .section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 6px;
  }
  .section-title { font-weight: 600; color: #52606d; }
  .link-btn {
    border: none;
    background: none;
    color: #1f6feb;
    font-size: 11px;
    padding: 0;
    cursor: pointer;
  }
  .link-btn:hover { text-decoration: underline; }
  .theme-row { display: flex; gap: 4px; flex-wrap: wrap; align-items: center; }
  .theme-btn {
    padding: 4px 10px;
    font-size: 11px;
    border: 1px solid #cbd2d9;
    border-radius: 12px;
    background: #fff;
    color: #52606d;
    cursor: pointer;
  }
  .theme-btn.active {
    background: #1f6feb;
    color: #fff;
    border-color: #1f6feb;
  }
  .custom-tag {
    font-size: 11px;
    color: #888;
    padding: 4px 8px;
    border: 1px dashed #cbd2d9;
    border-radius: 12px;
  }
  .meta-group { margin-bottom: 12px; }
  .meta-group label { margin-bottom: 8px; }
  .suggest-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
    flex-wrap: wrap;
  }
  .suggest-btn {
    font-size: 11px;
    padding: 4px 8px;
    border: 1px solid #1f6feb;
    color: #1f6feb;
    background: #fff;
    border-radius: 4px;
    cursor: pointer;
    white-space: nowrap;
  }
  .suggest-btn:hover:not(:disabled) { background: #e8f0fe; }
  .suggest-btn:disabled { opacity: 0.45; cursor: not-allowed; }
  .suggest-hint { color: #888; }
  .search-on { color: #2e7d32; font-weight: 500; }
  .series-row { gap: 8px; }
  .series-name { flex: 2; }
  .series-idx { flex: 1; max-width: 100px; }
  .meta-group textarea {
    width: 100%;
    resize: vertical;
    font-family: inherit;
    font-size: 13px;
    padding: 4px 6px;
    box-sizing: border-box;
    border: 1px solid #cbd2d9;
    border-radius: 3px;
  }
  .suggestion-panel {
    background: #f0f7ff;
    border: 1px solid #b3d4ff;
    border-radius: 4px;
    padding: 8px 10px;
    margin-bottom: 8px;
    font-size: 12px;
  }
  .suggestion-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 6px;
  }
  .suggestion-title { font-weight: 600; color: #1f6feb; }
  .suggestion-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin-bottom: 4px;
    flex-wrap: wrap;
  }
  .suggestion-desc .sug-value { max-width: 100%; white-space: pre-wrap; }
  .sug-label { color: #52606d; min-width: 70px; flex-shrink: 0; }
  .sug-value { color: #1f2933; flex: 1; word-break: break-all; }
  .sug-hint { color: #888; font-style: italic; }
  .sug-apply {
    font-size: 11px;
    padding: 2px 6px;
    border: 1px solid #1f6feb;
    color: #1f6feb;
    background: #fff;
    border-radius: 3px;
    cursor: pointer;
    flex-shrink: 0;
  }
  .sug-apply:hover { background: #e8f0fe; }
  .css-editor {
    display: block;
    width: 100%;
    margin-top: 8px;
    padding: 8px;
    font-family: Consolas, "Cascadia Mono", monospace;
    font-size: 11px;
    line-height: 1.5;
    border: 1px solid #cbd2d9;
    border-radius: 4px;
    background: #f8fafc;
    color: #1f2933;
    resize: vertical;
    box-sizing: border-box;
  }
  .status-hints {
    margin-top: 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .status-hints .hint { margin: 0; }
  .error {
    margin-top: 10px;
    padding: 8px 10px;
    background: #ffebee;
    color: #c62828;
    border: 1px solid #ef9a9a;
    border-radius: 4px;
    font-size: 12px;
    white-space: pre-wrap;
  }
  .ok {
    margin-top: 10px;
    padding: 8px 10px;
    background: #e8f5e9;
    color: #2e7d32;
    border: 1px solid #a5d6a7;
    border-radius: 4px;
    font-size: 12px;
    font-family: Consolas, "Cascadia Mono", monospace;
  }
</style>
