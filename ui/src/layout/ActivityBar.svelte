<script>
  // 最左 ~56px 活动栏。点击切阶段。
  import { stage, setStage, STAGE_DEFS } from "../stores/stage.svelte.js";
  import { pipeline } from "../stores/pipeline.svelte.js";

  // 阶段 2-4 需要先加载 pipeline 才能访问。
  const stageEnabled = (id) => id === 1 || !!pipeline.dto;
</script>

<nav class="activity-bar" aria-label="阶段导航">
  {#each STAGE_DEFS as def (def.id)}
    {@const active = stage.id === def.id}
    {@const enabled = stageEnabled(def.id)}
    <button
      class="item"
      class:active
      class:disabled={!enabled}
      title={enabled ? def.label : `${def.label}(需先加载文件)`}
      onclick={() => enabled && setStage(def.id)}
      disabled={!enabled}
    >
      <span class="icon" aria-hidden="true">{def.icon}</span>
      <span class="badge">{def.id}</span>
    </button>
  {/each}
</nav>

<style>
  .activity-bar {
    width: 56px;
    background: #2c3440;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    padding-top: 8px;
    flex-shrink: 0;
  }
  .item {
    background: transparent;
    border: none;
    color: #cbd2d9;
    padding: 12px 0 14px 0;
    cursor: pointer;
    border-left: 2px solid transparent;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
  }
  .item:hover:not(.disabled) { background: #3a4452; }
  .item.active {
    color: #fff;
    border-left-color: #1f6feb;
    background: #3a4452;
  }
  .item.disabled { opacity: 0.35; cursor: not-allowed; }
  .icon { font-size: 22px; line-height: 1; }
  .badge {
    font-size: 9px;
    opacity: 0.6;
    font-family: Consolas, "Cascadia Mono", monospace;
  }
</style>
