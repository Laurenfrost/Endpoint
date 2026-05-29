<script>
  import { Tabs as TabsPrimitive } from "bits-ui";
  import { cn } from "$lib/utils.js";

  let {
    class: className,
    value = $bindable(""),
    children,
    ref = $bindable(null),
    ...rest
  } = $props();
</script>

<TabsPrimitive.Root bind:ref bind:value class={cn("flex flex-col gap-2", className)} {...rest}>
  {@render children?.()}
</TabsPrimitive.Root>
