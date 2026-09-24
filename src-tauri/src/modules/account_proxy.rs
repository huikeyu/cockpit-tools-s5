//! Per-account network isolation. No process-wide proxy/environment mutations.
use cockpit_account_proxy::{parse, parse_subscription, ProxyCore, ProxySpec, SubscriptionNode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    io::{Read, Write},
    net::{TcpListener, TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{atomic::{AtomicBool, Ordering}, LazyLock, Mutex},
    time::{Duration, Instant},
};

const MARKER: &str = ".cockpit-isolated-account";
include!("account_proxy_self_test.rs");
static RUNTIMES: LazyLock<Mutex<HashMap<String, Runtime>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static STORE_LOCK: Mutex<()> = Mutex::new(());
static INVENTORY_LOCK: Mutex<()> = Mutex::new(());
static PENDING_BINDINGS: LazyLock<Mutex<HashMap<String, PendingBinding>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static LOCKED_AUTH_PROXIES: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
const PENDING_IMPORT_KEY: &str = "__pending_import__";
static GROUP_MONITOR_STARTED: AtomicBool = AtomicBool::new(false);
static GROUP_MONITORED_IDS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Clone)]
struct RouteSelection {
    uris: Vec<String>,
    group_id: Option<String>,
    group_label: Option<String>,
}

#[derive(Clone)]
struct PendingBinding {
    selection: RouteSelection,
    port: u16,
}

