<script>
  import { Tooltip as TooltipPrimitive } from "bits-ui";

  let {
    delayDuration = 200,
    children,
    ...rest
  } = $props();
</script>

<TooltipPrimitive.Root {delayDuration} {...rest}>
  {@render children?.()}
</TooltipPrimitive.Root>
