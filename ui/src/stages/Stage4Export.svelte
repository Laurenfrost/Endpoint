<script>
  // 阶段 4:样式预览与导出。2.3 接入完整功能(沿用 2.2 的临时实现逻辑)。
  // 2.5 子阶段补:封面预览、前几章前几页预览(若时间允许)。
  import { pipeline } from "../stores/pipeline.svelte.js";
  import { progress, setBusy } from "../stores/progress.svelte.js";
  import { pickOutputFile, pickExecutableFile, buildEpub } from "../ipc.js";

  let outputPath = $state("");
  let title = $state("");
  let author = $state("");
  let kepubifyPath = $state("");
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

  async function onBuild() {
    if (!outputPath) return (error = "请先选择输出位置");
    if (!title || !author) return (error = "请填写书名与作者");
    error = "";
    result = "";
    setBusy(true);
    try {
      const finalPath = await buildEpub({
        outputPath,
        title,
        author,
        kepubifyPath: kepubifyPath || null,
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

    {#if error}<div class="error">{error}</div>{/if}
    {#if result}<div class="ok">✓ {result}</div>{/if}

    <p class="hint" style="margin-top: 16px;">
      封面 / 字体嵌入 / CSS 编辑器 属于阶段四(详见 CLAUDE.md 路线图)。
    </p>
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