#[derive(Clone, Serialize, Deserialize)]
struct ProxyInventoryEntry {
    id: String,
    label: String,
    group_id: String,
    group_label: String,
    uri: String,
    enabled: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyInventoryItem {
    pub id: String,
    pub label: String,
    pub group_id: String,
    pub group_label: String,
    pub enabled: bool,
    pub protocol: String,
    pub server_host: String,
    pub server_port: u16,
    pub insecure_tls: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionImportResult {
    pub imported: usize,
    pub updated: usize,
    pub skipped: usize,
    pub already_in_other_group: usize,
    pub insecure_nodes: usize,
    pub format: String,
    pub group_label: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Binding {
    uri: String,
    port: u16,
    #[serde(default)]
    fallback_uris: Vec<String>,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    group_label: Option<String>,
    #[serde(default)]
    failover_count: u64,
    #[serde(default)]
    last_exit_ip: Option<String>,
    #[serde(default)]
    last_probe_at: Option<i64>,
    #[serde(default)]
    last_probe_latency_ms: Option<u64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyStatus {
    pub enabled: bool,
    pub label: Option<String>,
    pub protocol: Option<String>,
    pub server_host: Option<String>,
    pub server_port: Option<u16>,
    pub security: Option<String>,
    pub transport: Option<String>,
    pub local_port: Option<u16>,
    pub running: bool,
    pub uptime_seconds: Option<u64>,
    pub last_exit_ip: Option<String>,
    pub last_probe_at: Option<i64>,
    pub last_probe_latency_ms: Option<u64>,
    pub required: bool,
    pub isolation_dir: String,
    pub group_id: Option<String>,
    pub group_label: Option<String>,
    pub route_count: usize,
    pub active_route_index: usize,
    pub failover_count: u64,
}

struct Runtime {
    child: Child,
    fingerprint: String,
    started_at: Instant,
    #[cfg(windows)]
    job: isize,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        #[cfg(windows)]
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(windows::Win32::Foundation::HANDLE(
                self.job as *mut _,
            ));
        }
    }
}

fn key(id: &str) -> String {
    format!("{:x}", Sha256::digest(id.as_bytes()))
}
fn binding_path(id: &str) -> Result<PathBuf, String> {
    Ok(super::account::get_data_dir()?
        .join("account-proxies")
        .join(format!("{}.json", key(id))))
}
fn required_marker_path(id: &str) -> Result<PathBuf, String> {
    Ok(super::account::get_data_dir()?
        .join("account-proxies")
        .join(format!("{}.required", key(id))))
}

fn inventory_path() -> Result<PathBuf, String> {
    Ok(super::account::get_data_dir()?.join("proxy-inventory.json"))
}

fn load_inventory() -> Result<Vec<ProxyInventoryEntry>, String> {
    let path = inventory_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    super::secure_account_storage::read_account_file_readonly::<Vec<ProxyInventoryEntry>>(
        &path,
        &super::account::get_data_dir()?.join("secure-account-storage.key"),
    )
    .map_err(|_| "代理库存无法解密或已损坏".into())
}

fn save_inventory(entries: &[ProxyInventoryEntry]) -> Result<(), String> {
    let path = inventory_path()?;
    let content = super::secure_account_storage::serialize_account_file(
        "codex-proxy-inventory",
        &entries.to_vec(),
    )?;
    std::fs::create_dir_all(path.parent().ok_or("代理库存目录无效")?)
        .map_err(|_| "创建代理库存目录失败")?;
    super::atomic_write::write_string_atomic(&path, &content)
        .map_err(|_| "保存代理库存失败".into())
}

fn refresh_inventory_bound_runtimes() {
    let ids = GROUP_MONITORED_IDS.lock().map(|value| value.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    for id in ids {
        if let Ok(mut runtimes) = RUNTIMES.lock() { runtimes.remove(&id); }
        let _ = endpoint(&id);
    }
}

fn inventory_item(entry: &ProxyInventoryEntry) -> Result<ProxyInventoryItem, String> {
    let spec = parse(&entry.uri)?;
    let parsed = url::Url::parse(&entry.uri).map_err(|_| "库存代理链接无效")?;
    Ok(ProxyInventoryItem {
        id: entry.id.clone(),
        label: entry.label.clone(),
        group_id: entry.group_id.clone(),
        group_label: entry.group_label.clone(),
        enabled: entry.enabled,
        protocol: spec.protocol,
        server_host: parsed.host_str().unwrap_or_default().to_string(),
        server_port: parsed.port_or_known_default().unwrap_or_default(),
        insecure_tls: parsed.query_pairs().any(|(key, value)| key == "insecure" && value == "1"),
    })
}

pub fn inventory_list() -> Result<Vec<ProxyInventoryItem>, String> {
    load_inventory()?.iter().map(inventory_item).collect()
}

pub fn inventory_save(
    id: Option<&str>,
    label: &str,
    group_label: &str,
    uri: &str,
) -> Result<Vec<ProxyInventoryItem>, String> {
    if label.chars().count() > 160 || group_label.chars().count() > 80
        || label.chars().any(char::is_control) || group_label.chars().any(char::is_control) {
        return Err("库存线路名称或分组名称过长或包含控制字符".into());
    }
    let _guard = INVENTORY_LOCK.lock().map_err(|_| "代理库存锁异常")?;
    let mut entries = load_inventory()?;
    let effective_uri = if uri.trim().is_empty() {
        id.and_then(|value| entries.iter().find(|item| item.id == value).map(|item| item.uri.clone()))
            .ok_or("新增库存线路必须填写代理链接")?
    } else {
        uri.to_string()
    };
    let spec = parse(&effective_uri)?;
    let entry_id = id.filter(|v| !v.trim().is_empty()).map(str::to_string)
        .unwrap_or_else(|| format!("proxy-{:x}", Sha256::digest(spec.normalized_uri.as_bytes())));
    let clean_group = if group_label.trim().is_empty() { "默认线路组" } else { group_label.trim() };
    let group_id = format!("group-{:x}", Sha256::digest(clean_group.to_ascii_lowercase().as_bytes()));
    let next = ProxyInventoryEntry {
        id: entry_id,
        label: if label.trim().is_empty() { spec.label.clone() } else { label.trim().to_string() },
        group_id,
        group_label: clean_group.to_string(),
        uri: spec.normalized_uri,
        enabled: true,
    };
    if entries.iter().any(|item| item.id != next.id && item.uri == next.uri) {
        return Err("该代理链接已存在于库存，不能重复作为故障接管线路".into());
    }
    if let Some(existing) = entries.iter_mut().find(|item| item.id == next.id) {
        *existing = next;
    } else {
        entries.push(next);
    }
    save_inventory(&entries)?;
    refresh_inventory_bound_runtimes();
    inventory_list()
}

pub fn inventory_delete(id: &str) -> Result<Vec<ProxyInventoryItem>, String> {
    let _guard = INVENTORY_LOCK.lock().map_err(|_| "代理库存锁异常")?;
    let mut entries = load_inventory()?;
    entries.retain(|item| item.id != id);
    save_inventory(&entries)?;
    refresh_inventory_bound_runtimes();
    inventory_list()
}

fn import_inventory_nodes(group_label: &str, nodes: Vec<SubscriptionNode>) -> Result<(usize, usize, usize), String> {
    if group_label.trim().is_empty() || group_label.chars().count() > 80
        || group_label.chars().any(char::is_control) {
        return Err("请填写不超过 80 字的订阅分组名称".into());
    }
    let _guard = INVENTORY_LOCK.lock().map_err(|_| "代理库存锁异常")?;
    let mut entries = load_inventory()?;
    let group_id = format!("group-{:x}", Sha256::digest(group_label.trim().to_ascii_lowercase().as_bytes()));
    let mut imported = 0;
    let mut updated = 0;
    let mut already_in_other_group = 0;
    for node in nodes {
        let id = format!("proxy-{:x}", Sha256::digest(node.normalized_uri.as_bytes()));
        if let Some(entry) = entries.iter_mut().find(|item| item.id == id) {
            if entry.group_id == group_id {
                entry.label = node.label;
                entry.enabled = true;
                updated += 1;
            } else {
                already_in_other_group += 1;
            }
        } else if entries.iter().any(|item| item.uri == node.normalized_uri) {
            already_in_other_group += 1;
        } else {
            entries.push(ProxyInventoryEntry {
                id, label: node.label, group_id: group_id.clone(),
                group_label: group_label.trim().to_string(),
                uri: node.normalized_uri, enabled: true,
            });
            imported += 1;
        }
    }
    if imported + updated > 0 {
        save_inventory(&entries)?;
        refresh_inventory_bound_runtimes();
    }
    Ok((imported, updated, already_in_other_group))
}

pub fn import_subscription_url(
    raw_url: &str,
    group_label: &str,
    fetch_via_inventory_id: Option<&str>,
    allow_direct: bool,
) -> Result<SubscriptionImportResult, String> {
    if group_label.trim().is_empty() || group_label.chars().count() > 80
        || group_label.chars().any(char::is_control) {
        return Err("请填写不超过 80 字的订阅分组名称".into());
    }
    let url = url::Url::parse(raw_url.trim()).map_err(|_| "订阅地址格式无效")?;
    if !safe_subscription_url(&url) {
        return Err("订阅地址必须是公开 HTTPS 地址，不能包含用户信息或指向本机/私有网络".into());
    }
    let direct_fetch = fetch_via_inventory_id.is_none_or(|value| value.trim().is_empty());
    if direct_fetch {
        if !allow_direct { return Err("请选一条库存代理拉取订阅，或明确允许本机网络直连订阅服务".into()); }
        resolved_public_subscription_addrs(url.host_str().unwrap_or_default())?;
    }
    let mut lease = None;
    let permitted_hosts = if direct_fetch {
        collect_direct_subscription_addresses(&[url.clone()])?
    } else { HashMap::new() };
    let mut builder = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() < 3 && safe_subscription_url(attempt.url())
                && (!direct_fetch || permitted_hosts.contains_key(attempt.url().host_str().unwrap_or_default())) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(35));
    if let Some(id) = fetch_via_inventory_id.filter(|value| !value.trim().is_empty()) {
        let item = load_inventory()?.into_iter().find(|entry| entry.id == id && entry.enabled)
            .ok_or("用于拉取订阅的库存线路不存在或已停用")?;
        let spec = parse(&item.uri)?;
        let port = allocate_port()?;
        let runtime = spawn_core(&spec, port)?;
        builder = builder.proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))
            .map_err(|_| "订阅拉取代理配置无效")?);
        lease = Some(runtime);
    }
    if direct_fetch {
        let target_host = url.host_str().unwrap_or_default().to_string();
        for address in resolved_public_subscription_addrs(&target_host)? {
            builder = builder.resolve(&target_host, address);
        }
    }
    let client = builder.build().map_err(|_| "创建订阅拉取客户端失败")?;
    let response = client.get(url).header("User-Agent", "CockpitTools-Isolation/4")
        .send().map_err(|_| "订阅拉取失败；未改动代理库存")?;
    if !response.status().is_success() {
        return Err(format!("订阅服务返回 HTTP {}；未改动代理库存", response.status().as_u16()));
    }
    let mut body = Vec::new();
    response.take(2 * 1024 * 1024 + 1).read_to_end(&mut body)
        .map_err(|_| "读取订阅内容失败；未改动代理库存")?;
    drop(lease);
    if body.len() > 2 * 1024 * 1024 { return Err("订阅内容超过 2 MiB 上限".into()); }
    let text = String::from_utf8(body).map_err(|_| "订阅不是 UTF-8 文本")?;
    let parsed = parse_subscription(&text)?;
    let insecure_nodes = parsed.nodes.iter().filter(|node| {
        url::Url::parse(&node.normalized_uri).ok().is_some_and(|url| {
            url.query_pairs().any(|(key, value)| key == "insecure" && value == "1")
        })
    }).count();
    let (imported, updated, already_in_other_group) = import_inventory_nodes(group_label, parsed.nodes)?;
    Ok(SubscriptionImportResult {
        imported, updated, skipped: parsed.skipped, already_in_other_group, insecure_nodes,
        format: parsed.format.to_string(), group_label: group_label.trim().to_string(),
    })
}

