<script module>
  import { tv } from "tailwind-variants";

  export const buttonVariants = tv({
    base: "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium ring-offset-background transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 cursor-pointer [&_svg]:size-4 [&_svg]:shrink-0",
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:bg-primary/90",
        destructive: "bg-destructive text-destructive-foreground hover:bg-destructive/90",
        outline: "border border-input bg-background hover:bg-accent hover:text-accent-foreground",
        secondary: "bg-secondary text-secondary-foreground hover:bg-secondary/80",
        ghost: "hover:bg-accent hover:text-accent-foreground",
        link: "text-primary underline-offset-4 hover:underline",
      },
      size: {
        default: "h-9 px-4 py-2",
        sm: "h-8 rounded-md px-3 text-xs",
        lg: "h-10 rounded-md px-6",
        icon: "h-9 w-9",
        "icon-sm": "h-8 w-8",
      },
    },
    defaultVariants: { variant: "default", size: "default" },
  });
</script>

<script>
  import { cn } from "$lib/utils.js";
  import { buttonVariants as bv } from "./button.svelte";

  let {
    class: className,
    variant,
    size,
    type = "button",
    href = undefined,
    children,
    ref = $bindable(null),
    ...rest
  } = $props();
</script>

{#if href}
  <a {href} bind:this={ref} class={cn(bv({ variant, size }), className)} {...rest}>
    {@render children?.()}
  </a>
{:else}
  <button {type} bind:this={ref} class={cn(bv({ variant, size }), className)} {...rest}>
    {@render children?.()}
  </button>
{/if}
