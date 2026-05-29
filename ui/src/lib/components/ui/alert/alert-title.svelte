<script>
  import { cn } from "$lib/utils.js";

  let { class: className, children, ref = $bindable(null), ...rest } = $props();
</script>

<h5 bind:this={ref} class={cn("mb-1 font-medium leading-none tracking-tight", className)} {...rest}>
  {@render children?.()}
</h5>
