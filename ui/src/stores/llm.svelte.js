// LLM 配置状态。由 Stage4Export 或 App.svelte 在启动时从后端加载。
// `configured` = API key 已填写且 base_url 非空。
export const llm = $state({ configured: false, baseUrl: "", model: "", keyMasked: "" });

/** 把后端 get_llm_config 返回值更新到 store。 */
export function applyLlmConfig(cfg) {
  llm.configured = cfg.key_set ?? false;
  llm.baseUrl = cfg.base_url ?? "";
  llm.model = cfg.model ?? "";
  llm.keyMasked = cfg.key_masked ?? "";
}