fn safe_subscription_url(url: &url::Url) -> bool {
    if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty()
        || url.password().is_some() || url.fragment().is_some() || url.port().is_some_and(|port| port != 443) {
        return false;
    }
    let host = url.host_str().unwrap_or_default();
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".local")
        || host.ends_with(".localhost") || host.ends_with(".internal") || !host.contains('.') {
        return false;
    }
    // Do not allow IP-literal subscription hosts, including public literals:
    // DNS names can still be validated against private-address rebinding.
    host.parse::<std::net::IpAddr>().is_err()
}

#[cfg(test)]
mod subscription_url_tests {
    use super::*;
    #[test]
    fn only_public_https_subscription_hosts_are_accepted() {
        for value in [
            "http://example.com/sub", "https://localhost/sub", "https://127.0.0.1/sub",
            "https://192.168.1.5/sub", "https://example.local/sub", "https://example.com:8443/sub",
            "https://user:secret@example.com/sub", "https://example.com/sub#fragment",
        ] {
            assert!(!safe_subscription_url(&url::Url::parse(value).unwrap()), "{value}");
        }
        assert!(safe_subscription_url(&url::Url::parse("https://example.com/sub?token=synthetic").unwrap()));
    }
}

fn resolved_public_subscription_addrs(host: &str) -> Result<Vec<std::net::SocketAddr>, String> {
    let addresses = (host, 443u16).to_socket_addrs()
        .map_err(|_| "订阅域名解析失败；未发送请求")?
        .collect::<Vec<_>>();
    if addresses.is_empty() || addresses.iter().any(|address| !public_subscription_address(address.ip())) {
        return Err("订阅域名解析到了私有或本地地址；已阻止请求".into());
    }
    Ok(addresses)
}

fn collect_direct_subscription_addresses(
    urls: &[url::Url],
) -> Result<HashMap<String, Vec<std::net::SocketAddr>>, String> {
    let mut result = HashMap::new();
    for url in urls {
        if !safe_subscription_url(url) { return Err("订阅重定向地址不安全".into()); }
        let host = url.host_str().unwrap_or_default().to_string();
        if !result.contains_key(&host) {
            result.insert(host.clone(), resolved_public_subscription_addrs(&host)?);
        }
    }
    Ok(result)
}

fn public_subscription_address(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => !(ip.is_private() || ip.is_loopback() || ip.is_link_local()
            || ip.is_unspecified() || ip.is_broadcast() || ip.is_documentation()
            || ip.is_multicast() || ip.octets()[0] >= 240
            || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
            || (ip.octets()[0] == 198 && (ip.octets()[1] == 18 || ip.octets()[1] == 19))),
        std::net::IpAddr::V6(ip) => !(ip.is_loopback() || ip.is_unique_local()
            || ip.is_unicast_link_local() || ip.is_unspecified() || ip.is_multicast()),
    }
}

fn same_routes(a: &[String], b: &[String]) -> bool {
    let mut left = a.to_vec();
    let mut right = b.to_vec();
    left.sort();
    right.sort();
    left == right
}

fn resolve_selection(input: &str) -> Result<RouteSelection, String> {
    let trimmed = input.trim();
    if let Some(id) = trimmed.strip_prefix("inventory://") {
        let entries = load_inventory()?;
        let selected = entries.iter().find(|item| item.id == id && item.enabled)
            .ok_or("代理库存线路不存在或已停用")?;
        let mut uris = entries.iter()
            .filter(|item| item.enabled && item.group_id == selected.group_id)
            .map(|item| item.uri.clone())
            .collect::<Vec<_>>();
        if uris.is_empty() { return Err("代理线路组没有可用线路".into()); }
        uris.retain(|uri| uri != &selected.uri);
        uris.insert(0, selected.uri.clone());
        return Ok(RouteSelection {
            uris,
            group_id: Some(selected.group_id.clone()),
            group_label: Some(selected.group_label.clone()),
        });
    }
    let spec = parse(trimmed)?;
    Ok(RouteSelection { uris: vec![spec.normalized_uri], group_id: None, group_label: None })
}

fn binding_uris(binding: &Binding) -> Vec<String> {
    let mut uris = vec![binding.uri.clone()];
    uris.extend(binding.fallback_uris.iter().cloned());
    uris
}

pub fn require_proxy(id: &str) -> Result<(), String> {
    let path = required_marker_path(id)?;
    std::fs::create_dir_all(path.parent().ok_or("账号代理目录无效")?)
        .map_err(|_| "创建账号代理目录失败")?;
    super::atomic_write::write_string_atomic(&path, "proxy-required")
        .map_err(|_| "标记账号代理为必需时失败".to_string())
}
pub fn isolation_dir(id: &str) -> Result<PathBuf, String> {
    Ok(super::codex_instance::get_default_instances_root_dir()?
        .join(format!("isolated-{}", &key(id)[..24])))
}
pub fn is_isolated_profile(path: &Path) -> bool {
    path.join(MARKER).is_file()
}

pub fn validate_isolated_binding(profile: &Path, binding: Option<&str>) -> Result<(), String> {
    if !is_isolated_profile(profile) {
        return Ok(());
    }
    let expected =
        std::fs::read_to_string(profile.join(MARKER)).map_err(|_| "读取隔离账号标记失败")?;
    if binding != Some(expected.trim()) {
        return Err(
            "隔离实例不能改绑其他账号或账号池；请从目标账号的独立代理入口创建新实例".into(),
        );
    }
    Ok(())
}

