// Codex Local Access 统一入口。
// 业务分片只在完整顶层 item 之间切开，通过 include! 保持同一模块作用域；
// 调用方、网关生命周期、请求转换和账号池行为均与拆分前一致。
include!("codex_local_access_foundation.rs");
include!("codex_local_access_quota_cooldown.rs");
include!("codex_local_access_request_transform.rs");
include!("codex_local_access_routing_pricing.rs");
include!("codex_local_access_request_logs.rs");
include!("codex_local_access_profile_takeover.rs");
include!("codex_local_access_takeover_maintenance.rs");
include!("codex_local_access_sidecar_config.rs");
include!("codex_local_access_automatic_routing.rs");
include!("codex_local_access_sidecar_runtime.rs");
include!("codex_local_access_collection.rs");
include!("codex_local_access_gateway_runtime.rs");
include!("codex_local_access_provider_gateway.rs");
include!("codex_local_access_instance_gateways.rs");
include!("codex_local_access_probe_chat.rs");
include!("codex_pelican_transport.rs");
include!("codex_local_access_commands.rs");
include!("codex_local_access_http.rs");

/// Proxy changes must not leave a live sidecar with an old direct/global route.
pub async fn ensure_account_proxy_edit_safe(account_id: &str) -> Result<(), String> {
    let runtime = gateway_runtime().lock().await;
    if (runtime.running || runtime.collection.as_ref().is_some_and(|c| c.enabled))
        && runtime.collection.as_ref().is_some_and(|c| effective_sidecar_account_ids(c).iter().any(|id| id == account_id)) {
        return Err("请先停止 API 服务再修改账号代理，保存后重新启动服务".into());
    }
    drop(runtime);
    let runtimes = provider_gateway_runtime_store().lock().await;
    if runtimes.values().any(|r| r.collection.as_ref().is_some_and(|c| effective_sidecar_account_ids(c).iter().any(|id| id == account_id))) {
        return Err("请先停止包含此账号的实例网关，再修改账号代理".into());
    }
    Ok(())
}
// The retired in-process WebSocket gateway is retained only as a test oracle.
// Production traffic is handled by the bundled CLIProxyAPI sidecar.
#[cfg(test)]
include!("codex_local_access_recovery.rs");

#[cfg(test)]
mod tests {
    #[test]
    fn deepseek_multi_agent_survives_profile_template_override() {
        let mut catalog = serde_json::json!({"models": [{"slug": "deepseek-flash"}]});
        let definitions = vec![super::ProfileModelDefinition {
            model_id: "deepseek-flash".into(),
            display_name: "DeepSeek-V4.1-Flash".into(),
            template: Some(serde_json::json!({"multi_agent_version": null,
                "supported_reasoning_levels": [{"effort": "max"}]})),
            image_capable: true,
        }];
        super::apply_profile_model_definition_overrides(&mut catalog, &definitions);
        assert_eq!(catalog["models"][0]["multi_agent_version"], "v2");
        assert_eq!(catalog["models"][0]["supported_reasoning_levels"][0]["effort"], "max");
        assert_eq!(catalog["models"][0]["input_modalities"], serde_json::json!(["text", "image"]));
    }
    include!("codex_local_access_tests_automatic_routing.rs");
    include!("codex_local_access_tests_sidecar_gateway.rs");
    include!("codex_local_access_tests_grok_lifecycle.rs");
    include!("codex_local_access_tests_pricing_profile.rs");
    include!("codex_local_access_tests_request_routing.rs");
    include!("codex_local_access_tests_provider_gateway_vision.rs");
    include!("codex_local_access_tests_takeover.rs");
    include!("codex_local_access_tests_takeover_maintenance.rs");
    include!("codex_local_access_tests_internal_service.rs");
}
