<script>
  // 设置面板:LLM / 搜索后端 / kepubify。
  // 从 Stage4Export 抽出的全局配置区,与具体导出无关。
  import { progress } from "../stores/progress.svelte.js";
  import { llm, applyLlmConfig } from "../stores/llm.svelte.js";
  import {
    pickExecutableFile,
    getLlmConfig, setLlmConfig, setSearchConfig,
    getKepubifyConfig, setKepubifyConfig,
  } from "../ipc.js";

  // LLM 配置
  let llmBaseUrl = $state("");
  let llmModel = $state("");
  let llmApiKey = $state("");
  let llmSaving = $state(false);
  let llmMsg = $state("");

  // Brave 搜索配置
  let searchProvider = $state("brave");
  let searchApiKey = $state("");
  let searchSaving = $state(false);
  let searchMsg = $state("");

  // kepubify 配置
  let kepubifyPath = $state("");
  let kepubifyEnabled = $state(false);
  let kepubifyMsg = $state("");

  // 进入时加载持久化配置
  $effect(() => {
    getLlmConfig().then(cfg => {
      applyLlmConfig(cfg);
      llmBaseUrl = cfg.base_url ?? "";
      llmModel = cfg.model ?? "";
      llmApiKey = "";
      searchProvider = cfg.search_provider || "brave";
      searchApiKey = "";
    }).catch(() => {});

    getKepubifyConfig().then(cfg => {
      kepubifyPath = cfg.path ?? "";
      kepubifyEnabled = !!cfg.enabled;
    }).catch(() => {});
  });

  async function onSaveLlm() {
    if (!llmBaseUrl.trim()) {
      llmMsg = "请填写 API 接口地址(base_url),不是用 placeholder 里的示例。";
      return;
    }
    if (!llmModel.trim()) {
      llmMsg = "请填写模型名。";
      return;
    }
    llmSaving = true;
    llmMsg = "";
    try {
      await setLlmConfig(llmBaseUrl, llmModel, llmApiKey);
      const cfg = await getLlmConfig();
      applyLlmConfig(cfg);
      if (llm.configured) {
        llmMsg = "✓ 已保存并连接";
      } else if (cfg.key_set) {
        llmMsg = "已保存,但 LLM 功能未启用(base_url 或模型为空)";
      } else {
        llmMsg = "已保存(未填 API key,LLM 功能未启用)";
      }
      llmApiKey = "";
    } catch (e) {
      llmMsg = `保存失败: ${e}`;
    } finally {
      llmSaving = false;
    }
  }

  async function onSaveSearch() {
    if (searchProvider && !searchApiKey && !llm.searchKeyMasked) {
      searchMsg = "请填写 Brave API Key,或留空 provider 字段禁用搜索。";
      return;
    }
    searchSaving = true;
    searchMsg = "";
    try {
      await setSearchConfig(searchProvider.trim(), searchApiKey);
      const cfg = await getLlmConfig();
      applyLlmConfig(cfg);
      if (llm.searchConfigured) {
        searchMsg = "✓ 已保存,搜索后端可用";
      } else if (!searchProvider.trim()) {
        searchMsg = "已保存(已禁用搜索,LLM 只用训练知识)";
      } else {
        searchMsg = "已保存,但搜索未启用(provider 或 key 为空)";
      }
      searchApiKey = "";
    } catch (e) {
      searchMsg = `保存失败: ${e}`;
    } finally {
      searchSaving = false;
    }
  }

  async function persistKepubify(path, enabled) {
    try {
      await setKepubifyConfig(path, enabled);
      kepubifyMsg = "";
    } catch (e) {
      kepubifyMsg = `保存失败: ${e}`;
    }
  }

  async function onPickKepubify() {
    try {
      const p = await pickExecutableFile();
      if (typeof p === "string") {
        kepubifyPath = p;
        if (!kepubifyEnabled) kepubifyEnabled = true;
        persistKepubify(kepubifyPath, kepubifyEnabled);
      }
    } catch (e) {
      kepubifyMsg = String(e);
    }
  }

  function onClearKepubify() {
    kepubifyPath = "";
    kepubifyEnabled = false;
    persistKepubify("", false);
  }

  function onToggleKepubifyEnabled() {
    persistKepubify(kepubifyPath, kepubifyEnabled);
  }
</script>

