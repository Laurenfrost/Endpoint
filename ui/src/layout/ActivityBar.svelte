<script>
  // 最左 ~56px 活动栏。上方:四个阶段;下方:设置入口 + LLM 状态指示。
  import { stage, setStage, toggleSettings, STAGE_DEFS } from "../stores/stage.svelte.js";
  import { pipeline } from "../stores/pipeline.svelte.js";
  import { llm } from "../stores/llm.svelte.js";

  // 阶段 2-4 需要先加载 pipeline 才能访问。
  const stageEnabled = (id) => id === 1 || !!pipeline.dto;

  // 高亮规则:view 为 stage 时高亮当前 stage;view 为 settings 时四个阶段都不亮。
  const isStageActive = (id) => stage.view === "stage" && stage.id === id;
</script>

<nav class="activity-bar" aria-label="导航">
  {#each STAGE_DEFS as def (def.id)}
    {@const active = isStageActive(def.id)}
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

  <div class="spacer"></div>

  <button
    class="item settings"
    class:active={stage.view === "settings"}
    title="设置(LLM / 搜索 / kepubify)"
    onclick={toggleSettings}
    aria-label="设置"
  >
    <span class="icon" aria-hidden="true">⚙</span>
    <span class="llm-dot" class:on={llm.configured}
      title={llm.configured ? `LLM 已配置: ${llm.model || llm.baseUrl}` : "LLM 未配置"}></span>
  </button>
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
  .spacer { flex: 1; }
  .item.settings {
    padding: 10px 0 12px 0;
    position: relative;
  }
  .item.settings .icon { font-size: 20px; }
  .item.settings .llm-dot {
    position: absolute;
    bottom: 8px;
    right: 12px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #52606d;
    box-shadow: 0 0 0 2px #2c3440;
    transition: background 0.2s;
  }
  .item.settings.active .llm-dot { box-shadow: 0 0 0 2px #3a4452; }
  .item.settings .llm-dot.on { background: #4caf50; }
</style>
