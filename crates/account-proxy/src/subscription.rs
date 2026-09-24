//! Offline parsing only. The caller fetches the subscription without logging its URL or body.
use super::{parse, percent_encode_userinfo_component};
use base64::{engine::general_purpose::{STANDARD, URL_SAFE}, Engine};
use serde_yaml::Value;
use std::collections::HashSet;

pub struct SubscriptionNode {
    pub label: String,
    pub normalized_uri: String,
}

pub struct SubscriptionParseResult {
    pub nodes: Vec<SubscriptionNode>,
    pub skipped: usize,
    pub format: &'static str,
}

fn component(value: &str) -> String {
    percent_encode_userinfo_component(value)
}

fn host(value: &str) -> String {
    if value.contains(':') && !value.starts_with('[') {
        format!("[{value}]")
    } else {
        value.to_string()
    }
}

fn field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|item| match item {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

fn is_true(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool) == Some(true)
}

fn alpn(value: &Value) -> Option<String> {
    let item = value.get("alpn")?;
    if let Some(text) = item.as_str() { return Some(text.to_string()); }
    item.as_sequence().map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(","))
}

fn query(pairs: Vec<(&str, String)>) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        if !value.is_empty() { serializer.append_pair(key, &value); }
    }
    serializer.finish()
}

fn clash_link(value: &Value) -> Option<String> {
    let kind = field(value, "type")?.to_ascii_lowercase();
    let name = field(value, "name")?;
    let server = field(value, "server")?;
    let port: u16 = field(value, "port")?.parse().ok()?;
    if name.is_empty() || server.is_empty() || port == 0 { return None; }
    let address = format!("{}:{port}", host(&server));
    let fragment = component(&name);
    match kind.as_str() {
        "hysteria2" | "hy2" => {
            let password = field(value, "password").or_else(|| field(value, "auth"))?;
            let mut pairs = vec![];
            if let Some(sni) = field(value, "sni") { pairs.push(("sni", sni)); }
            if is_true(value, "skip-cert-verify") { pairs.push(("insecure", "1".into())); }
            if let Some(alpn) = alpn(value) { pairs.push(("alpn", alpn)); }
            if let Some(obfs) = field(value, "obfs") { pairs.push(("obfs", obfs)); }
            if let Some(secret) = field(value, "obfs-password") { pairs.push(("obfs-password", secret)); }
            Some(format!("hysteria2://{}@{address}?{}#{fragment}", component(&password), query(pairs)))
        }
        "tuic" => {
            let uuid = field(value, "uuid")?;
            let password = field(value, "password")?;
            let mut pairs = vec![];
            if let Some(sni) = field(value, "sni") { pairs.push(("sni", sni)); }
            if is_true(value, "skip-cert-verify") { pairs.push(("insecure", "1".into())); }
            if let Some(alpn) = alpn(value) { pairs.push(("alpn", alpn)); }
            if let Some(control) = field(value, "congestion-controller")
                .or_else(|| field(value, "congestion_control")) {
                pairs.push(("congestion_control", control));
            }
            Some(format!("tuic://{}@{address}?{}#{fragment}", component(&format!("{uuid}:{password}")), query(pairs)))
        }
        "vless" => {
            if is_true(value, "skip-cert-verify") { return None; }
            let uuid = field(value, "uuid")?;
            let reality = value.get("reality-opts");
            let security = if reality.is_some() { "reality" } else if is_true(value, "tls") { "tls" } else { "none" };
            let network = field(value, "network").unwrap_or_else(|| "tcp".into());
            let mut pairs = vec![("encryption", "none".into()), ("security", security.into()), ("type", network)];
            if let Some(sni) = field(value, "servername").or_else(|| field(value, "sni")) { pairs.push(("sni", sni)); }
            if let Some(fp) = field(value, "client-fingerprint") { pairs.push(("fp", fp)); }
            if let Some(flow) = field(value, "flow") { pairs.push(("flow", flow)); }
            if let Some(reality) = reality {
                if let Some(pbk) = field(reality, "public-key") { pairs.push(("pbk", pbk)); }
                if let Some(sid) = field(reality, "short-id") { pairs.push(("sid", sid)); }
            }
            if let Some(ws) = value.get("ws-opts") {
                if let Some(path) = field(ws, "path") { pairs.push(("path", path)); }
                if let Some(headers) = ws.get("headers") {
                    if let Some(host) = field(headers, "Host").or_else(|| field(headers, "host")) {
                        pairs.push(("host", host));
                    }
                }
            }
            Some(format!("vless://{}@{address}?{}#{fragment}", component(&uuid), query(pairs)))
        }
        "http" | "socks5" | "socks" => {
            let scheme = if kind == "socks" { "socks5" } else { kind.as_str() };
            let user = field(value, "username").unwrap_or_default();
            let password = field(value, "password").unwrap_or_default();
            let auth = if user.is_empty() { String::new() }
                else { format!("{}:{}@", component(&user), component(&password)) };
            if user.is_empty() && !password.is_empty() { return None; }
            Some(format!("{scheme}://{auth}{address}#{fragment}"))
        }
        _ => None,
    }
}

