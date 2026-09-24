//! Strict, secret-safe share-link parsing. No network or application state.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use url::Url;

mod subscription;
pub use subscription::{parse_subscription, SubscriptionNode, SubscriptionParseResult};

// Deliberately no Debug: links and outbound JSON contain proxy credentials.
pub struct ProxySpec {
    pub normalized_uri: String,
    pub label: String,
    pub protocol: String,
    outbound: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyCore {
    Xray,
    SingBox,
}

fn decode(value: &str) -> Result<String, String> {
    // Decode percent escapes without treating a literal '+' as a space.
    url::form_urlencoded::parse(format!("v={}", value.replace('+', "%2B")).as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .ok_or_else(|| "代理字段编码无效".into())
}

pub fn normalize_share_link(input: &str) -> String {
    let mut value = input.trim().trim_matches('`').trim().to_string();
    // Chat clients sometimes autolink an email-shaped credential inside a
    // pasted share URI. Only undo a mailto link when its visible text matches.
    while let Some(open) = value.find('[') {
        let Some(close_relative) = value[open..].find("](mailto:") else { break };
        let close = open + close_relative;
        let href_start = close + "](mailto:".len();
        let Some(end_relative) = value[href_start..].find(')') else { break };
        let end = href_start + end_relative;
        let visible = value[open + 1..close].to_string();
        if visible != value[href_start..end] || !visible.contains('@') { break }
        value.replace_range(open..=end, &visible);
    }
    for tail in ["&#x20;", "&#32;", "&nbsp;"] {
        while value.ends_with(tail) {
            value.truncate(value.len() - tail.len());
            value = value.trim_end().to_string();
        }
    }
    value = value.replace("&amp;", "&");
    for ch in [':', '/', '@', '_', '#', '?', '&', '=', '+', '-'] {
        value = value.replace(&format!("\\{ch}"), &ch.to_string());
    }
    value.trim().to_string()
}

pub fn parse(input: &str) -> Result<ProxySpec, String> {
    let normalized_uri = normalize_share_link(input);
    if normalized_uri.is_empty()
        || normalized_uri.len() > 8192
        || normalized_uri.contains("](mailto:")
        || normalized_uri
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
    {
        return Err("请粘贴一条完整代理链接（不能包含空白或多行，最长 8192 字符）".into());
    }
    let url = Url::parse(&normalized_uri).map_err(|_| "代理链接格式无效".to_string())?;
    let host = url
        .host_str()
        .filter(|s| !s.is_empty())
        .ok_or("代理链接缺少服务器地址")?
        .trim_matches(['[', ']']);
    let port = url
        .port_or_known_default()
        .filter(|p| *p > 0)
        .ok_or("代理链接缺少有效端口（1–65535）")?;
    let protocol = url.scheme().to_string();
    let label = match url.fragment().filter(|s| !s.is_empty()) {
        Some(fragment) => decode(fragment)?,
        None => format!("{protocol} · {host}:{port}"),
    };
    if label.chars().count() > 160 || label.chars().any(char::is_control) {
        return Err("节点名称过长或包含控制字符".into());
    }
    let outbound = match protocol.as_str() {
        "vless" => vless_outbound(&url, host, port)?,
        "hysteria2" | "hy2" => hysteria2_outbound(&url, host, port)?,
        "tuic" => tuic_outbound(&url, host, port)?,
        "http" | "https" | "socks5" | "socks5h" => {
            if url.query().is_some() || !matches!(url.path(), "" | "/") {
                return Err("HTTP / SOCKS 代理只支持 协议://用户名:密码@服务器:端口".into());
            }
            let mut server = json!({"address": host, "port": port});
            if !url.username().is_empty() {
                server["users"] = json!([{"user": decode(url.username())?, "pass": decode(url.password().unwrap_or(""))?}]);
            } else if url.password().is_some() {
                return Err("代理密码需要配套用户名".into());
            }
            let mut outbound = json!({
                "tag": "account-proxy",
                "protocol": if protocol.starts_with("socks") {"socks"} else {"http"},
                "settings": {"servers": [server]}
            });
            if protocol == "https" {
                outbound["streamSettings"] = json!({"network": "tcp", "security": "tls", "tlsSettings": {"serverName": host, "allowInsecure": false}});
            }
            outbound
        }
        _ => {
            return Err(
                "暂不支持该协议；支持 VLESS、Hysteria2、TUIC、HTTP(S)、SOCKS5".into(),
            )
        }
    };
    Ok(ProxySpec {
        normalized_uri,
        label,
        protocol,
        outbound,
    })
}

fn strict_query(url: &Url, allowed: &[&str]) -> Result<BTreeMap<String, String>, String> {
    let mut query = BTreeMap::new();
    for (key, value) in url.query_pairs() {
        if !allowed.contains(&key.as_ref()) || query.insert(key.into_owned(), value.into_owned()).is_some() {
            return Err("代理链接包含不支持或重复的参数".into());
        }
    }
    Ok(query)
}

fn bool_flag(value: Option<&String>) -> Result<bool, String> {
    match value.map(String::as_str).unwrap_or("0") {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => Err("insecure 参数只能为 0 或 1".into()),
    }
}

fn sing_tls(host: &str, query: &BTreeMap<String, String>) -> Result<Value, String> {
    let server_name = query.get("sni").map(String::as_str).unwrap_or(host);
    if server_name.is_empty() { return Err("TLS 节点缺少 SNI".into()) }
    let mut tls = json!({"enabled": true, "server_name": server_name,
        "insecure": bool_flag(query.get("insecure"))?});
    if let Some(alpn) = query.get("alpn") {
        let protocols = alpn.split(',').filter(|value| !value.is_empty()).collect::<Vec<_>>();
        if protocols.is_empty() { return Err("ALPN 列表为空".into()) }
        tls["alpn"] = json!(protocols);
    }
    Ok(tls)
}

fn hysteria2_outbound(url: &Url, host: &str, port: u16) -> Result<Value, String> {
    if !matches!(url.path(), "" | "/") { return Err("Hysteria2 链接路径无效".into()) }
    let query = strict_query(url, &["sni", "insecure", "alpn", "obfs", "obfs-password", "upmbps", "downmbps"])?;
    let password = if let Some(raw_password) = url.password() {
        format!("{}:{}", decode(url.username())?, decode(raw_password)?)
    } else { decode(url.username())? };
    if password.is_empty() { return Err("Hysteria2 节点缺少认证密码".into()) }
    let mut outbound = json!({"type":"hysteria2", "tag":"account-proxy", "server":host,
        "server_port":port, "password":password, "tls":sing_tls(host, &query)?});
    if let Some(obfs) = query.get("obfs") {
        if obfs != "salamander" { return Err("Hysteria2 当前只支持 salamander 混淆".into()) }
        let secret = query.get("obfs-password").filter(|value| !value.is_empty())
            .ok_or("Hysteria2 混淆缺少密码")?;
        outbound["obfs"] = json!({"type":"salamander", "password":secret});
    }
    for (source, target) in [("upmbps", "up_mbps"), ("downmbps", "down_mbps")] {
        if let Some(value) = query.get(source) {
            let rate = value.parse::<u32>().map_err(|_| "Hysteria2 带宽参数无效")?;
            if rate == 0 { return Err("Hysteria2 带宽参数必须大于零".into()) }
            outbound[target] = json!(rate);
        }
    }
    Ok(outbound)
}

fn tuic_outbound(url: &Url, host: &str, port: u16) -> Result<Value, String> {
    if !matches!(url.path(), "" | "/") { return Err("TUIC 链接路径无效".into()) }
    let query = strict_query(url, &["sni", "insecure", "alpn", "congestion_control"])?;
    let user_info = decode(url.username())?;
    let (uuid, password) = if let Some(raw_password) = url.password() {
        (user_info, decode(raw_password)?)
    } else {
        let (uuid, password) = user_info.split_once(':').ok_or("TUIC 链接缺少 UUID 或密码")?;
        (uuid.to_string(), password.to_string())
    };
    uuid::Uuid::parse_str(&uuid).map_err(|_| "TUIC 用户 UUID 格式无效")?;
    if password.is_empty() { return Err("TUIC 节点缺少密码".into()) }
    let congestion = query.get("congestion_control").map(String::as_str).unwrap_or("cubic");
    if !["cubic", "new_reno", "bbr"].contains(&congestion) {
        return Err("TUIC 拥塞控制参数无效".into());
    }
    Ok(json!({"type":"tuic", "tag":"account-proxy", "server":host,
        "server_port":port, "uuid":uuid, "password":password,
        "congestion_control":congestion, "tls":sing_tls(host, &query)?}))
}

fn vless_outbound(url: &Url, host: &str, port: u16) -> Result<Value, String> {
    if !matches!(url.path(), "" | "/") || url.password().is_some() {
        return Err("VLESS 链接中的用户或路径格式无效".into());
    }
    let id = decode(url.username())?;
    uuid::Uuid::parse_str(&id).map_err(|_| "VLESS 用户 ID 必须是有效 UUID".to_string())?;
    let mut query = BTreeMap::new();
    for (key, value) in url.query_pairs() {
        if ![
            "encryption",
            "flow",
            "security",
            "sni",
            "fp",
            "pbk",
            "sid",
            "spx",
            "type",
            "headerType",
            "path",
            "host",
            "alpn",
            "allowInsecure",
        ]
        .contains(&key.as_ref())
        {
            // Do not echo arbitrary fields: they may contain secrets.
            return Err("节点包含当前版本不支持的参数，请使用 TCP/WS 的标准 VLESS 分享链接".into());
        }
        if query.insert(key.into_owned(), value.into_owned()).is_some() {
            return Err("节点包含重复参数，无法安全确定配置".into());
        }
    }
    let q = |key: &str, default: &str| query.get(key).cloned().unwrap_or_else(|| default.into());
    if q("encryption", "none") != "none"
        || q("headerType", "none") != "none"
        || !["0", "false"].contains(&q("allowInsecure", "0").as_str())
    {
        return Err("不支持该加密/伪装参数，且不允许跳过 TLS 证书验证".into());
    }
    let network = q("type", "tcp");
    let network = if network == "raw" {
        "tcp".to_string()
    } else {
        network
    };
    if !["tcp", "ws"].contains(&network.as_str()) {
        return Err("当前版本的 VLESS 仅支持 TCP（raw）和 WebSocket 传输".into());
    }
    let security = q("security", "none");
    if !["none", "tls", "reality"].contains(&security.as_str()) {
        return Err("不支持的 VLESS 安全类型".into());
    }
    let flow = q("flow", "");
    if !flow.is_empty() && (flow != "xtls-rprx-vision" || network != "tcp" || security == "none") {
        return Err("Vision 仅支持 TCP + TLS/Reality".into());
    }
    let mut stream = json!({"network": network, "security": security});
    if security == "reality" {
        if network != "tcp" {
            return Err("当前版本 Reality 仅支持 TCP".into());
        }
        let public_key = q("pbk", "");
        if URL_SAFE_NO_PAD
            .decode(&public_key)
            .map(|v| v.len())
            .unwrap_or(0)
            != 32
        {
            return Err("Reality 公钥 pbk 必须是 32 字节的 Base64URL 公钥".into());
        }
        let short_id = q("sid", "");
        if short_id.len() > 16
            || short_id.len() % 2 != 0
            || !short_id.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("Reality sid 必须是最多 16 位、偶数长度的十六进制字符串".into());
        }
        let server_name = q("sni", "");
        if server_name.is_empty() {
            return Err("Reality 节点缺少 sni".into());
        }
        stream["realitySettings"] = json!({
            "serverName": server_name, "fingerprint": q("fp", "chrome"),
            "publicKey": public_key, "shortId": short_id, "spiderX": q("spx", "/")
        });
    } else if security == "tls" {
        let mut tls = json!({"serverName": q("sni", host), "allowInsecure": false, "fingerprint": q("fp", "chrome")});
        if let Some(alpn) = query.get("alpn") {
            tls["alpn"] = json!(alpn.split(',').collect::<Vec<_>>());
        }
        stream["tlsSettings"] = tls;
    }
    if network == "ws" {
        let mut ws = json!({"path": q("path", "/")});
        if let Some(host) = query.get("host") {
            ws["headers"] = json!({"Host": host});
        }
        stream["wsSettings"] = ws;
    }
    Ok(json!({
        "tag": "account-proxy", "protocol": "vless",
        "settings": {"vnext": [{"address": host, "port": port, "users": [{"id": id, "encryption": "none", "flow": flow}]}]},
        "streamSettings": stream, "mux": {"enabled": false}
    }))
}

impl ProxySpec {
    pub fn core(&self) -> ProxyCore {
        if matches!(self.protocol.as_str(), "hysteria2" | "hy2" | "tuic") {
            ProxyCore::SingBox
        } else {
            ProxyCore::Xray
        }
    }

