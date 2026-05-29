<script>
  import { Dialog as DialogPrimitive } from "bits-ui";
  import { cn } from "$lib/utils.js";

  let { class: className, children, ref = $bindable(null), ...rest } = $props();
</script>

<DialogPrimitive.Title
  bind:ref
  class={cn("text-base font-semibold leading-none tracking-tight", className)}
  {...rest}
>
  {@render children?.()}
</DialogPrimitive.Title>
