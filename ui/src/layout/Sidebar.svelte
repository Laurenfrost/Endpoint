<script>
  import { stage } from "../stores/stage.svelte.js";
  import Stage1Input from "../stages/Stage1Input.svelte";
  import Stage2Cleaning from "../stages/Stage2Cleaning.svelte";
  import Stage3Chapter from "../stages/Stage3Chapter.svelte";
  import Stage4Export from "../stages/Stage4Export.svelte";
</script>

<aside class="sidebar">
  {#if stage.id === 1}
    <Stage1Input />
  {:else if stage.id === 2}
    <Stage2Cleaning />
  {:else if stage.id === 3}
    <Stage3Chapter />
  {:else if stage.id === 4}
    <Stage4Export />
  {/if}
</aside>

<style>
  .sidebar {
    width: 320px;
    background: #f5f7fa;
    border-right: 1px solid #cbd2d9;
    overflow-y: auto;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
  }
</style>
