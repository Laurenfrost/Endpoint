<script>
  // 阶段 1:文件选择 + 编码自动/手动。
  import AlertCircle from "@lucide/svelte/icons/alert-circle";
  import FolderOpen from "@lucide/svelte/icons/folder-open";
  import { pickInputFile, loadAndAnalyze } from "../ipc.js";
  import { setPipeline, pipeline } from "../stores/pipeline.svelte.js";
  import { setStage } from "../stores/stage.svelte.js";
  import { progress, setBusy } from "../stores/progress.svelte.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import * as Select from "$lib/components/ui/select/index.js";
  import * as Alert from "$lib/components/ui/alert/index.js";

  let inputPath = $state("");
  let encodingOverride = $state("auto"); // "auto" = 自动探测
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
      const override = encodingOverride === "auto" ? null : encodingOverride;
      const dto = await loadAndAnalyze(inputPath, override);
      setPipeline(dto, inputPath);
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

  const encodingOptions = [
    { value: "auto", label: "自动探测(推荐)" },
    { value: "UTF-8", label: "UTF-8" },
    { value: "GBK", label: "GBK" },
    { value: "GB18030", label: "GB18030" },
    { value: "UTF-16LE", label: "UTF-16LE" },
    { value: "UTF-16BE", label: "UTF-16BE" },
  ];
  const encodingLabel = $derived(
    encodingOptions.find((o) => o.value === encodingOverride)?.label ?? "",
  );
</script>

<div class="flex flex-col gap-4 p-4">
  <h2 class="text-sm font-semibold">1. 文本选择</h2>

  <div class="flex flex-col gap-1.5">
    <Label for="input-path">txt 文件</Label>
    <div class="flex gap-1.5">
      <Input id="input-path" readonly bind:value={inputPath} placeholder="点击选择…" />
      <Button variant="outline" size="sm" onclick={onPick} disabled={progress.busy}>
        <FolderOpen />
        选择
      </Button>
    </div>
  </div>

  <div class="flex flex-col gap-1.5">
    <Label for="encoding">编码</Label>
    <Select.Root type="single" bind:value={encodingOverride} disabled={progress.busy}>
      <Select.Trigger id="encoding">{encodingLabel}</Select.Trigger>
      <Select.Content>
        {#each encodingOptions as opt}
          <Select.Item value={opt.value} label={opt.label} />
        {/each}
      </Select.Content>
    </Select.Root>
  </div>

  <Button class="w-full" onclick={onLoad} disabled={progress.busy || !inputPath}>
    {pipeline.dto ? "重新加载并分析" : "加载并分析"}
  </Button>

  {#if error}
    <Alert.Root variant="destructive">
      <AlertCircle />
      <Alert.Description>{error}</Alert.Description>
    </Alert.Root>
  {/if}

  {#if pipeline.dto}
    <div class="rounded-lg border bg-card p-3">
      <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
        分析结果
      </h3>
      <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
        <dt class="text-muted-foreground">实际编码</dt>
        <dd class="m-0 font-mono">{pipeline.dto.source_encoding}</dd>
        <dt class="text-muted-foreground">源文本</dt>
        <dd class="m-0 font-mono">{pipeline.dto.source_text.length.toLocaleString()} 字符</dd>
        <dt class="text-muted-foreground">清洗标注</dt>
        <dd class="m-0 font-mono">{pipeline.dto.cleaning.length} 条</dd>
        <dt class="text-muted-foreground">顶层条目</dt>
        <dd class="m-0 font-mono">{pipeline.dto.book.entries.length}</dd>
      </dl>
      <p class="my-2 text-[11px] text-muted-foreground">
        切到「文本处理」「章节分析」「样式预览与导出」继续。
      </p>
      <Button variant="outline" size="sm" class="w-full" onclick={reload}>
        用当前编码重跑
      </Button>
    </div>
  {/if}
</div>