fn load_binding(id: &str) -> Result<Option<Binding>, String> {
    let path = binding_path(id)?;
    match std::fs::metadata(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("无法读取账号代理配置；为防止直连已阻止请求".into()),
    }
    let mut binding = super::secure_account_storage::read_account_file_readonly::<Binding>(
        &path,
        &super::account::get_data_dir()?.join("secure-account-storage.key"),
    )
    .map_err(|_| "账号代理配置无法解密或已损坏；为防止直连已阻止请求".to_string())?;
    if let Some(group_id) = binding.group_id.as_deref() {
        let members = load_inventory()?.into_iter()
            .filter(|item| item.enabled && item.group_id == group_id)
            .collect::<Vec<_>>();
        if members.is_empty() {
            return Err("账号绑定的代理线路组已无可用线路；已阻止直连".into());
        }
        let active = members.iter().find(|item| item.uri == binding.uri).unwrap_or(&members[0]);
        if active.uri != binding.uri {
            binding.last_exit_ip = None;
            binding.last_probe_at = None;
            binding.last_probe_latency_ms = None;
        }
        binding.uri = active.uri.clone();
        binding.group_label = Some(active.group_label.clone());
        binding.fallback_uris = members.iter().filter(|item| item.uri != active.uri).map(|item| item.uri.clone()).collect();
    }
    Ok(Some(binding))
}

fn allocate_port() -> Result<u16, String> {
    let data = super::account::get_data_dir()?;
    let directory = data.join("account-proxies");
    let mut reserved = std::collections::HashSet::new();
    if directory.exists() {
        for entry in std::fs::read_dir(&directory).map_err(|_| "无法检查已分配代理端口")?
        {
            let path = entry.map_err(|_| "读取代理绑定目录失败")?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let binding: Binding = super::secure_account_storage::read_account_file_readonly(
                &path,
                &data.join("secure-account-storage.key"),
            )
            .map_err(|_| "已有代理绑定损坏，无法安全分配新端口")?;
            reserved.insert(binding.port);
        }
    }
    let mut rejected = Vec::new();
    for _ in 0..64 {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| "无法分配账号代理端口")?;
        let port = listener
            .local_addr()
            .map_err(|_| "无法读取账号代理端口")?
            .port();
        if !reserved.contains(&port) {
            return Ok(port);
        }
        rejected.push(listener);
    }
    Err("无法分配未被其他账号保留的代理端口".into())
}

pub fn status(id: &str) -> Result<ProxyStatus, String> {
    let binding = load_binding(id)?;
    let spec = binding.as_ref().map(|b| parse(&b.uri)).transpose()?;
    let inventory_label = if let Some(bound) = binding.as_ref().filter(|item| item.group_id.is_some()) {
        load_inventory()?.into_iter()
            .find(|item| Some(&item.group_id) == bound.group_id.as_ref() && item.uri == bound.uri)
            .map(|item| item.label)
    } else { None };
    let active_route_index = if let Some(binding) = binding.as_ref().filter(|value| value.group_id.is_some()) {
        load_inventory()?.iter()
            .filter(|item| item.enabled && Some(&item.group_id) == binding.group_id.as_ref())
            .position(|item| item.uri == binding.uri)
            .unwrap_or(0)
    } else { 0 };
    let parsed_uri = binding.as_ref().and_then(|b| url::Url::parse(&b.uri).ok());
    let query = parsed_uri.as_ref().map(|url| url.query_pairs().collect::<HashMap<_, _>>());
    let (running, uptime_seconds) = RUNTIMES
        .lock()
        .map_err(|_| "代理运行状态锁异常")?
        .get_mut(id)
        .map(|runtime| {
            let running = spec.as_ref().is_some_and(|spec| runtime.fingerprint == key(&spec.normalized_uri))
                && matches!(runtime.child.try_wait(), Ok(None));
            (running, running.then(|| runtime.started_at.elapsed().as_secs()))
        })
        .unwrap_or((false, None));
    Ok(ProxyStatus {
        enabled: binding.is_some(),
        label: inventory_label.or_else(|| spec.as_ref().map(|s| s.label.clone())),
        protocol: spec.as_ref().map(|s| s.protocol.clone()),
        server_host: parsed_uri.as_ref().and_then(|url| url.host_str().map(str::to_string)),
        server_port: parsed_uri.as_ref().and_then(url::Url::port_or_known_default),
        security: if spec.as_ref().is_some_and(|item| item.core() == ProxyCore::SingBox) {
            Some(if query.as_ref().is_some_and(|values| values.get("insecure").is_some_and(|value| value == "1")) {
                "TLS insecure".into()
            } else { "TLS".into() })
        } else { query.as_ref().and_then(|values| values.get("security").map(|v| v.to_string())) },
        transport: query.as_ref().and_then(|values| values.get("type").map(|v| v.to_string())),
        local_port: binding.as_ref().map(|b| b.port),
        running,
        uptime_seconds,
        last_exit_ip: binding.as_ref().and_then(|b| b.last_exit_ip.clone()),
        last_probe_at: binding.as_ref().and_then(|b| b.last_probe_at),
        last_probe_latency_ms: binding.as_ref().and_then(|b| b.last_probe_latency_ms),
        required: required_marker_path(id)?.exists(),
        isolation_dir: isolation_dir(id)?.to_string_lossy().into_owned(),
        group_id: binding.as_ref().and_then(|b| b.group_id.clone()),
        group_label: binding.as_ref().and_then(|b| b.group_label.clone()),
        route_count: binding.as_ref().map(|b| binding_uris(b).len()).unwrap_or(0),
        active_route_index,
        failover_count: binding.as_ref().map(|b| b.failover_count).unwrap_or(0),
    })
}

fn core_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|_| "无法定位代理内核")?;
    let mut candidates = vec![exe
        .parent()
        .ok_or("无法定位程序目录")?
        .join("proxy-core/xray.exe")];
    #[cfg(debug_assertions)]
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proxy-core/xray.exe"));
    if let Some(app) = crate::get_app_handle() {
        use tauri::Manager;
        if let Ok(path) = app.path().resource_dir() {
            candidates.push(path.join("proxy-core/xray.exe"));
        }
    }
    candidates.into_iter().find(|p| p.is_file()).ok_or_else(|| {
        "缺少内置代理内核 proxy-core/xray.exe，请使用完整编译成品目录或安装包".into()
    })
}

fn sing_box_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|_| "无法定位 sing-box 内核")?;
    let mut candidates = vec![exe.parent().ok_or("无法定位程序目录")?
        .join("proxy-core/sing-box.exe")];
    #[cfg(debug_assertions)]
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proxy-core/sing-box.exe"));
    if let Some(app) = crate::get_app_handle() {
        use tauri::Manager;
        if let Ok(path) = app.path().resource_dir() {
            candidates.push(path.join("proxy-core/sing-box.exe"));
        }
    }
    candidates.into_iter().find(|path| path.is_file())
        .ok_or_else(|| "缺少内置 sing-box 内核，请使用完整 V4 安装包".into())
}

