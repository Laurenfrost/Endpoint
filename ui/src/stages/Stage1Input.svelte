<script>
  // 阶段 1:文件选择 + 编码自动/手动。
  import { pickInputFile, loadAndAnalyze } from "../ipc.js";
  import { setPipeline, pipeline } from "../stores/pipeline.svelte.js";
  import { setStage } from "../stores/stage.svelte.js";
  import { progress, setBusy } from "../stores/progress.svelte.js";

  let inputPath = $state("");
  let encodingOverride = $state(""); // "" = auto
  let error = $state("");

  // 注:进度事件订阅在 App.svelte 全局挂一次,跨阶段都活着;
  // 之前放在 Stage1Input 会在跳阶段后被 unlisten,Stage 4 build 进度就丢了。

  async function onPick() {
    error = "";
    try {
      const p = await pickInputFile();
      if (typeof p === "string") inputPath = p;
    } catch (e) {
      error = String(e);
    }
  }

  async function onLoad() {
    if (!inputPath) {
      error = "请先选择 txt 文件";
      return;
    }
    error = "";
    setBusy(true);
    try {
      const dto = await loadAndAnalyze(inputPath, encodingOverride || null);
      setPipeline(dto, inputPath);
      // 自动跳到阶段 2(清洗)
      setStage(2);
    } catch (e) {
      error = String(e);
    } finally {
      setBusy(false);
    }
  }

  function reload() {
    onLoad();
  }
</script>

<div class="panel">
  <h2>1. 文本选择</h2>

  <label>
    <span>txt 文件</span>
    <div class="row">
      <input type="text" readonly bind:value={inputPath} placeholder="点击选择..." />
      <button onclick={onPick} disabled={progress.busy}>选择</button>
    </div>
  </label>

  <label>
    <span>编码</span>
    <select bind:value={encodingOverride} disabled={progress.busy}>
      <option value="">自动探测(推荐)</option>
      <option value="UTF-8">UTF-8</option>
      <option value="GBK">GBK</option>
      <option value="GB18030">GB18030</option>
      <option value="UTF-16LE">UTF-16LE</option>
      <option value="UTF-16BE">UTF-16BE</option>
    </select>
  </label>

  <button class="primary" onclick={onLoad} disabled={progress.busy || !inputPath}>
    {pipeline.dto ? "重新加载并分析" : "加载并分析"}
  </button>

  {#if error}
    <div class="error">{error}</div>
  {/if}

  {#if pipeline.dto}
    <div class="summary">
      <h3>分析结果</h3>
      <dl>
        <dt>实际编码</dt><dd>{pipeline.dto.source_encoding}</dd>
        <dt>源文本</dt><dd>{pipeline.dto.source_text.length.toLocaleString()} 字符</dd>
        <dt>清洗标注</dt><dd>{pipeline.dto.cleaning.length} 条</dd>
        <dt>顶层条目</dt><dd>{pipeline.dto.book.entries.length}</dd>
      </dl>
      <p class="hint">切到「文本处理」「章节分析」「样式预览与导出」继续。</p>
      <button class="secondary" onclick={reload}>用当前编码重跑</button>
    </div>
  {/if}
</div>

<style>
  .panel { padding: 16px; }
  h2 { font-size: 14px; margin: 0 0 12px 0; color: #1f2933; }
  h3 { font-size: 12px; margin: 12px 0 6px 0; color: #52606d; text-transform: uppercase; letter-spacing: 0.5px; }
  label { display: block; margin-bottom: 12px; font-size: 12px; color: #52606d; }
  label span { display: block; margin-bottom: 4px; }
  .row { display: flex; gap: 4px; }
  .row input { flex: 1; }
  input[type=text], select {
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
  button.secondary { margin-top: 8px; }
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
  .summary {
    margin-top: 14px;
    padding: 10px 12px;
    background: #fff;
    border: 1px solid #cbd2d9;
    border-radius: 4px;
  }
  dl { margin: 0; display: grid; grid-template-columns: auto 1fr; gap: 4px 12px; font-size: 12px; }
  dt { color: #52606d; }
  dd { margin: 0; color: #1f2933; font-family: Consolas, "Cascadia Mono", monospace; }
  .hint { font-size: 11px; color: #52606d; margin: 8px 0; }
</style>
