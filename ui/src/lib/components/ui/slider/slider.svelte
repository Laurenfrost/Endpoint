<script>
  import { Slider as SliderPrimitive } from "bits-ui";
  import { cn } from "$lib/utils.js";

  let {
    class: className,
    value = $bindable(0),
    min = 0,
    max = 100,
    step = 1,
    ref = $bindable(null),
    ...rest
  } = $props();
</script>

<SliderPrimitive.Root
  type="single"
  bind:ref
  bind:value
  {min}
  {max}
  {step}
  class={cn(
    "relative flex w-full touch-none select-none items-center",
    className,
  )}
  {...rest}
>
  {#snippet children({ thumbs })}
    <span
      class="relative h-2 w-full grow overflow-hidden rounded-full bg-secondary"
      data-slot="track"
    >
      <SliderPrimitive.Range class="absolute h-full bg-primary" />
    </span>
    {#each thumbs as i (i)}
      <SliderPrimitive.Thumb
        index={i}
        class="block h-4 w-4 rounded-full border-2 border-primary bg-background ring-offset-background transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50"
      />
    {/each}
  {/snippet}
</SliderPrimitive.Root>
