<script>
  // 明暗模式切换:三态(system → light → dark → system),图标随当前实际模式变化。
  import Sun from "@lucide/svelte/icons/sun";
  import Moon from "@lucide/svelte/icons/moon";
  import Monitor from "@lucide/svelte/icons/monitor";
  import { mode, userPrefersMode, setMode } from "mode-watcher";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";

  function cycle() {
    const next = userPrefersMode.current === "light"
      ? "dark"
      : userPrefersMode.current === "dark"
        ? "system"
        : "light";
    setMode(next);
  }

  const label = $derived(
    userPrefersMode.current === "system"
      ? `跟随系统(当前 ${mode.current === "dark" ? "深色" : "浅色"})`
      : userPrefersMode.current === "dark"
        ? "深色模式"
        : "浅色模式",
  );
</script>

<Tooltip.Provider delayDuration={200}>
  <Tooltip.Root>
    <Tooltip.Trigger>
      {#snippet child({ props })}
        <Button
          {...props}
          variant="ghost"
          size="icon-sm"
          onclick={cycle}
          class="text-activitybar-foreground/70 hover:bg-white/10 hover:text-activitybar-foreground"
          aria-label={label}
        >
          {#if userPrefersMode.current === "system"}
            <Monitor />
          {:else if mode.current === "dark"}
            <Moon />
          {:else}
            <Sun />
          {/if}
        </Button>
      {/snippet}
    </Tooltip.Trigger>
    <Tooltip.Content side="right">{label}</Tooltip.Content>
  </Tooltip.Root>
</Tooltip.Provider>
