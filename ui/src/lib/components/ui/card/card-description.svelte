<script>
  import { cn } from "$lib/utils.js";

  let { class: className, children, ref = $bindable(null), ...rest } = $props();
</script>

<p bind:this={ref} class={cn("text-xs text-muted-foreground", className)} {...rest}>
  {@render children?.()}
</p>
