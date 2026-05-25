<script>
  // 阶段 4:样式预览与导出。4.0 封面+CSS 覆盖;4.1 字体嵌入;4.2 CSS 主题预设+编辑器。
  import { pipeline } from "../stores/pipeline.svelte.js";
  import { progress, setBusy } from "../stores/progress.svelte.js";
  import {
    pickOutputFile, pickExecutableFile, pickCoverFile, pickFontFile,
    buildEpub, listThemes, loadTheme, generateTextCover,
    getLlmConfig, setLlmConfig, suggestMetadata,
  } from "../ipc.js";
  import { llm, applyLlmConfig } from "../stores/llm.svelte.js";
  import { serializeForIpc, decisionCount } from "../stores/decisions.svelte.js";

  const THEME_LABELS = { standard: "标准", classic: "古风", highcontrast: "高对比度" };

  let outputPath = $state("");
  let title = $state("");
  let author = $state("");
  let kepubifyPath = $state("");
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
  let themes = $state([]);          // 主题名称列表
  let selectedTheme = $state("");   // 当前选中主题名;"" = 自定义
  let cssText = $state("");         // textarea 内容
  let cssExpanded = $state(false);  // 高级 CSS 展开状态
  let loadingTheme = $state(false);
  // LLM 配置
  let llmOpen = $state(false);
  let llmBaseUrl = $state("");
  let llmModel = $state("");
  let llmApiKey = $state("");
  let llmSaving = $state(false);
  let llmMsg = $state("");
  // LLM 元数据建议(4.6)
  let suggesting = $state(false);
  let suggestion = $state(null);   // { title?, author?, description?, cover_keywords? } | null
  let suggestionMsg = $state("");
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
        if (list.includes("standard")) {
          onSelectTheme("standard");
        }
      }).catch(() => {});
    }
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
    // 用户直接编辑了 textarea → 切换为自定义
    selectedTheme = "";
  }

  async function onPickOutput() {
    try {
      const p = await pickOutputFile(outputPath || null);
      if (typeof p === "string") outputPath = p;
    } catch (e) { error = String(e); }
  }

  async function onPickKepubify() {
    try {
      const p = await pickExecutableFile();
      if (typeof p === "string") kepubifyPath = p;
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

  // 进入阶段时加载当前 LLM 配置
  $effect(() => {
    getLlmConfig().then(cfg => {
      applyLlmConfig(cfg);
      llmBaseUrl = cfg.base_url ?? "";
      llmModel = cfg.model ?? "";
      llmApiKey = "";  // 不预填 key,让用户重新输入
    }).catch(() => {});
  });

  function onToggleLlm() {
    llmOpen = !llmOpen;
    llmMsg = "";
  }

  async function onSaveLlm() {
    llmSaving = true;
    llmMsg = "";
    try {
      await setLlmConfig(llmBaseUrl, llmModel, llmApiKey);
      const cfg = await getLlmConfig();
      applyLlmConfig(cfg);
      llmMsg = llm.configured ? "✓ 已保存并连接" : "已保存(未填 API key,LLM 功能未启用)";
      llmApiKey = "";
    } catch (e) {
      llmMsg = `保存失败: ${e}`;
    } finally {
      llmSaving = false;
    }
  }

  async function onSuggestMetadata() {
    suggesting = true;
    suggestion = null;
    suggestionMsg = "";
    try {
      const result = await suggestMetadata();
      if (!result) {
        suggestionMsg = llm.configured ? "LLM 无法从正文推断元数据" : "请先在下方配置 LLM";
      } else {
        suggestion = result;
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
      const finalPath = await buildEpub({
        outputPath,
        title,
        author,
        kepubifyPath: kepubifyPath || null,
        decisions: decisions.length > 0 ? decisions : null,
        coverPath: coverPath || null,
        cssOverride: cssText || null,
        embedFonts,
        fontPath: embedFonts && fontSource === "custom" ? customFontPath : null,
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
      <div class="suggest-row">
        <button
          class="suggest-btn"
          onclick={onSuggestMetadata}
          disabled={progress.busy || suggesting || !pipeline.dto}
          title={llm.configured ? "从正文前约 1 万字请 LLM 推断书名、作者、简介" : "请先在下方 LLM 设置中配置 API key"}
        >
          {suggesting ? "推断中..." : "从正文建议 ▸"}
        </button>
        {#if suggestionMsg}<span class="hint suggest-hint">{suggestionMsg}</span>{/if}
      </div>

      {#if suggestion}
        <div class="suggestion-panel">
          <div class="suggestion-header">
            <span class="suggestion-title">LLM 建议</span>
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

    <!-- LLM 配置 -->
    <div class="section">
      <div class="section-header">
        <span class="section-title">
          LLM 设置
          {#if llm.configured}
            <span class="llm-dot configured" title="已配置 LLM"></span>
          {:else}
            <span class="llm-dot" title="未配置 LLM"></span>
          {/if}
        </span>
        <button class="link-btn" onclick={onToggleLlm}>
          {llmOpen ? "收起 ▲" : "配置 ▼"}
        </button>
      </div>
      {#if !llmOpen}
        <p class="hint" style="margin:0">
          {llm.configured ? `已配置(${llm.keyMasked})` : "未配置 — LLM 功能(仲裁/规则归纳/元数据建议)不可用"}
        </p>
      {:else}
        <div class="inset llm-form">
          <label>
            <span>API 接口地址(base_url)</span>
            <input type="text" bind:value={llmBaseUrl}
              placeholder="https://api.deepseek.com"
              disabled={llmSaving} />
          </label>
          <label>
            <span>模型</span>
            <input type="text" bind:value={llmModel}
              placeholder="deepseek-chat"
              disabled={llmSaving} />
          </label>
          <label>
            <span>API Key {llm.configured ? `(当前: ${llm.keyMasked})` : ""}</span>
            <input type="password" bind:value={llmApiKey}
              placeholder="留空保持原 key 不变"
              disabled={llmSaving} />
          </label>
          <div class="row" style="justify-content:flex-end;gap:6px;">
            <button onclick={onSaveLlm} disabled={llmSaving}>
              {llmSaving ? "保存中..." : "保存"}
            </button>
          </div>
          {#if llmMsg}<p class="hint" style="margin:4px 0 0">{llmMsg}</p>{/if}
          <p class="hint" style="margin:4px 0 0;color:#9aa">
            API key 明文存储于 AppData\Endpoint\config.toml,仅限本机使用。
            兼容 OpenAI /v1/chat/completions 协议(DeepSeek / OpenAI / 本地 Ollama)。
          </p>
        </div>
      {/if}
    </div>

    <!-- kepubify -->
    <label style="margin-top:4px;">
      <span>kepubify(可选)</span>
      <div class="row">
        <input type="text" bind:value={kepubifyPath} placeholder="留空只出 .epub" disabled={progress.busy} />
        <button onclick={onPickKepubify} disabled={progress.busy}>选择</button>
      </div>
    </label>

    <button class="primary" onclick={onBuild} disabled={progress.busy}>
      {progress.busy ? "生成中..." : "生成 EPUB"}
    </button>
    {#if decisionCount() > 0}
      <p class="hint" style="margin-top:6px;">将随生成应用 <strong>{decisionCount()}</strong> 条用户决策。</p>
    {/if}

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
  /* 通用 section 容器 */
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
  /* CSS 主题区域 */
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
  /* 元数据建议 */
  .meta-group { margin-bottom: 12px; }
  .meta-group label { margin-bottom: 8px; }
  .suggest-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
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
  .llm-dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #cbd2d9;
    margin-left: 4px;
    vertical-align: middle;
  }
  .llm-dot.configured { background: #2e7d32; }
  .llm-form label { margin-bottom: 8px; }
  .llm-form label span { display: block; margin-bottom: 2px; }
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
