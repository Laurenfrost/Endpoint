<script>
  // 阶段 4:样式预览与导出。4.0 封面嵌入 + CSS 覆盖;4.1 字体嵌入 opt-in。
  import { pipeline } from "../stores/pipeline.svelte.js";
  import { progress, setBusy } from "../stores/progress.svelte.js";
  import { pickOutputFile, pickExecutableFile, pickCoverFile, pickFontFile, buildEpub } from "../ipc.js";
  import { serializeForIpc, decisionCount } from "../stores/decisions.svelte.js";

  let outputPath = $state("");
  let title = $state("");
  let author = $state("");
  let kepubifyPath = $state("");
  let coverPath = $state("");
  let coverDataUrl = $state("");
  // 字体嵌入(4.1)
  let embedFonts = $state(false);
  let fontSource = $state("builtin"); // "builtin" | "custom"
  let customFontPath = $state("");
  let error = $state("");
  let result = $state("");

  // 输出路径默认推导自源文件
  $effect(() => {
    if (pipeline.sourcePath && !outputPath) {
      outputPath = pipeline.sourcePath.replace(/\.txt$/i, "") + ".epub";
    }
  });

  async function onPickOutput() {
    try {
      const p = await pickOutputFile(outputPath || null);
      if (typeof p === "string") outputPath = p;
    } catch (e) {
      error = String(e);
    }
  }

  async function onPickKepubify() {
    try {
      const p = await pickExecutableFile();
      if (typeof p === "string") kepubifyPath = p;
    } catch (e) {
      error = String(e);
    }
  }

  async function onPickCover() {
    try {
      const res = await pickCoverFile();
      if (res && res.path) {
        coverPath = res.path;
        coverDataUrl = res.dataUrl;
      }
    } catch (e) {
      error = String(e);
    }
  }

  function onClearCover() {
    coverPath = "";
    coverDataUrl = "";
  }

  async function onPickCustomFont() {
    try {
      const p = await pickFontFile();
      if (typeof p === "string") customFontPath = p;
    } catch (e) {
      error = String(e);
    }
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
        cssOverride: null,
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

    <label>
      <span>书名 *</span>
      <input type="text" bind:value={title} disabled={progress.busy} />
    </label>

    <label>
      <span>作者 *</span>
      <input type="text" bind:value={author} disabled={progress.busy} />
    </label>

    <label>
      <span>封面图片(可选)</span>
      <div class="row">
        <input type="text" bind:value={coverPath} readonly placeholder="留空则无封面" />
        <button onclick={onPickCover} disabled={progress.busy}>选择</button>
        {#if coverPath}
          <button onclick={onClearCover} disabled={progress.busy} title="清除封面">✕</button>
        {/if}
      </div>
      {#if coverDataUrl}
        <img class="cover-preview" src={coverDataUrl} alt="封面预览" />
      {/if}
    </label>

    <div class="font-section">
      <label class="checkbox-label">
        <input type="checkbox" bind:checked={embedFonts} disabled={progress.busy} />
        <span>嵌入中文字体(约 +16 MB)</span>
      </label>
      {#if embedFonts}
        <div class="font-options">
          <label class="radio-label">
            <input type="radio" bind:group={fontSource} value="builtin" disabled={progress.busy} />
            霞鹜文楷(内置,需运行 fetch-fonts.ps1)
          </label>
          <label class="radio-label">
            <input type="radio" bind:group={fontSource} value="custom" disabled={progress.busy} />
            自定义字体文件
          </label>
          {#if fontSource === "custom"}
            <div class="row" style="margin-top:4px;">
              <input type="text" bind:value={customFontPath} readonly placeholder="选择 .ttf / .otf 文件" />
              <button onclick={onPickCustomFont} disabled={progress.busy}>选择</button>
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <label style="margin-top:12px;">
      <span>kepubify(可选)</span>
      <div class="row">
        <input
          type="text"
          bind:value={kepubifyPath}
          placeholder="留空只出 .epub"
          disabled={progress.busy}
        />
        <button onclick={onPickKepubify} disabled={progress.busy}>选择</button>
      </div>
    </label>

    <button class="primary" onclick={onBuild} disabled={progress.busy}>
      {progress.busy ? "生成中..." : "生成 EPUB"}
    </button>
    {#if decisionCount() > 0}
      <p class="hint" style="margin-top: 6px;">将随生成应用 <strong>{decisionCount()}</strong> 条用户决策(阶段 2 中的接受 / 拒绝)。</p>
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
  }
  button {
    padding: 5px 10px;
    font-size: 12px;
    border: 1px solid #cbd2d9;
    background: #fff;
    border-radius: 4px;
    cursor: pointer;
    color: #1f2933;
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
  .cover-preview {
    display: block;
    margin-top: 6px;
    max-width: 100%;
    max-height: 180px;
    object-fit: contain;
    border: 1px solid #cbd2d9;
    border-radius: 4px;
  }
  /* 字体嵌入区域 */
  .font-section {
    margin-bottom: 12px;
    font-size: 12px;
    color: #52606d;
  }
  .checkbox-label {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 0;
    cursor: pointer;
    user-select: none;
  }
  .checkbox-label input[type=checkbox] { margin: 0; width: auto; }
  .font-options {
    margin-top: 8px;
    padding: 8px 10px;
    background: #f5f7fa;
    border: 1px solid #e4e7eb;
    border-radius: 4px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .radio-label {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 0;
    cursor: pointer;
    user-select: none;
    font-size: 12px;
    color: #52606d;
  }
  .radio-label input[type=radio] { margin: 0; width: auto; }
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
