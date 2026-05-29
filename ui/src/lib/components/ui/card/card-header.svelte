<script>
  import { cn } from "$lib/utils.js";

  let { class: className, children, ref = $bindable(null), ...rest } = $props();
</script>

<div bind:this={ref} class={cn("flex flex-col space-y-1.5 p-4", className)} {...rest}>
  {@render children?.()}
</div>
