<script>
  import { Tooltip as TooltipPrimitive } from "bits-ui";

  let { delayDuration = 200, children, ...rest } = $props();
</script>

<TooltipPrimitive.Provider {delayDuration} {...rest}>
  {@render children?.()}
</TooltipPrimitive.Provider>
