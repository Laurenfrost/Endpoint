<script>
  import { Progress as ProgressPrimitive } from "bits-ui";
  import { cn } from "$lib/utils.js";

  let {
    class: className,
    value = 0,
    max = 100,
    ref = $bindable(null),
    ...rest
  } = $props();
</script>

<ProgressPrimitive.Root
  bind:ref
  {value}
  {max}
  class={cn(
    "relative h-2 w-full overflow-hidden rounded-full bg-secondary",
    className,
  )}
  {...rest}
>
  <div
    class="h-full w-full flex-1 bg-primary transition-all"
    style="transform: translateX(-{100 - (100 * (value ?? 0)) / (max || 100)}%)"
  ></div>
</ProgressPrimitive.Root>
