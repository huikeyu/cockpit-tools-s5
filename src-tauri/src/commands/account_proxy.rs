use crate::modules::{
    self,
    account_proxy::{self, ProxyStatus},
};
use serde::Serialize;
use std::time::{Duration, Instant};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyRequestActivity {
    timestamp: i64,
    request_id: String,
    model: String,
    requested_model: String,
    upstream_model: String,
    request_kind: String,
    gateway_mode: Option<String>,
    service_tier: Option<String>,
    reasoning_effort: Option<String>,
    success: bool,
    http_status: Option<u16>,
    latency_ms: u64,
    error_category: String,
    error_message: String,
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    estimated_cost_usd: f64,
    api_key_label: String,
    client_instance_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyAccountDashboard {
    proxy: ProxyStatus,
    logged_requests: u64,
    recent_requests: Vec<ProxyRequestActivity>,
    logs_available: bool,
}

#[tauri::command]
pub async fn codex_get_account_proxy_dashboard(
    account_id: String,
) -> Result<ProxyAccountDashboard, String> {
    require_account(&account_id)?;
    let status_id = account_id.clone();
    let proxy = tauri::async_runtime::spawn_blocking(move || account_proxy::status(&status_id))
        .await.map_err(|_| "读取代理运行状态失败")??;
    let logs = modules::codex_local_access::query_local_access_usage_events(
        1, 20, None, None, None, None, Some(account_id.clone()), None, None,
        None, None, None, None,
    ).await;
    let (logged_requests, recent_requests, logs_available) = match logs {
        Ok(page) => {
            let events = page.events.into_iter()
                .filter(|event| event.account_id == account_id)
                .take(5)
                .map(|event| {
                    let model = if event.requested_model.trim().is_empty() {
                        event.model_id.clone()
                    } else {
                        event.requested_model.clone()
                    };
                    ProxyRequestActivity {
                        timestamp: event.timestamp,
                        request_id: event.request_id,
                        model,
                        requested_model: event.requested_model,
                        upstream_model: event.upstream_model,
                        request_kind: serde_json::to_value(event.request_kind).ok()
                            .and_then(|value| value.as_str().map(str::to_string))
                            .unwrap_or_else(|| "other".into()),
                        gateway_mode: event.gateway_mode.map(|value| format!("{value:?}")),
                        service_tier: event.service_tier,
                        reasoning_effort: event.reasoning_effort,
                        success: event.success,
                        http_status: event.http_status,
                        latency_ms: event.latency_ms,
                        error_category: event.error_category,
                        error_message: event.error_message,
                        input_tokens: event.input_tokens,
                        output_tokens: event.output_tokens,
                        total_tokens: event.total_tokens,
                        estimated_cost_usd: event.estimated_cost_usd,
                        api_key_label: event.api_key_label,
                        client_instance_id: event.client_instance_id,
                    }
                }).collect();
            (page.total, events, true)
        }
        Err(_) => (0, Vec::new(), false),
    };
    Ok(ProxyAccountDashboard { proxy, logged_requests, recent_requests, logs_available })
}

fn require_account(id: &str) -> Result<(), String> {
    modules::codex_account::load_account(id)
        .map(|_| ())
        .ok_or_else(|| "Codex 账号不存在".into())
}

#[tauri::command]
pub async fn codex_get_account_proxy(account_id: String) -> Result<ProxyStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        require_account(&account_id)?;
        account_proxy::status(&account_id)
    })
    .await
    .map_err(|_| "读取代理任务失败")?
}

#[tauri::command]
pub async fn codex_save_account_proxy(
    account_id: String,
    proxy_uri: Option<String>,
) -> Result<ProxyStatus, String> {
    require_account(&account_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        // The sidecar and desktop client keep the same account-local loopback
        // address. save() preflights the replacement and swaps that listener;
        // old upstream connections are closed, never redirected to direct.
        account_proxy::save(&account_id, proxy_uri.as_deref())
    })
    .await
    .map_err(|_| "保存代理任务失败")?
}

#[tauri::command]
pub async fn codex_get_proxy_group_bindings(account_ids: Vec<String>) -> Result<Vec<account_proxy::ProxyGroupBinding>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        for id in &account_ids { require_account(id)?; }
        account_proxy::group_bindings(&account_ids)
    }).await.map_err(|_| "读取分组绑定任务失败")?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyTest {
    exit_ip: String,
    latency_ms: u128,
}