#[cfg(windows)]
fn attach_job(child: &Child) -> Result<isize, String> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::*,
    };
    unsafe {
        let job = CreateJobObjectW(None, None).map_err(|_| "创建代理进程保护失败")?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let result = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of_val(&info) as u32,
        )
        .and_then(|_| AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())));
        if result.is_err() {
            let _ = CloseHandle(job);
            return Err("绑定代理进程保护失败".into());
        }
        Ok(job.0 as isize)
    }
}

fn spawn_core(spec: &ProxySpec, port: u16) -> Result<Runtime, String> {
    let probe = TcpListener::bind(("127.0.0.1", port))
        .map_err(|_| "账号代理本地端口被占用；请关闭占用程序后重试（不会切换为直连）")?;
    let (binary, args, config) = match spec.core() {
        ProxyCore::Xray => (core_path()?, vec!["run", "-format", "json", "-config", "stdin:"], spec.xray_config(port)),
        ProxyCore::SingBox => (sing_box_path()?, vec!["run", "-c", "stdin"], spec.sing_box_config(port)),
    };
    let mut command = Command::new(binary);
    // Xray accepts stdin: as its config source. Secrets never appear in argv or temp files.
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    drop(probe);
    let mut child = command.spawn().map_err(|_| "无法启动内置代理内核")?;
    #[cfg(windows)]
    let job = match attach_job(&child) {
        Ok(job) => job,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    let mut runtime = Runtime {
        child,
        fingerprint: key(&spec.normalized_uri),
        started_at: Instant::now(),
        #[cfg(windows)]
        job,
    };
    let config = config.to_string();
    runtime
        .child
        .stdin
        .take()
        .ok_or("代理配置管道不可用")?
        .write_all(config.as_bytes())
        .map_err(|_| "写入代理配置失败")?;
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if runtime
            .child
            .try_wait()
            .map_err(|_| "检查代理进程失败")?
            .is_some()
        {
            return Err("代理内核拒绝节点配置；请检查节点协议、指纹及 Reality 参数".into());
        }
        if TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().unwrap(),
            Duration::from_millis(80),
        )
        .is_ok()
        {
            return Ok(runtime);
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    Err("代理内核启动超时；已阻止直连".into())
}

fn probe_proxy_port(port: u16) -> Result<(String, u64), String> {
    // Some callers are inside Tokio. Keep reqwest's blocking runtime on its
    // own OS thread to prevent nested-runtime panics.
    std::thread::spawn(move || probe_proxy_port_blocking(port))
        .join()
        .map_err(|_| "代理健康检测线程异常；已阻止直连".to_string())?
}

fn probe_proxy_port_blocking(port: u16) -> Result<(String, u64), String> {
    let url = std::env::var("COCKPIT_PROXY_SELFTEST_PROBE_URL")
        .unwrap_or_else(|_| "https://api.ipify.org?format=json".into());
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))
            .map_err(|_| "代理健康检测配置无效")?)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|_| "创建代理健康检测客户端失败")?;
    let started = Instant::now();
    let response = client.get(url).send()
        .map_err(|_| "代理健康检测失败；未使用直连")?;
    if !response.status().is_success() {
        return Err("代理健康检测返回失败状态".into());
    }
    let body = response.text().map_err(|_| "读取代理健康检测响应失败")?;
    let exit_ip = serde_json::from_str::<serde_json::Value>(&body).ok()
        .and_then(|value| value["ip"].as_str().map(str::to_string))
        .unwrap_or_default();
    Ok((exit_ip, started.elapsed().as_millis() as u64))
}

fn begin_group_monitor(id: &str) {
    if let Ok(mut ids) = GROUP_MONITORED_IDS.lock() {
        ids.insert(id.to_string());
    }
    if GROUP_MONITOR_STARTED.swap(true, Ordering::SeqCst) { return; }
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(20));
        let ids = GROUP_MONITORED_IDS.lock().map(|value| value.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for id in ids {
            let _ = monitor_group_route_once(&id);
        }
    });
}

fn monitor_group_route_once(id: &str) -> Result<(), String> {
    let binding = match load_binding(id) {
        Ok(Some(value)) => value,
        Ok(None) => return Ok(()),
        Err(error) => {
            if let Ok(mut runtimes) = RUNTIMES.lock() { runtimes.remove(id); }
            return Err(error);
        }
    };
    if binding.group_id.is_none() { return Ok(()); }
    let is_running = RUNTIMES.lock().map_err(|_| "代理运行状态锁异常")?
        .get_mut(id).is_some_and(|runtime| runtime.fingerprint == key(&binding.uri) && matches!(runtime.child.try_wait(), Ok(None)));
    if is_running && probe_proxy_port(binding.port).is_ok() { return Ok(()); }
    RUNTIMES.lock().map_err(|_| "代理运行状态锁异常")?.remove(id);
    endpoint(id)?.ok_or("代理线路组没有可用线路；已阻止直连")?;
    Ok(())
}

/// Resolve only this account. Failure is propagated, never swallowed or replaced by global proxy.
pub fn endpoint(id: &str) -> Result<Option<String>, String> {
    let _guard = STORE_LOCK.lock().map_err(|_| "代理配置锁异常")?;
    let Some(binding) = load_binding(id)? else {
        if required_marker_path(id)?.exists() {
            return Err("此账号必须绑定独立代理；当前配置不可用，已阻止直连".into());
        }
        return Ok(None);
    };
    if binding.port == 0 {
        return Err("账号代理端口无效；已阻止直连".into());
    }
    let mut runtimes = RUNTIMES.lock().map_err(|_| "代理运行状态锁异常")?;
    let healthy = runtimes.get_mut(id).is_some_and(|r| {
        r.fingerprint == key(&binding.uri) && matches!(r.child.try_wait(), Ok(None))
    });
    if !healthy {
        runtimes.remove(id);
        let mut last_error = "代理线路组没有可用线路".to_string();
        let routes = binding_uris(&binding);
        for (index, uri) in routes.iter().enumerate() {
            let spec = parse(uri)?;
            match spawn_core(&spec, binding.port) {
                Ok(runtime) => {
                    if binding.group_id.is_some() && probe_proxy_port(binding.port).is_err() {
                        drop(runtime);
                        last_error = "代理线路健康检测失败；已尝试同组下一条线路".into();
                        continue;
                    }
                    if index > 0 {
                        let mut rotated = binding.clone();
                        rotated.uri = uri.clone();
                        rotated.fallback_uris = routes.iter().filter(|candidate| *candidate != uri).cloned().collect();
                        rotated.failover_count = binding.failover_count.saturating_add(1);
                        rotated.last_exit_ip = None;
                        rotated.last_probe_at = None;
                        rotated.last_probe_latency_ms = None;
                        let path = binding_path(id)?;
                        let encrypted = super::secure_account_storage::serialize_account_file("codex-account-proxy", &rotated)?;
                        super::atomic_write::write_string_atomic(&path, &encrypted).map_err(|_| "保存故障接管线路失败".to_string())?;
                    }
                    runtimes.insert(id.to_string(), runtime);
                    if binding.group_id.is_some() { begin_group_monitor(id); }
                    return Ok(Some(format!("http://127.0.0.1:{}", binding.port)));
                }
                Err(error) => last_error = error,
            }
        }
        return Err(last_error);
    }
    if binding.group_id.is_some() { begin_group_monitor(id); }
    Ok(Some(format!("http://127.0.0.1:{}", binding.port)))
}

