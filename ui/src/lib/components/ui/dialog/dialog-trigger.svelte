<script>
  import { Dialog as DialogPrimitive } from "bits-ui";

  let { children, ref = $bindable(null), ...rest } = $props();
</script>

<DialogPrimitive.Trigger bind:ref {...rest}>
  {@render children?.()}
</DialogPrimitive.Trigger>
