<script>
  import { cn } from "$lib/utils.js";

  let {
    class: className,
    children,
    ref = $bindable(null),
    ...rest
  } = $props();
</script>

<div
  bind:this={ref}
  class={cn(
    "rounded-lg border bg-card text-card-foreground shadow-sm",
    className,
  )}
  {...rest}
>
  {@render children?.()}
</div>