    pub fn sing_box_config(&self, port: u16) -> Value {
        json!({
            "log": {"level": "error"},
            "inbounds": [{"type":"http", "tag":"account-http", "listen":"127.0.0.1", "listen_port":port}],
            "outbounds": [self.outbound],
            "route": {"final":"account-proxy"}
        })
    }
    /// One loopback listener and one proxy-only outbound; never a freedom/direct fallback.
    pub fn xray_config(&self, port: u16) -> Value {
        json!({
            "log": {"loglevel": "none"},
            "inbounds": [{"listen": "127.0.0.1", "port": port, "protocol": "http", "settings": {}, "sniffing": {"enabled": false}}],
            "outbounds": [self.outbound]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const VLESS: &str = "vless://11111111-2222-4333-8444-555555555555@192.0.2.2:8443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=example.com&fp=chrome&pbk=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&sid=2fe2&spx=%2F&type=tcp&headerType=none#%E8%8A%82%E7%82%B9";

    #[test]
    fn parses_reality_vision_and_encoded_label() {
        let spec = parse(VLESS).unwrap();
        assert_eq!(spec.label, "节点");
        let config = spec.xray_config(32123);
        assert_eq!(config["inbounds"][0]["listen"], "127.0.0.1");
        assert_eq!(config["outbounds"].as_array().unwrap().len(), 1);
        assert_eq!(
            config["outbounds"][0]["settings"]["vnext"][0]["users"][0]["flow"],
            "xtls-rprx-vision"
        );
        assert_eq!(
            config["outbounds"][0]["streamSettings"]["realitySettings"]["spiderX"],
            "/"
        );
        assert!(!config.to_string().contains("freedom"));
    }
    #[test]
    fn accepts_pasted_markdown_html_escapes() {
        let pasted = format!(
            " {}&#x20; ",
            VLESS
                .replace("vless:", "vless\\:")
                .replace('@', "\\@")
                .replace('&', "&amp;")
        );
        assert_eq!(parse(&pasted).unwrap().normalized_uri, VLESS);
    }
    #[test]
    fn decodes_proxy_credentials_and_ipv6_without_leaking_them_into_label() {
        let spec = parse("socks5://a%40b:p%3Ass+word@[::1]:1080").unwrap();
        assert!(!spec.label.contains("word"));
        let server = &spec.xray_config(32000)["outbounds"][0]["settings"]["servers"][0];
        assert_eq!(server["address"], "::1");
        assert_eq!(server["users"][0]["user"], "a@b");
        assert_eq!(server["users"][0]["pass"], "p:ss+word");
    }
    #[test]
    fn rejects_bad_protocols_ports_keys_duplicates_and_unsupported_transports() {
        for value in [
            "file:///secret",
            "http://host:0",
            "socks5://host",
            "http://host:8080/path",
            "http://host:8080\nhttp://other:80",
        ] {
            assert!(parse(value).is_err());
        }
        for value in [
            VLESS.replace("type=tcp", "type=grpc"),
            VLESS.replace("sid=2fe2", "sid=xyz"),
            VLESS.replace(
                "pbk=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                "pbk=secret",
            ),
            VLESS.replace("type=tcp", "type=tcp&type=ws"),
            VLESS.replace("headerType=none", "headerType=http"),
        ] {
            assert!(parse(&value).is_err());
        }
    }
    #[test]
    fn supports_tls_websocket_and_http_defaults() {
        let ws = parse("vless://11111111-2222-4333-8444-555555555555@example.com:443?security=tls&type=ws&path=%2Fws&host=example.com").unwrap().xray_config(32000);
        assert_eq!(
            ws["outbounds"][0]["streamSettings"]["wsSettings"]["path"],
            "/ws"
        );
        assert_eq!(
            parse("https://example.com").unwrap().xray_config(32000)["outbounds"][0]["settings"]
                ["servers"][0]["port"],
            443
        );
    }
    #[test]
    fn supports_hysteria2_and_tuic_with_sing_box_only() {
        let hy2 = parse("hysteria2://synthetic-secret@192.0.2.10:443?sni=example.com&insecure=1#%E6%B5%8B%E8%AF%95").unwrap();
        assert_eq!(hy2.core(), ProxyCore::SingBox);
        assert_eq!(hy2.label, "测试");
        let hy2_config = hy2.sing_box_config(32001);
        assert_eq!(hy2_config["inbounds"][0]["listen"], "127.0.0.1");
        assert_eq!(hy2_config["outbounds"][0]["type"], "hysteria2");
        assert_eq!(hy2_config["outbounds"][0]["tls"]["insecure"], true);
        assert!(!hy2_config.to_string().contains("\"type\":\"direct\""));

        let tuic = parse("tuic://11111111-2222-4333-8444-555555555555%3Asynthetic-password@198.51.100.20:443?sni=example.com&alpn=h3&congestion_control=bbr").unwrap();
        assert_eq!(tuic.core(), ProxyCore::SingBox);
        let tuic_config = tuic.sing_box_config(32002);
        assert_eq!(tuic_config["outbounds"][0]["type"], "tuic");
        assert_eq!(tuic_config["outbounds"][0]["uuid"], "11111111-2222-4333-8444-555555555555");
        assert_eq!(tuic_config["outbounds"][0]["password"], "synthetic-password");
        assert_eq!(tuic_config["outbounds"][0]["tls"]["alpn"][0], "h3");
    }
    #[test]
    fn restores_mailto_autolink_only_when_visible_text_matches() {
        let input = "hysteria2://prefix-[part@example.invalid](mailto:part@example.invalid):443?sni=example.invalid";
        assert_eq!(parse(input).unwrap().normalized_uri,
            "hysteria2://prefix-part@example.invalid:443?sni=example.invalid");
        assert!(parse("hysteria2://[safe@example.invalid](mailto:other@example.invalid):443").is_err());
    }
}
