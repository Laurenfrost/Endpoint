<script>
  import { cn } from "$lib/utils.js";

  let { class: className, children, ref = $bindable(null), ...rest } = $props();
</script>

<div bind:this={ref} class={cn("text-xs [&_p]:leading-relaxed whitespace-pre-wrap", className)} {...rest}>
  {@render children?.()}
</div>
