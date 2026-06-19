use crate::models::ScheduleAlgorithm;
use crate::state::AppState;
use rand::Rng;
use tracing::{debug, warn};

/// 调度器 - 选择下一个可用的端点
pub struct Scheduler;

impl Scheduler {
    /// 根据调度算法选择端点（指定池）
    pub fn select_endpoint(
        state: &AppState,
        pool_id: &str,
        algorithm: &ScheduleAlgorithm,
    ) -> Option<String> {
        let available = state.available_endpoint_ids_in_pool(pool_id);
        if available.is_empty() {
            warn!("池 {} 中没有可用的端点", pool_id);
            return None;
        }

        match algorithm {
            ScheduleAlgorithm::RoundRobin => Self::round_robin(state, pool_id, &available),
            ScheduleAlgorithm::Failover => Self::failover(state, pool_id, &available),
            ScheduleAlgorithm::Random => Self::random(&available),
        }
    }

    /// 轮询算法：依次选择下一个可用端点
    fn round_robin(state: &AppState, pool_id: &str, available: &[String]) -> Option<String> {
        let all_endpoints: Vec<String> = {
            let endpoints = state.endpoints.read();
            endpoints.values()
                .filter(|ep| ep.config.pool_id.as_deref() == Some(pool_id))
                .map(|ep| ep.config.id.clone())
                .collect()
        };

        if all_endpoints.is_empty() {
            return None;
        }

        let total = all_endpoints.len();
        let current_index = state.get_scheduler_index(pool_id);
        let start = current_index % total;

        for i in 0..total {
            let idx = (start + i) % total;
            let ep_id = &all_endpoints[idx];
            if available.contains(ep_id) {
                state.update_scheduler_index(pool_id, (idx + 1) % total);
                debug!("轮询选择端点: {}", ep_id);
                return Some(ep_id.clone());
            }
        }

        None
    }

    /// 轮换算法：优先使用当前活跃端点，耗尽后切换到下一个
    fn failover(state: &AppState, pool_id: &str, available: &[String]) -> Option<String> {
        let all_endpoints: Vec<String> = {
            let endpoints = state.endpoints.read();
            let mut keys: Vec<String> = endpoints.values()
                .filter(|ep| ep.config.pool_id.as_deref() == Some(pool_id))
                .map(|ep| ep.config.id.clone())
                .collect();
            keys.sort();
            keys
        };

        if all_endpoints.is_empty() {
            return None;
        }

        let total = all_endpoints.len();
        let current_index = state.get_failover_index(pool_id);
        let start = current_index % total;

        for i in 0..total {
            let idx = (start + i) % total;
            let ep_id = &all_endpoints[idx];
            if available.contains(ep_id) {
                if idx != start {
                    state.update_failover_index(pool_id, idx);
                    debug!("轮换切换到端点: {}", ep_id);
                } else {
                    debug!("轮换继续使用端点: {}", ep_id);
                }
                return Some(ep_id.clone());
            }
        }

        None
    }

    /// 随机算法：随机选择一个可用端点
    fn random(available: &[String]) -> Option<String> {
        if available.is_empty() {
            return None;
        }
        let mut rng = rand::thread_rng();
        let idx = rng.gen_range(0..available.len());
        let selected = &available[idx];
        debug!("随机选择端点: {}", selected);
        Some(selected.clone())
    }

    /// 随机重试：当请求失败时，选择下一个可用端点（排除当前失败的）
    pub fn select_next_for_retry(
        state: &AppState,
        pool_id: &str,
        failed_id: &str,
    ) -> Option<String> {
        let available: Vec<String> = state
            .available_endpoint_ids_in_pool(pool_id)
            .into_iter()
            .filter(|id| id != failed_id)
            .collect();

        if available.is_empty() {
            warn!("池 {} 中没有其他可用的端点进行重试", pool_id);
            return None;
        }

        let mut rng = rand::thread_rng();
        let idx = rng.gen_range(0..available.len());
        let selected = &available[idx];
        debug!("重试选择端点: {} (排除失败的 {})", selected, failed_id);
        Some(selected.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigManager;
    use crate::models::*;
    use std::sync::Arc;

    async fn create_test_state(endpoints: Vec<EndpointConfig>) -> Arc<AppState> {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        std::mem::forget(tmp);

        let config_manager = ConfigManager::new(Some(&path));
        let mut config = AppConfig::default();
        config.endpoints = endpoints;
        config_manager.save(&config).await.unwrap();

        Arc::new(AppState::new(config_manager).await.unwrap())
    }

    fn make_endpoint(id: &str, pool_id: &str, enabled: bool, limit: u64) -> EndpointConfig {
        EndpointConfig {
            id: id.to_string(),
            name: format!("Test {}", id),
            url: "http://localhost".to_string(),
            api_type: ApiType::OpenAI,
            api_key: "key".to_string(),
            token_limit: limit,
            reset_policy: ResetPolicy::Manual,
            enabled,
            pool_id: Some(pool_id.to_string()),
            timeout: 300,
            model_mappings: vec![],
        }
    }

    #[actix_rt::test]
    async fn test_round_robin_in_pool() {
        let ep1 = make_endpoint("ep1", "pool1", true, 1000);
        let ep2 = make_endpoint("ep2", "pool1", true, 1000);
        let ep3 = make_endpoint("ep3", "pool1", true, 1000);
        let ep4 = make_endpoint("ep4", "pool2", true, 1000); // 不同池

        let state = create_test_state(vec![ep1, ep2, ep3, ep4]).await;

        let mut selected = Vec::new();
        for _ in 0..6 {
            if let Some(id) = Scheduler::select_endpoint(&state, "pool1", &ScheduleAlgorithm::RoundRobin) {
                selected.push(id);
            }
        }

        assert_eq!(selected.len(), 6);
        // 应该只选择 pool1 中的端点
        for id in &selected {
            assert!(id.starts_with("ep") && id != "ep4");
        }
    }

    #[actix_rt::test]
    async fn test_no_available_endpoints_in_pool() {
        let ep1 = make_endpoint("ep1", "pool1", false, 1000);
        let state = create_test_state(vec![ep1]).await;

        let result = Scheduler::select_endpoint(&state, "pool1", &ScheduleAlgorithm::RoundRobin);
        assert!(result.is_none());
    }

    #[actix_rt::test]
    async fn test_skip_exhausted_endpoint_in_pool() {
        let ep1 = make_endpoint("ep1", "pool1", true, 100);
        let ep2 = make_endpoint("ep2", "pool1", true, 1000);

        let state = create_test_state(vec![ep1, ep2]).await;

        // 手动设置 ep1 的 tokens_used 为限额值
        {
            let mut endpoints = state.endpoints.write();
            if let Some(ep) = endpoints.get_mut("ep1") {
                ep.tokens_used = 100;
            }
        }

        let result = Scheduler::select_endpoint(&state, "pool1", &ScheduleAlgorithm::RoundRobin);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "ep2");
    }
}
