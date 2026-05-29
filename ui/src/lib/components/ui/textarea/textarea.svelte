<script>
  import { cn } from "$lib/utils.js";

  let {
    class: className,
    value = $bindable(""),
    ref = $bindable(null),
    ...rest
  } = $props();
</script>

<textarea
  bind:value
  bind:this={ref}
  class={cn(
    "flex min-h-[60px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50",
    className,
  )}
  {...rest}
></textarea>
