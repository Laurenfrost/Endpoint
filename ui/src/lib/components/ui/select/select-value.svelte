<script>
  import { Select as SelectPrimitive } from "bits-ui";

  let { ref = $bindable(null), ...rest } = $props();
</script>

<SelectPrimitive.Value bind:ref {...rest} />
