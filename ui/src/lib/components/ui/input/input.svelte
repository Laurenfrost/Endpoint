<script>
  import { cn } from "$lib/utils.js";

  let {
    class: className,
    value = $bindable(""),
    type = "text",
    ref = $bindable(null),
    ...rest
  } = $props();
</script>

<input
  {type}
  bind:value
  bind:this={ref}
  class={cn(
    "flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-xs transition-colors file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50",
    className,
  )}
  {...rest}
/>
