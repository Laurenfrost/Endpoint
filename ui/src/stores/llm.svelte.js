// LLM + 搜索配置状态。由 Stage4Export 或 App.svelte 在启动时从后端加载。
// - `configured` = LLM 真正可用(base_url + api_key 都非空)
// - `searchConfigured` = Pass B 搜索可用(provider + api_key 都非空)
export const llm = $state({
  configured: false,
  baseUrl: "",
  model: "",
  keyMasked: "",
  searchProvider: "",
  searchKeyMasked: "",
  searchConfigured: false,
});

/** 把后端 get_llm_config 返回值更新到 store。 */
export function applyLlmConfig(cfg) {
  llm.configured = cfg.configured ?? cfg.key_set ?? false;
  llm.baseUrl = cfg.base_url ?? "";
  llm.model = cfg.model ?? "";
  llm.keyMasked = cfg.key_masked ?? "";
  llm.searchProvider = cfg.search_provider ?? "";
  llm.searchKeyMasked = cfg.search_key_masked ?? "";
  llm.searchConfigured = cfg.search_configured ?? false;
}