pub fn effective_proxy(id: &str, _fallback: Option<&str>) -> Result<Option<String>, String> {
    endpoint(id)?.map(Some).ok_or_else(|| "API 服务账号缺少独立代理；已阻止全局代理或直连回退".into())
}

/// Save a proxy before an account ID exists. This state is memory-only and is
/// used by OAuth browser/token exchange; it is never treated as an account binding.
pub fn save_pending(login_id: Option<&str>, input: Option<&str>) -> Result<Option<String>, String> {
    let key = login_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(PENDING_IMPORT_KEY)
        .to_string();
    let runtime_key = format!("__pending__:{key}");
    if let Some(value) = input.filter(|value| !value.trim().is_empty()) {
        let selection = resolve_selection(value)?;
        let selection_key = selection.uris.join("|");
        if LOCKED_AUTH_PROXIES
            .lock()
            .map_err(|_| "认证代理锁状态异常")?
            .contains(&key)
        {
            let current = PENDING_BINDINGS
                .lock()
                .map_err(|_| "待认证代理状态锁异常")?
                .get(&key)
                .map(|pending| pending.selection.uris.join("|"));
            if current.as_deref() != Some(selection_key.as_str())
                && !PENDING_BINDINGS.lock().map_err(|_| "待认证代理状态锁异常")?
                    .get(&key).is_some_and(|pending| same_routes(&pending.selection.uris, &selection.uris))
            {
                return Err("认证已开始，不能更换此流程的代理；请取消后重新登录".into());
            }
        }
        if let Some(old) = PENDING_BINDINGS
            .lock()
            .map_err(|_| "待认证代理状态锁异常")?
            .get(&key)
            .cloned()
        {
            if same_routes(&old.selection.uris, &selection.uris) {
                let healthy = RUNTIMES
                    .lock()
                    .map_err(|_| "代理运行状态锁异常")?
                    .get_mut(&runtime_key)
                    .is_some_and(|runtime| {
                        runtime.fingerprint == crate::modules::account_proxy::key(&old.selection.uris[0])
                            && matches!(runtime.child.try_wait(), Ok(None))
                    });
                if healthy {
                    return Ok(Some(format!("http://127.0.0.1:{}", old.port)));
                }
            }
        }
        let port = allocate_port()?;
        let mut runtime = None;
        let mut active_uri = None;
        for uri in &selection.uris {
            let spec = parse(uri)?;
            match spawn_core(&spec, port) {
                Ok(value) => {
                    if selection.group_id.is_some() && probe_proxy_port(port).is_err() {
                        drop(value);
                        continue;
                    }
                    runtime = Some(value);
                    active_uri = Some(spec.normalized_uri);
                    break;
                }
                Err(_) => {}
            }
        }
        let runtime = runtime.ok_or("代理线路组没有可用线路；已阻止认证继续")?;
        let active_uri = active_uri.ok_or("代理线路组没有可用线路")?;
        let mut ordered = selection.clone();
        ordered.uris.retain(|uri| uri != &active_uri);
        ordered.uris.insert(0, active_uri);
        RUNTIMES
            .lock()
            .map_err(|_| "代理运行状态锁异常")?
            .insert(runtime_key, runtime);
        PENDING_BINDINGS
            .lock()
            .map_err(|_| "待认证代理状态锁异常")?
            .insert(key.clone(), PendingBinding { selection: ordered, port });
        Ok(Some(format!("http://127.0.0.1:{port}")))
    } else {
        clear_pending(login_id);
        Ok(None)
    }
}

pub fn pending_uri(login_id: Option<&str>) -> Option<String> {
    let key = login_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(PENDING_IMPORT_KEY);
    PENDING_BINDINGS
        .lock()
        .ok()?
        .get(key)
        .and_then(|pending| pending.selection.uris.first().cloned())
}

