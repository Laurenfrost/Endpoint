<script>
  import { cn } from "$lib/utils.js";

  let { class: className, children, ref = $bindable(null), ...rest } = $props();
</script>

<h3 bind:this={ref} class={cn("text-base font-semibold leading-none tracking-tight", className)} {...rest}>
  {@render children?.()}
</h3>
