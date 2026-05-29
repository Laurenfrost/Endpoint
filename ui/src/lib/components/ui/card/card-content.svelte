<script>
  import { cn } from "$lib/utils.js";

  let { class: className, children, ref = $bindable(null), ...rest } = $props();
</script>

<div bind:this={ref} class={cn("p-4 pt-0", className)} {...rest}>
  {@render children?.()}
</div>
