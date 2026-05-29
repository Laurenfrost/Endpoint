<script module>
  import { tv } from "tailwind-variants";

  export const badgeVariants = tv({
    base: "inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2",
    variants: {
      variant: {
        default: "border-transparent bg-primary text-primary-foreground hover:bg-primary/80",
        secondary: "border-transparent bg-secondary text-secondary-foreground hover:bg-secondary/80",
        destructive: "border-transparent bg-destructive text-destructive-foreground hover:bg-destructive/80",
        outline: "text-foreground",
        success: "border-transparent bg-emerald-500/15 text-emerald-700 dark:text-emerald-300",
        warning: "border-transparent bg-amber-500/15 text-amber-700 dark:text-amber-300",
        info: "border-transparent bg-blue-500/15 text-blue-700 dark:text-blue-300",
      },
    },
    defaultVariants: { variant: "default" },
  });
</script>

<script>
  import { cn } from "$lib/utils.js";
  import { badgeVariants as bv } from "./badge.svelte";

  let { class: className, variant, children, ref = $bindable(null), ...rest } = $props();
</script>

<span bind:this={ref} class={cn(bv({ variant }), className)} {...rest}>
  {@render children?.()}
</span>