#[tauri::command]
pub async fn codex_test_account_proxy(account_id: String) -> Result<ProxyTest, String> {
    require_account(&account_id)?;
    let client_account_id = account_id.clone();
    let client = tauri::async_runtime::spawn_blocking(move || {
        if account_proxy::endpoint(&client_account_id)?.is_none() {
            return Err("请先保存代理绑定".to_string());
        }
        account_proxy::client(&client_account_id, Duration::from_secs(20))
    })
    .await
    .map_err(|_| "测试代理任务失败")??;
    let started = Instant::now();
    // Public IP probe only: no account bearer token/cookie is sent to the probe service.
    let response = client
        .get("https://api.ipify.org?format=json")
        .send()
        .await
        .map_err(|_| "代理连接失败或超时（未尝试直连），请检查节点是否可用")?;
    if !response.status().is_success() {
        return Err(format!(
            "出口检测服务返回 HTTP {}",
            response.status().as_u16()
        ));
    }
    let body: serde_json::Value = response.json().await.map_err(|_| "出口检测响应格式无效")?;
    let ip = body["ip"]
        .as_str()
        .and_then(|s| s.parse::<std::net::IpAddr>().ok())
        .ok_or("出口检测未返回有效 IP")?;
    account_proxy::record_probe(&account_id, ip.to_string(), started.elapsed().as_millis() as u64)?;
    Ok(ProxyTest {
        exit_ip: ip.to_string(),
        latency_ms: started.elapsed().as_millis(),
    })
}

#[tauri::command]
pub async fn codex_prepare_isolated_instance(account_id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        require_account(&account_id)?;
        account_proxy::ensure_isolated_instance(&account_id)
    })
    .await
    .map_err(|_| "创建隔离实例任务失败")?
}

/// Set the proxy before an account ID exists. `login_id` is used for OAuth;
/// omit it for token/JSON import and device-auth bootstrap.
#[tauri::command]
pub async fn codex_set_pending_auth_proxy(
    login_id: Option<String>,
    proxy_uri: Option<String>,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        account_proxy::save_pending(login_id.as_deref(), proxy_uri.as_deref())
    })
    .await
    .map_err(|_| "保存添加账号代理任务失败")?
}

#[tauri::command]
pub async fn codex_test_pending_auth_proxy(
    login_id: Option<String>,
    proxy_uri: String,
) -> Result<ProxyTest, String> {
    let login_for_start = login_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        account_proxy::save_pending(login_for_start.as_deref(), Some(&proxy_uri))
    })
    .await
    .map_err(|_| "启动添加账号代理测试失败")??;
    let started = Instant::now();
    let client = account_proxy::pending_client(login_id.as_deref(), Duration::from_secs(20))?;
    let response = client
        .get("https://api.ipify.org?format=json")
        .send()
        .await
        .map_err(|_| "添加账号代理出口检测失败（已阻止直连）")?;
    if !response.status().is_success() {
        return Err(format!(
            "出口检测服务返回 HTTP {}",
            response.status().as_u16()
        ));
    }
    let body: serde_json::Value = response.json().await.map_err(|_| "出口检测响应格式无效")?;
    let ip = body["ip"]
        .as_str()
        .and_then(|value| value.parse::<std::net::IpAddr>().ok())
        .ok_or("出口检测未返回有效 IP")?;
    Ok(ProxyTest {
        exit_ip: ip.to_string(),
        latency_ms: started.elapsed().as_millis(),
    })
}

#[tauri::command]
pub fn codex_get_proxy_inventory() -> Result<Vec<account_proxy::ProxyInventoryItem>, String> {
    account_proxy::inventory_list()
}

#[tauri::command]
pub async fn codex_save_proxy_inventory_entry(
    id: Option<String>,
    label: String,
    group_label: String,
    proxy_uri: String,
) -> Result<Vec<account_proxy::ProxyInventoryItem>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        account_proxy::inventory_save(id.as_deref(), &label, &group_label, &proxy_uri)
    }).await.map_err(|_| "保存代理库存任务失败")?
}

#[tauri::command]
pub async fn codex_delete_proxy_inventory_entry(
    id: String,
) -> Result<Vec<account_proxy::ProxyInventoryItem>, String> {
    tauri::async_runtime::spawn_blocking(move || account_proxy::inventory_delete(&id))
        .await.map_err(|_| "删除代理库存任务失败")?
}

#[tauri::command]
pub async fn codex_move_proxy_inventory_entry(
    id: String,
    group_label: String,
) -> Result<Vec<account_proxy::ProxyInventoryItem>, String> {
    tauri::async_runtime::spawn_blocking(move || account_proxy::inventory_move(&id, &group_label))
        .await.map_err(|_| "移动库存线路任务失败")?
}

#[tauri::command]
pub async fn codex_import_proxy_subscription(
    subscription_url: String,
    group_label: String,
    fetch_via_inventory_id: Option<String>,
    allow_direct: bool,
) -> Result<account_proxy::SubscriptionImportResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        account_proxy::import_subscription_url(
            &subscription_url,
            &group_label,
            fetch_via_inventory_id.as_deref(),
            allow_direct,
        )
    })
    .await
    .map_err(|_| "导入订阅任务异常")?
}
