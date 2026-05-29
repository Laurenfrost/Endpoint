<script>
  // 设置面板:LLM / 搜索后端 / kepubify。
  import { progress } from "../stores/progress.svelte.js";
  import { llm, applyLlmConfig } from "../stores/llm.svelte.js";
  import {
    pickExecutableFile,
    getLlmConfig, setLlmConfig, setSearchConfig,
    getKepubifyConfig, setKepubifyConfig,
  } from "../ipc.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import { Checkbox } from "$lib/components/ui/checkbox/index.js";
  import { cn } from "$lib/utils.js";

  let llmBaseUrl = $state("");
  let llmModel = $state("");
  let llmApiKey = $state("");
  let llmSaving = $state(false);
  let llmMsg = $state("");

  let searchProvider = $state("brave");
  let searchApiKey = $state("");
  let searchSaving = $state(false);
  let searchMsg = $state("");

  let kepubifyPath = $state("");
  let kepubifyEnabled = $state(false);
  let kepubifyMsg = $state("");

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

<div class="flex flex-col gap-4 p-4">
  <h2 class="text-sm font-semibold">⚙ 设置</h2>

  <!-- LLM -->
  <section class="flex flex-col gap-2 border-b pb-4">
    <div class="flex items-center justify-between">
      <span class="flex items-center gap-1.5 text-xs font-semibold">
        LLM
        <span class={cn(
          "inline-block size-1.5 rounded-full",
          llm.configured ? "bg-emerald-500" : "bg-muted-foreground/50",
        )}></span>
      </span>
    </div>
    <p class="text-[11px] text-muted-foreground">
      用于水印仲裁、规则归纳、元数据建议。未配置时所有 LLM 功能静默跳过,不影响其他流程。
    </p>
    <div class="flex flex-col gap-1.5">
      <Label for="llm-url">
        API 接口地址 base_url
        <span class="ml-1 rounded bg-rose-100 px-1 text-[10px] text-rose-700 dark:bg-rose-500/20 dark:text-rose-300">必填</span>
      </Label>
      <Input id="llm-url" bind:value={llmBaseUrl}
        placeholder="例如:https://api.deepseek.com" disabled={llmSaving} />
      <span class="text-[11px] text-muted-foreground">
        OpenAI 兼容的 chat completions 服务都可以(DeepSeek / 本地 Ollama / OpenAI 等)。
      </span>
    </div>
    <div class="flex flex-col gap-1.5">
      <Label for="llm-model">
        模型
        <span class="ml-1 rounded bg-rose-100 px-1 text-[10px] text-rose-700 dark:bg-rose-500/20 dark:text-rose-300">必填</span>
      </Label>
      <Input id="llm-model" bind:value={llmModel}
        placeholder="例如:deepseek-chat" disabled={llmSaving} />
    </div>
    <div class="flex flex-col gap-1.5">
      <Label for="llm-key">
        API Key {llm.configured ? `(当前: ${llm.keyMasked})` : ""}
      </Label>
      <Input id="llm-key" type="password" bind:value={llmApiKey}
        placeholder="留空保持原 key 不变" disabled={llmSaving} />
    </div>
    <div class="flex justify-end">
      <Button size="sm" onclick={onSaveLlm} disabled={llmSaving}>
        {llmSaving ? "保存中…" : "保存 LLM 配置"}
      </Button>
    </div>
    {#if llmMsg}<p class="text-[11px] text-muted-foreground">{llmMsg}</p>{/if}
    <p class="text-[11px] text-muted-foreground/70">
      API key 明文存储于 <code class="rounded bg-muted px-1 font-mono text-[10px]">%APPDATA%\Endpoint\config.toml</code>,仅限本机使用。
    </p>
  </section>

  <!-- Web 搜索后端 -->
  <section class="flex flex-col gap-2 border-b pb-4">
    <div class="flex items-center justify-between">
      <span class="flex items-center gap-1.5 text-xs font-semibold">
        Web 搜索后端(可选)
        <span class={cn(
          "inline-block size-1.5 rounded-full",
          llm.searchConfigured ? "bg-emerald-500" : "bg-muted-foreground/50",
        )}></span>
      </span>
    </div>
    <p class="text-[11px] text-muted-foreground">
      冷门作品 LLM 训练知识识别不到时,会用搜索结果补全分类/系列/简介。留空 provider 即可禁用搜索。
    </p>
    <div class="flex flex-col gap-1.5">
      <Label for="search-provider">Provider</Label>
      <Input id="search-provider" bind:value={searchProvider}
        placeholder="brave(留空 = 禁用搜索)" disabled={searchSaving} />
      <span class="text-[11px] text-muted-foreground">
        目前仅支持 <code class="rounded bg-muted px-1 font-mono text-[10px]">brave</code>。在
        <a class="text-primary hover:underline" href="https://brave.com/search/api/" target="_blank" rel="noopener">brave.com/search/api</a>
        注册免费 2000 次/月。
      </span>
    </div>
    <div class="flex flex-col gap-1.5">
      <Label for="search-key">
        Brave API Key {llm.searchConfigured ? `(当前: ${llm.searchKeyMasked})` : ""}
      </Label>
      <Input id="search-key" type="password" bind:value={searchApiKey}
        placeholder="留空保持原 key 不变" disabled={searchSaving} />
    </div>
    <div class="flex justify-end">
      <Button size="sm" onclick={onSaveSearch} disabled={searchSaving}>
        {searchSaving ? "保存中…" : "保存搜索配置"}
      </Button>
    </div>
    {#if searchMsg}<p class="text-[11px] text-muted-foreground">{searchMsg}</p>{/if}
  </section>

  <!-- kepubify -->
  <section class="flex flex-col gap-2">
    <span class="text-xs font-semibold">kepubify 优化(可选)</span>
    <p class="text-[11px] text-muted-foreground">
      生成 <code class="rounded bg-muted px-1 font-mono text-[10px]">.kepub.epub</code>,Kobo 设备排版/分页体验更好。需要本地 kepubify.exe。
    </p>
    <label class="flex cursor-pointer items-center gap-2 text-xs">
      <Checkbox
        bind:checked={kepubifyEnabled}
        onCheckedChange={onToggleKepubifyEnabled}
        disabled={progress.busy || !kepubifyPath}
      />
      生成 .kepub.epub
    </label>
    <div class="flex gap-1.5">
      <Input value={kepubifyPath} readonly
        placeholder="未设置 kepubify.exe 路径" disabled={progress.busy} />
      <Button variant="outline" size="sm" onclick={onPickKepubify} disabled={progress.busy}>选择</Button>
      {#if kepubifyPath}
        <Button variant="outline" size="sm" onclick={onClearKepubify} disabled={progress.busy} title="清除路径">✕</Button>
      {/if}
    </div>
    <p class="text-[11px] text-muted-foreground">
      {#if !kepubifyPath}
        未配置时只生成标准 .epub。路径设过一次后会自动记住。
      {:else if !kepubifyEnabled}
        已记住路径但暂不启用,导出时只出 .epub。
      {:else}
        导出时将额外跑 kepubify 生成 .kepub.epub。
      {/if}
    </p>
    {#if kepubifyMsg}<p class="text-[11px] text-destructive">{kepubifyMsg}</p>{/if}
  </section>

  <!-- 作者信息 -->
  <footer class="mt-2 border-t pt-3 text-[11px] text-muted-foreground/60">
    <p>作者: Laurenfrost</p>
    <p>邮箱: <u>me@lvy.ink</u></p>
    <p class="mt-0.5">
      项目地址:
      <a class="hover:text-muted-foreground hover:underline" href="https://github.com/Laurenfrost/Endpoint" target="_blank" rel="noopener">
        https://github.com/Laurenfrost/Endpoint
      </a>
    </p>
    <p>AI 声明: 本项目在开发过程中使用了 AI 工具</p>
  </footer>
</div>
