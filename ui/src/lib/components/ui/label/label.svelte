<script>
  import { Label as LabelPrimitive } from "bits-ui";
  import { cn } from "$lib/utils.js";

  let {
    class: className,
    for: htmlFor,
    children,
    ref = $bindable(null),
    ...rest
  } = $props();
</script>

<LabelPrimitive.Root
  bind:ref
  for={htmlFor}
  class={cn(
    "text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70",
    className,
  )}
  {...rest}
>
  {@render children?.()}
</LabelPrimitive.Root>
