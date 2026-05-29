<script module>
  import { tv } from "tailwind-variants";

  export const alertVariants = tv({
    base: "relative w-full rounded-lg border px-3 py-2 text-xs [&>svg]:size-4 [&>svg]:text-foreground [&>svg+div]:translate-y-[-2px] [&>svg]:absolute [&>svg]:left-3 [&>svg]:top-2.5 [&>svg~*]:pl-6",
    variants: {
      variant: {
        default: "bg-background text-foreground",
        destructive: "border-destructive/50 text-destructive bg-destructive/10 [&>svg]:text-destructive",
        warning: "border-amber-500/50 text-amber-700 dark:text-amber-300 bg-amber-500/10 [&>svg]:text-amber-700 dark:[&>svg]:text-amber-300",
        info: "border-blue-500/50 text-blue-700 dark:text-blue-300 bg-blue-500/10 [&>svg]:text-blue-700 dark:[&>svg]:text-blue-300",
      },
    },
    defaultVariants: { variant: "default" },
  });
</script>

<script>
  import { cn } from "$lib/utils.js";
  import { alertVariants as av } from "./alert.svelte";

  let { class: className, variant, children, ref = $bindable(null), ...rest } = $props();
</script>

<div bind:this={ref} role="alert" class={cn(av({ variant }), className)} {...rest}>
  {@render children?.()}
</div>
