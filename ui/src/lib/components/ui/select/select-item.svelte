<script>
  import { Select as SelectPrimitive } from "bits-ui";
  import Check from "@lucide/svelte/icons/check";
  import { cn } from "$lib/utils.js";

  let {
    class: className,
    value,
    label,
    children,
    ref = $bindable(null),
    ...rest
  } = $props();
</script>

<SelectPrimitive.Item
  bind:ref
  {value}
  {label}
  class={cn(
    "relative flex w-full cursor-default select-none items-center rounded-sm py-1.5 pl-2 pr-8 text-sm outline-none focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50 data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground",
    className,
  )}
  {...rest}
>
  {#snippet children({ selected })}
    <span class="absolute right-2 flex h-3.5 w-3.5 items-center justify-center">
      {#if selected}
        <Check class="h-4 w-4" />
      {/if}
    </span>
    {label ?? value}
  {/snippet}
</SelectPrimitive.Item>
