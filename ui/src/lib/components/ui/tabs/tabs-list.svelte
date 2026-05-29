<script>
  import { Tabs as TabsPrimitive } from "bits-ui";
  import { cn } from "$lib/utils.js";

  let { class: className, children, ref = $bindable(null), ...rest } = $props();
</script>

<TabsPrimitive.List
  bind:ref
  class={cn(
    "inline-flex h-9 items-center justify-center rounded-lg bg-muted p-1 text-muted-foreground",
    className,
  )}
  {...rest}
>
  {@render children?.()}
</TabsPrimitive.List>
