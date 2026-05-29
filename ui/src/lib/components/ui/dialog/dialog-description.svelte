<script>
  import { Dialog as DialogPrimitive } from "bits-ui";
  import { cn } from "$lib/utils.js";

  let { class: className, children, ref = $bindable(null), ...rest } = $props();
</script>

<DialogPrimitive.Description
  bind:ref
  class={cn("text-xs text-muted-foreground", className)}
  {...rest}
>
  {@render children?.()}
</DialogPrimitive.Description>
