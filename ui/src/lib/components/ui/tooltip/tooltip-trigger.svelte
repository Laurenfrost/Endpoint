<script>
  import { Tooltip as TooltipPrimitive } from "bits-ui";

  let { children, ref = $bindable(null), ...rest } = $props();
</script>

<TooltipPrimitive.Trigger bind:ref {...rest}>
  {@render children?.()}
</TooltipPrimitive.Trigger>