<div class="panel">
  <h2>⚙ 设置</h2>

  <!-- LLM -->
  <section class="section">
    <div class="section-header">
      <span class="section-title">
        LLM
        {#if llm.configured}
          <span class="dot configured" title="已配置"></span>
        {:else}
          <span class="dot" title="未配置"></span>
        {/if}
      </span>
    </div>
    <p class="hint section-explain">
      用于水印仲裁、规则归纳、元数据建议。未配置时所有 LLM 功能静默跳过,不影响其他流程。
    </p>
    <label>
      <span>API 接口地址 base_url <em class="required">必填</em></span>
      <input type="text" bind:value={llmBaseUrl}
        placeholder="例如:https://api.deepseek.com"
        disabled={llmSaving} />
      <span class="field-hint">OpenAI 兼容的 chat completions 服务都可以(DeepSeek / 本地 Ollama / OpenAI 等)。</span>
    </label>
    <label>
      <span>模型 <em class="required">必填</em></span>
      <input type="text" bind:value={llmModel}
        placeholder="例如:deepseek-chat"
        disabled={llmSaving} />
    </label>
    <label>
      <span>API Key {llm.configured ? `(当前: ${llm.keyMasked})` : ""}</span>
      <input type="password" bind:value={llmApiKey}
        placeholder="留空保持原 key 不变"
        disabled={llmSaving} />
    </label>
    <div class="row right">
      <button onclick={onSaveLlm} disabled={llmSaving}>
        {llmSaving ? "保存中..." : "保存 LLM 配置"}
      </button>
    </div>
    {#if llmMsg}<p class="hint">{llmMsg}</p>{/if}
    <p class="hint storage">
      API key 明文存储于 <code>%APPDATA%\Endpoint\config.toml</code>,仅限本机使用。
    </p>
  </section>

  <!-- Web 搜索后端 -->
  <section class="section">
    <div class="section-header">
      <span class="section-title">
        Web 搜索后端(可选)
        {#if llm.searchConfigured}
          <span class="dot configured" title="搜索已配置"></span>
        {:else}
          <span class="dot" title="搜索未配置"></span>
        {/if}
      </span>
    </div>
    <p class="hint section-explain">
      冷门作品 LLM 训练知识识别不到时,会用搜索结果补全分类/系列/简介。留空 provider 即可禁用搜索。
    </p>
    <label>
      <span>Provider</span>
      <input type="text" bind:value={searchProvider}
        placeholder="brave(留空 = 禁用搜索)"
        disabled={searchSaving} />
      <span class="field-hint">
        目前仅支持 <code>brave</code>。在
        <a href="https://brave.com/search/api/" target="_blank">brave.com/search/api</a>
        注册免费 2000 次/月。
      </span>
    </label>
    <label>
      <span>Brave API Key {llm.searchConfigured ? `(当前: ${llm.searchKeyMasked})` : ""}</span>
      <input type="password" bind:value={searchApiKey}
        placeholder="留空保持原 key 不变"
        disabled={searchSaving} />
    </label>
    <div class="row right">
      <button onclick={onSaveSearch} disabled={searchSaving}>
        {searchSaving ? "保存中..." : "保存搜索配置"}
      </button>
    </div>
    {#if searchMsg}<p class="hint">{searchMsg}</p>{/if}
  </section>

  <!-- kepubify -->
  <section class="section">
    <div class="section-header">
      <span class="section-title">kepubify 优化(可选)</span>
    </div>
    <p class="hint section-explain">
      生成 <code>.kepub.epub</code>,Kobo 设备排版/分页体验更好。需要本地 kepubify.exe。
    </p>
    <label class="checkbox-label">
      <input
        type="checkbox"
        bind:checked={kepubifyEnabled}
        onchange={onToggleKepubifyEnabled}
        disabled={progress.busy || !kepubifyPath}
      />
      <span>生成 .kepub.epub</span>
    </label>
    <div class="row">
      <input type="text" value={kepubifyPath} readonly
        placeholder="未设置 kepubify.exe 路径"
        disabled={progress.busy} />
      <button onclick={onPickKepubify} disabled={progress.busy}>选择</button>
      {#if kepubifyPath}
        <button onclick={onClearKepubify} disabled={progress.busy} title="清除路径">✕</button>
      {/if}
    </div>
    <p class="hint">
      {#if !kepubifyPath}
        未配置时只生成标准 .epub。路径设过一次后会自动记住。
      {:else if !kepubifyEnabled}
        已记住路径但暂不启用,导出时只出 .epub。
      {:else}
        导出时将额外跑 kepubify 生成 .kepub.epub。
      {/if}
    </p>
    {#if kepubifyMsg}<p class="hint">{kepubifyMsg}</p>{/if}
  </section>
</div>

<style>
  .panel { padding: 16px; }
  h2 { font-size: 14px; margin: 0 0 14px 0; color: #1f2933; }
  .section {
    margin-bottom: 18px;
    padding-bottom: 14px;
    border-bottom: 1px solid #e4e7eb;
  }
  .section:last-child { border-bottom: none; }
  .section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 6px;
  }
  .section-title {
    font-weight: 600;
    color: #1f2933;
    font-size: 12px;
  }
  .section-explain {
    margin: 0 0 10px;
    color: #6b7280;
    font-size: 11px;
  }
  .dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #cbd2d9;
    margin-left: 6px;
    vertical-align: middle;
  }
  .dot.configured { background: #2e7d32; }
  label {
    display: block;
    margin-bottom: 8px;
    font-size: 12px;
    color: #52606d;
  }
  label span { display: block; margin-bottom: 2px; }
  .checkbox-label {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 6px;
    cursor: pointer;
    user-select: none;
  }
  .checkbox-label input[type=checkbox] { margin: 0; width: auto; }
  .row { display: flex; gap: 4px; }
  .row.right { justify-content: flex-end; }
  .row input { flex: 1; }
  input[type=text], input[type=password] {
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
  .hint {
    font-size: 11px;
    color: #52606d;
    margin: 4px 0 0;
  }
  .hint.storage { color: #9aa5b1; margin-top: 6px; }
  .required {
    display: inline-block;
    margin-left: 6px;
    padding: 0 5px;
    background: #ffe4e6;
    color: #b42318;
    border-radius: 3px;
    font-size: 10px;
    font-style: normal;
    vertical-align: middle;
  }
  .field-hint {
    display: block;
    margin-top: 3px;
    font-size: 11px;
    color: #6b7280;
  }
  a { color: #1f6feb; }
  code {
    background: #eef1f5;
    padding: 0 4px;
    border-radius: 3px;
    font-family: Consolas, "Cascadia Mono", monospace;
    font-size: 11px;
  }
</style>
