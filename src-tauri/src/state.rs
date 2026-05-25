//! Tauri 管理的全局应用状态。
//!
//! 阶段二的工作模式:
//! 1. 前端按 `load_and_analyze` 加载并解析一份 txt;命令把完整 [`PipelineOutput`]
//!    存入 [`AppState::pipeline`],并把 JSON DTO 返回给前端。
//! 2. 前端在阶段 1-3 操作 UI 时只消费 DTO 中的标注 + 源文本,**不**再触发后端。
//! 3. 前端进入阶段 4 点击「生成」时调 `build_epub`;命令从 [`AppState::pipeline`]
//!    取已缓存的完整 [`PipelineOutput`](含 `paragraphs` 等不进 IPC 的字段)写 EPUB。
//!
//! 取消标志注册表 [`AppState::cancel_flags`] 是 v1 的占位:`cancel_task` 命令置位,
//! 核心库长循环里的 `TODO(cancel)` 注释指向未来要检查这里。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use endpoint_core::PipelineOutput;

pub struct AppState {
    /// 最近一次 `load_and_analyze` 的结果。reload 时整体替换。
    /// 体积可达数 MB(source_text + paragraphs),但只持一份,内存压力可控。
    pub pipeline: Mutex<Option<CachedPipeline>>,
    /// 取消标志注册表(task_id → AtomicBool)。v1 只插入与置位,核心库不真正消费。
    pub cancel_flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// 任务 id 单调计数器。
    counter: AtomicU64,
    /// 当前生效的 LLM 客户端。初始化时从 `config.toml` 加载;`set_llm_config` 命令可替换。
    /// `NoopLlmClient` 表示未配置。
    pub llm_client: Mutex<Box<dyn endpoint_core::llm::LlmClient>>,
}

impl Default for AppState {
    fn default() -> Self {
        let cfg = crate::llm_config::load();
        Self {
            pipeline: Mutex::new(None),
            cancel_flags: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(0),
            llm_client: Mutex::new(crate::llm_config::create_client(&cfg)),
        }
    }
}

pub struct CachedPipeline {
    /// 记录加载源,便于将来做「重新载入同一文件」「跨命令一致性校验」等。
    /// v1 命令未读取此字段——保留是为了后续无需改 state 形状。
    #[allow(dead_code)]
    pub source_path: String,
    pub output: PipelineOutput,
}

impl AppState {
    /// 分配一个递增 task_id,形如 `"load-3"` / `"build-7"`。
    pub fn next_task_id(&self, prefix: &str) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}-{n}")
    }

    /// 为某 task 注册取消标志。返回的 Arc 给后台任务持有(v1 暂不消费)。
    pub fn register_cancel(&self, task_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.cancel_flags
            .lock()
            .expect("cancel_flags 锁中毒")
            .insert(task_id.to_string(), Arc::clone(&flag));
        flag
    }

    /// 任务结束(成功/失败/取消)后清除该 task 的标志。
    pub fn unregister_cancel(&self, task_id: &str) {
        self.cancel_flags
            .lock()
            .expect("cancel_flags 锁中毒")
            .remove(task_id);
    }
}
