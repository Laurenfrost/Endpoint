<script>
  import { Select as SelectPrimitive } from "bits-ui";

  let {
    value = $bindable(""),
    type = "single",
    onValueChange,
    children,
    ...rest
  } = $props();
</script>

<SelectPrimitive.Root {type} bind:value {onValueChange} {...rest}>
  {@render children?.()}
</SelectPrimitive.Root>