pub fn pending_endpoint(login_id: Option<&str>) -> Result<String, String> {
    let pending_key = login_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(PENDING_IMPORT_KEY);
    let pending = PENDING_BINDINGS
        .lock()
        .map_err(|_| "待认证代理状态锁异常")?
        .get(pending_key)
        .cloned()
        .ok_or_else(|| {
            "添加账号前必须先设置独立代理；为防止原始 IP 泄露，已阻止认证继续".to_string()
        })?;
    let uri = pending.selection.uris.first().cloned().ok_or("待认证代理线路为空")?;
    let port = pending.port;
    let runtime_key = format!("__pending__:{pending_key}");
    let healthy = RUNTIMES
        .lock()
        .map_err(|_| "代理运行状态锁异常")?
        .get_mut(&runtime_key)
        .is_some_and(|runtime| {
            runtime.fingerprint == crate::modules::account_proxy::key(&uri)
                && matches!(runtime.child.try_wait(), Ok(None))
        });
    if healthy {
        return Ok(format!("http://127.0.0.1:{port}"));
    }
    if LOCKED_AUTH_PROXIES.lock().map_err(|_| "认证代理锁状态异常")?.contains(pending_key) {
        return Err("认证期间绑定的代理已失效；为保持登录过程同一网络，请取消并重新登录".into());
    }
    if let Ok(mut runtimes) = RUNTIMES.lock() { runtimes.remove(&runtime_key); }
    let selection = pending.selection;
    let mut last_error = "添加账号线路组全部故障；已阻止直连".to_string();
    for candidate in &selection.uris {
        let spec = parse(candidate)?;
        match spawn_core(&spec, port) {
            Ok(runtime) => {
                if selection.group_id.is_some() && probe_proxy_port(port).is_err() {
                    drop(runtime);
                    continue;
                }
                RUNTIMES.lock().map_err(|_| "代理运行状态锁异常")?.insert(runtime_key.clone(), runtime);
                let mut ordered = selection.clone();
                ordered.uris.retain(|value| value != candidate);
                ordered.uris.insert(0, candidate.clone());
                PENDING_BINDINGS.lock().map_err(|_| "待认证代理状态锁异常")?
                    .insert(pending_key.to_string(), PendingBinding { selection: ordered, port });
                return Ok(format!("http://127.0.0.1:{port}"));
            }
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

pub fn pending_client(
    login_id: Option<&str>,
    timeout: Duration,
) -> Result<reqwest::Client, String> {
    let proxy = pending_endpoint(login_id)?;
    reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(proxy).map_err(|_| "添加账号代理配置无效")?)
        .connect_timeout(timeout.min(Duration::from_secs(15)))
        .timeout(timeout)
        .build()
        .map_err(|_| "创建添加账号网络客户端失败".into())
}

pub fn clear_pending(login_id: Option<&str>) {
    let key = login_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(PENDING_IMPORT_KEY)
        .to_string();
    if let Ok(mut pending) = PENDING_BINDINGS.lock() {
        pending.remove(&key);
    }
    if let Ok(mut runtimes) = RUNTIMES.lock() {
        runtimes.remove(&format!("__pending__:{key}"));
    }
    if let Ok(mut locked) = LOCKED_AUTH_PROXIES.lock() {
        locked.remove(&key);
    }
}

pub fn lock_pending(login_id: &str) -> Result<(), String> {
    pending_endpoint(Some(login_id))?;
    LOCKED_AUTH_PROXIES
        .lock()
        .map_err(|_| "认证代理锁状态异常")?
        .insert(login_id.to_string());
    Ok(())
}

pub fn promote_pending(login_id: Option<&str>, account_id: &str) -> Result<(), String> {
    let key = login_id.filter(|value| !value.trim().is_empty()).unwrap_or(PENDING_IMPORT_KEY);
    let selection = PENDING_BINDINGS.lock().ok().and_then(|pending| pending.get(key).map(|value| value.selection.clone()))
        .or_else(|| pending_uri(login_id).map(|uri| RouteSelection { uris: vec![uri], group_id: None, group_label: None }))
        .ok_or_else(|| "添加账号代理不存在；已阻止首次账号请求".to_string())?;
    if let Some(existing) = load_binding(account_id)? {
        if existing.uri != selection.uris.first().cloned().unwrap_or_default() {
            return Err("现有账号已绑定另一代理；请先停止 API 服务并在账号卡片中修改绑定".into());
        }
        clear_pending(login_id);
        return Ok(());
    }
    require_proxy(account_id)?;
    save_selection(account_id, selection)?;
    clear_pending(login_id);
    Ok(())
}

pub fn clone_pending_for_login(login_id: &str) -> Result<(), String> {
    let selection = PENDING_BINDINGS.lock().map_err(|_| "待认证代理状态锁异常")?
        .get(PENDING_IMPORT_KEY).map(|value| value.selection.clone())
        .ok_or("添加账号前必须设置独立代理")?;
    let active_uri = selection.uris.first().ok_or("待认证代理线路为空")?.clone();
    save_pending(Some(login_id), Some(&active_uri))?;
    let port = PENDING_BINDINGS.lock().map_err(|_| "待认证代理状态锁异常")?
        .get(login_id).map(|pending| pending.port).ok_or("复制待认证代理失败")?;
    if selection.group_id.is_some() && probe_proxy_port(port).is_err() {
        clear_pending(Some(login_id));
        return Err("原登录线路已失效；为保持整个登录过程同一网络，请重新开始".into());
    }
    PENDING_BINDINGS.lock().map_err(|_| "待认证代理状态锁异常")?
        .get_mut(login_id).ok_or("复制待认证代理失败")?.selection = selection;
    Ok(())
}

pub fn client(id: &str, timeout: Duration) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(timeout.min(Duration::from_secs(15)))
        .timeout(timeout);
    let proxy = endpoint(id)?.ok_or("Codex 账号缺少独立代理；已阻止直接网络请求")?;
    builder = builder
        .no_proxy()
        .proxy(reqwest::Proxy::all(proxy).map_err(|_| "账号代理配置无效")?);
    builder.build().map_err(|_| "创建账号网络客户端失败".into())
}

pub fn save(id: &str, input: Option<&str>) -> Result<ProxyStatus, String> {
    let Some(value) = input.filter(|s| !s.trim().is_empty()) else {
        let _guard = STORE_LOCK.lock().map_err(|_| "代理配置锁异常")?;
        if required_marker_path(id)?.exists() {
            return Err("此账号要求独立代理；请改绑节点或删除账号，不能解除后直连".into());
        }
        let path = binding_path(id)?;
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("删除代理绑定失败".into()),
        }
        RUNTIMES.lock().map_err(|_| "代理运行状态锁异常")?.remove(id);
        return status(id);
    };
    save_selection(id, resolve_selection(value)?)
}