fn decode_base64(content: &str) -> Option<String> {
    let mut compact: String = content.chars().filter(|ch| !ch.is_whitespace()).collect();
    while compact.len() % 4 != 0 { compact.push('='); }
    let bytes = STANDARD.decode(&compact).or_else(|_| URL_SAFE.decode(&compact)).ok()?;
    String::from_utf8(bytes).ok()
}

pub fn parse_subscription(content: &str) -> Result<SubscriptionParseResult, String> {
    if content.len() > 2 * 1024 * 1024 { return Err("订阅内容超过 2 MiB 上限".into()); }
    let mut nodes = Vec::new();
    let mut seen = HashSet::new();
    let mut skipped = 0;
    if let Ok(document) = serde_yaml::from_str::<Value>(content) {
        if let Some(proxies) = document.get("proxies").and_then(Value::as_sequence) {
            for entry in proxies.iter().take(2000) {
                if let Some(link) = clash_link(entry) {
                    if let Ok(spec) = parse(&link) {
                        if seen.insert(spec.normalized_uri.clone()) {
                            nodes.push(SubscriptionNode { label: spec.label, normalized_uri: spec.normalized_uri });
                        }
                        continue;
                    }
                }
                skipped += 1;
            }
            skipped += proxies.len().saturating_sub(2000);
            return if nodes.is_empty() { Err("订阅没有可用的受支持节点".into()) }
                else { Ok(SubscriptionParseResult { nodes, skipped, format: "Clash YAML" }) };
        }
    }
    let plain = if content.contains("://") { content.to_string() }
        else { decode_base64(content).ok_or("订阅既不是 Clash YAML，也不是 V2Ray 节点列表/Base64")? };
    for line in plain.lines().take(2000) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if let Ok(spec) = parse(line) {
            if seen.insert(spec.normalized_uri.clone()) {
                nodes.push(SubscriptionNode { label: spec.label, normalized_uri: spec.normalized_uri });
            }
        } else {
            skipped += 1;
        }
    }
    skipped += plain.lines().count().saturating_sub(2000);
    if nodes.is_empty() { return Err("订阅没有可用的受支持节点".into()); }
    Ok(SubscriptionParseResult { nodes, skipped, format: "V2Ray 节点列表" })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imports_base64_share_list_without_echoing_secrets() {
        let source = "hysteria2://synthetic-secret@192.0.2.10:443?sni=example.com#HY2\n\
tuic://11111111-2222-4333-8444-555555555555%3Asynthetic-password@198.51.100.20:443?sni=example.com&alpn=h3#TUIC\n\
vmess://unsupported";
        let encoded = STANDARD.encode(source);
        let result = parse_subscription(&encoded).unwrap();
        assert_eq!(result.nodes.len(), 2);
        assert_eq!(result.skipped, 1);
        assert_eq!(result.nodes[0].label, "HY2");
    }
    #[test]
    fn imports_clash_yaml_supported_subset_and_counts_unsupported() {
        let yaml = r#"
proxies:
  - name: Example-HY2
    type: hysteria2
    server: 192.0.2.10
    port: 443
    password: synthetic-secret
    sni: example.com
    skip-cert-verify: true
  - name: Example-TUIC
    type: tuic
    server: 198.51.100.20
    port: 443
    uuid: 11111111-2222-4333-8444-555555555555
    password: synthetic-password
    sni: example.com
    alpn: [h3]
    congestion-controller: bbr
  - name: Unsupported
    type: vmess
    server: 203.0.113.20
    port: 443
"#;
        let result = parse_subscription(yaml).unwrap();
        assert_eq!(result.format, "Clash YAML");
        assert_eq!(result.nodes.len(), 2);
        assert_eq!(result.skipped, 1);
        assert!(result.nodes[0].normalized_uri.contains("insecure=1"));
    }
    #[test]
    fn imports_clash_yaml_socks5_username_with_spaces() {
        let yaml = r#"
proxies:
  - name: 1024-US-NewYork
    type: socks5
    server: us.1024proxy.io
    port: 3000
    username: "e4uu743633-region-US-st-New York-city-New York City-sid-c5fVZ2BT-t-120"
    password: synthetic-password
"#;
        let result = parse_subscription(yaml).unwrap();
        assert_eq!(result.nodes.len(), 1);
        let spec = parse(&result.nodes[0].normalized_uri).unwrap();
        let server = &spec.xray_config(32200)["outbounds"][0]["settings"]["servers"][0];
        assert_eq!(
            server["users"][0]["user"],
            "e4uu743633-region-US-st-New York-city-New York City-sid-c5fVZ2BT-t-120"
        );
        assert_eq!(server["users"][0]["pass"], "synthetic-password");
    }
}
