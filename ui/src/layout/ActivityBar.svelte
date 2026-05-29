<script>
  // 最左 ~56px 活动栏。上方:四个阶段;下方:设置入口 + LLM 状态指示 + 主题切换。
  import Settings from "@lucide/svelte/icons/settings";
  import { stage, setStage, toggleSettings, STAGE_DEFS } from "../stores/stage.svelte.js";
  import { pipeline } from "../stores/pipeline.svelte.js";
  import { llm } from "../stores/llm.svelte.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";
  import ModeToggle from "$lib/components/mode-toggle.svelte";
  import { cn } from "$lib/utils.js";

  // 阶段 2-4 需要先加载 pipeline 才能访问。
  const stageEnabled = (id) => id === 1 || !!pipeline.dto;
  // 高亮规则:view 为 stage 时高亮当前 stage;view 为 settings 时四个阶段都不亮。
  const isStageActive = (id) => stage.view === "stage" && stage.id === id;
</script>

<Tooltip.Provider delayDuration={200}>
  <nav
    class="flex w-14 shrink-0 flex-col items-stretch gap-1 bg-activitybar pt-2 pb-2 text-activitybar-foreground"
    aria-label="导航"
  >
    {#each STAGE_DEFS as def (def.id)}
      {@const active = isStageActive(def.id)}
      {@const enabled = stageEnabled(def.id)}
      {@const Icon = def.icon}
      <Tooltip.Root>
        <Tooltip.Trigger>
          {#snippet child({ props })}
            <button
              {...props}
              type="button"
              class={cn(
                "relative flex flex-col items-center gap-0.5 border-l-2 border-transparent py-2.5 outline-none transition-colors",
                enabled ? "cursor-pointer hover:bg-white/10" : "cursor-not-allowed opacity-35",
                active && "border-l-primary bg-white/10 text-activitybar-foreground",
                !active && enabled && "text-activitybar-foreground/70 hover:text-activitybar-foreground",
              )}
              onclick={() => enabled && setStage(def.id)}
              disabled={!enabled}
              aria-label={def.label}
            >
              <Icon class="size-5" />
              <span class="font-mono text-[9px] opacity-60">{def.id}</span>
            </button>
          {/snippet}
        </Tooltip.Trigger>
        <Tooltip.Content side="right">
          {enabled ? def.label : `${def.label}(需先加载文件)`}
        </Tooltip.Content>
      </Tooltip.Root>
    {/each}

    <div class="flex-1"></div>

    <div class="flex flex-col items-center gap-1 pb-1">
      <ModeToggle />

      <Tooltip.Root>
        <Tooltip.Trigger>
          {#snippet child({ props })}
            <button
              {...props}
              type="button"
              class={cn(
                "relative flex items-center justify-center rounded-md p-2 outline-none transition-colors",
                stage.view === "settings"
                  ? "bg-white/10 text-activitybar-foreground"
                  : "text-activitybar-foreground/70 hover:bg-white/10 hover:text-activitybar-foreground",
              )}
              onclick={toggleSettings}
              aria-label="设置"
            >
              <Settings class="size-5" />
              <span
                class={cn(
                  "absolute bottom-1 right-1 size-1.5 rounded-full ring-2 transition-colors",
                  llm.configured ? "bg-emerald-500" : "bg-zinc-500",
                  stage.view === "settings" ? "ring-white/10" : "ring-activitybar",
                )}
              ></span>
            </button>
          {/snippet}
        </Tooltip.Trigger>
        <Tooltip.Content side="right">
          {llm.configured ? `LLM 已配置: ${llm.model || llm.baseUrl}` : "LLM 未配置"}
        </Tooltip.Content>
      </Tooltip.Root>
    </div>
  </nav>
</Tooltip.Provider>
