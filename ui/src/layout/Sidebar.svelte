<script>
  import { stage } from "../stores/stage.svelte.js";
  import Stage1Input from "../stages/Stage1Input.svelte";
  import Stage2Cleaning from "../stages/Stage2Cleaning.svelte";
  import Stage3Chapter from "../stages/Stage3Chapter.svelte";
  import Stage4Export from "../stages/Stage4Export.svelte";
  import Settings from "../stages/Settings.svelte";
</script>

<aside class="flex w-80 shrink-0 flex-col overflow-y-auto border-r bg-sidebar text-sidebar-foreground">
  {#if stage.view === "settings"}
    <Settings />
  {:else if stage.id === 1}
    <Stage1Input />
  {:else if stage.id === 2}
    <Stage2Cleaning />
  {:else if stage.id === 3}
    <Stage3Chapter />
  {:else if stage.id === 4}
    <Stage4Export />
  {/if}
</aside>