fn save_selection(id: &str, selection: RouteSelection) -> Result<ProxyStatus, String> {
    if selection.uris.is_empty() {
        return Err("代理线路组为空".into());
    }
    {
        let _guard = STORE_LOCK.lock().map_err(|_| "代理配置锁异常")?;
        let path = binding_path(id)?;
        let old_binding = load_binding(id)?;
        let port = match old_binding.as_ref() { Some(old) => old.port, None => allocate_port()? };
        let mut active_uri = None;
        let mut runtime = None;
        let mut last_error = "代理线路组没有可用线路".to_string();
        RUNTIMES.lock().map_err(|_| "代理运行状态锁异常")?.remove(id);
        for uri in &selection.uris {
            let spec = parse(uri)?;
            if let Ok(url) = url::Url::parse(&spec.normalized_uri) {
                if matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                    && url.port() == Some(port)
                { return Err("上游代理不能指向账号自身监听端口".into()); }
            }
            match spawn_core(&spec, port) {
                Ok(value) => {
                    if selection.group_id.is_some() && probe_proxy_port(port).is_err() {
                        drop(value);
                        last_error = "代理线路健康检测失败".into();
                        continue;
                    }
                    active_uri = Some(spec.normalized_uri); runtime = Some(value); break;
                }
                Err(error) => last_error = error,
            }
        }
        let active_uri = active_uri.ok_or(last_error)?;
        let runtime = runtime.ok_or("代理线路组启动失败")?;
        let fallback_uris = selection.uris.iter().filter(|uri| *uri != &active_uri).cloned().collect();
        let binding = Binding {
            last_exit_ip: old_binding.as_ref().filter(|old| old.uri == active_uri).and_then(|old| old.last_exit_ip.clone()),
            last_probe_at: old_binding.as_ref().filter(|old| old.uri == active_uri).and_then(|old| old.last_probe_at),
            last_probe_latency_ms: old_binding.as_ref().filter(|old| old.uri == active_uri).and_then(|old| old.last_probe_latency_ms),
            uri: active_uri,
            port,
            fallback_uris,
            group_id: selection.group_id,
            group_label: selection.group_label,
            failover_count: old_binding.as_ref().map(|old| old.failover_count).unwrap_or(0),
        };
        std::fs::create_dir_all(path.parent().ok_or("无效代理目录")?).map_err(|_| "创建代理存储目录失败")?;
        let content = super::secure_account_storage::serialize_account_file("codex-account-proxy", &binding)?;
        super::atomic_write::write_string_atomic(&path, &content).map_err(|_| "保存加密代理配置失败")?;
        let mut runtimes = RUNTIMES.lock().map_err(|_| "代理运行状态锁异常")?;
        runtimes.remove(id);
        runtimes.insert(id.to_string(), runtime);
        if binding.group_id.is_some() { begin_group_monitor(id); }
    }
    status(id)
}

pub fn record_probe(id: &str, exit_ip: String, latency_ms: u64) -> Result<(), String> {
    let _guard = STORE_LOCK.lock().map_err(|_| "代理配置锁异常")?;
    let mut binding = load_binding(id)?.ok_or("账号尚未绑定代理")?;
    binding.last_exit_ip = Some(exit_ip);
    binding.last_probe_at = Some(chrono::Utc::now().timestamp_millis());
    binding.last_probe_latency_ms = Some(latency_ms);
    let path = binding_path(id)?;
    let encrypted = super::secure_account_storage::serialize_account_file("codex-account-proxy", &binding)?;
    super::atomic_write::write_string_atomic(&path, &encrypted)
        .map_err(|_| "保存出口检测结果失败".to_string())
}

pub fn delete_for_removed_account(id: &str) -> Result<(), String> {
    let _guard = STORE_LOCK.lock().map_err(|_| "代理配置锁异常")?;
    for path in [binding_path(id)?, required_marker_path(id)?] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("删除已移除账号的代理配置失败".into()),
        }
    }
    RUNTIMES
        .lock()
        .map_err(|_| "代理运行状态锁异常")?
        .remove(id);
    Ok(())
}

pub fn shutdown() {
    if let Ok(mut runtimes) = RUNTIMES.lock() {
        runtimes.clear();
    }
}

pub fn proxy_env(proxy: &str) -> Vec<(String, String)> {
    let mut result: Vec<_> = [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ]
    .into_iter()
    .map(|key| (key.into(), proxy.into()))
    .collect();
    for name in ["NO_PROXY", "no_proxy"] {
        result.push((name.into(), "localhost,127.0.0.1,::1".into()));
    }
    result
}

pub fn profile_account_id(profile: &Path) -> Result<Option<String>, String> {
    if is_isolated_profile(profile) {
        let id =
            std::fs::read_to_string(profile.join(MARKER)).map_err(|_| "读取隔离账号标记失败")?;
        if let Some(instance) = super::codex_instance::load_instance_store()?
            .instances
            .iter()
            .find(|i| Path::new(&i.user_data_dir) == profile)
        {
            validate_isolated_binding(profile, instance.bind_account_id.as_deref())?;
        }
        return Ok(Some(id.trim().to_string()));
    }
    let store = super::codex_instance::load_instance_store()?;
    let account = store
        .instances
        .iter()
        .find(|i| Path::new(&i.user_data_dir) == profile)
        .and_then(|i| i.bind_account_id.clone());
    if account.is_some() {
        return Ok(account);
    }
    if profile == super::codex_instance::get_default_codex_home()? {
        return Ok(super::codex_account::get_current_account().map(|a| a.id));
    }
    Ok(None)
}

pub fn profile_proxy(profile: &Path) -> Result<Option<String>, String> {
    // The default desktop profile is a client of the local API Service, not a
    // member of its upstream account pool. Its synthetic binding has no
    // per-account proxy; selected upstream accounts are routed separately.
    if profile == super::codex_instance::get_default_codex_home()?
        && super::codex_instance::load_default_settings()?
            .bind_account_id
            .as_deref()
            .is_some_and(super::codex_instance::is_api_service_bind_account_id)
    {
        return Ok(None);
    }
    match profile_account_id(profile)? {
        Some(id) => {
            let proxy = endpoint(&id)?;
            if proxy.is_none() {
                return Err("隔离实例必须先绑定独立代理，已阻止无代理启动".into());
            }
            Ok(proxy)
        }
        None if profile == super::codex_instance::get_default_codex_home()? => {
            Err("默认 Codex 实例尚未绑定账号代理，已阻止无代理启动".into())
        }
        None => Ok(None),
    }
}

pub fn ensure_isolated_instance(id: &str) -> Result<String, String> {
    if endpoint(id)?.is_none() {
        return Err("请先保存此账号的独立代理".into());
    }
    let profile = isolation_dir(id)?;
    let store = super::codex_instance::load_instance_store()?;
    if let Some(instance) = store
        .instances
        .iter()
        .find(|i| Path::new(&i.user_data_dir) == profile)
    {
        if instance.bind_account_id.as_deref() != Some(id) {
            return Err("隔离实例被绑定到其他账号，请先在实例管理中修正".into());
        }
        return Ok(instance.id.clone());
    }
    std::fs::create_dir_all(&profile).map_err(|_| "创建账号隔离目录失败")?;
    super::atomic_write::write_string_atomic(&profile.join(MARKER), id)
        .map_err(|_| "写入隔离账号标记失败")?;
    let instance =
        super::codex_instance::create_instance(super::codex_instance::CreateInstanceParams {
            name: format!("隔离-{}", &key(id)[..12]),
            user_data_dir: profile.to_string_lossy().into_owned(),
            working_dir: None,
            extra_args: String::new(),
            bind_account_id: Some(id.into()),
            model_routing: None,
            copy_source_instance_id: None,
            init_mode: Some("existingDir".into()),
            launch_mode: None,
            app_speed: None,
        })?;
    Ok(instance.id)
}
