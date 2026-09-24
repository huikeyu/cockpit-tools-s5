// Acceptance test used by the release executable's --proxy-self-test switch.
// Only loopback mock servers and synthetic account IDs are used.
pub fn self_test() -> Result<serde_json::Value, String> {
    use std::io::Read;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    struct ProbeServer {
        port: u16,
        hits: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
        task: Option<std::thread::JoinHandle<()>>,
    }
    impl Drop for ProbeServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(task) = self.task.take() {
                let _ = task.join();
            }
        }
    }
    fn headers(stream: &mut TcpStream) -> Result<String, String> {
        let mut value = Vec::new();
        let mut byte = [0u8; 1];
        while value.len() < 16384 && !value.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).map_err(|e| e.to_string())?;
            value.push(byte[0]);
        }
        Ok(String::from_utf8_lossy(&value).into_owned())
    }
    fn server(label: &'static str) -> Result<ProbeServer, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (worker_hits, worker_stop) = (hits.clone(), stop.clone());
        let task = std::thread::spawn(move || {
            while !worker_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        worker_hits.fetch_add(1, Ordering::SeqCst);
                        // Windows accepted sockets inherit the listener's nonblocking flag.
                        // The mock protocol reader is synchronous; wait for the post-CONNECT request.
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let Ok(first) = headers(&mut stream) else {
                            continue;
                        };
                        if first.starts_with("CONNECT ") {
                            let _ =
                                stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n");
                            // Xray sends the actual HTTP request after CONNECT. Read it before
                            // answering; otherwise the response can race the proxy's write and
                            // Windows may abort the socket with WSAECONNABORTED.
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                            if headers(&mut stream).is_err() {
                                continue;
                            }
                        }
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            label.len(),
                            label
                        );
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Ok(ProbeServer {
            port,
            hits,
            stop,
            task: Some(task),
        })
    }
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            shutdown();
        }
    }
    let _cleanup = Cleanup;
    let a = server("ACCOUNT_A")?;
    let b = server("ACCOUNT_B")?;
    let direct = server("DIRECT_LEAK")?;
    let a_uri = format!("http://127.0.0.1:{}", a.port);
    if client("selftest-unbound", Duration::from_secs(1)).is_ok()
        || effective_proxy("selftest-unbound", Some("http://127.0.0.1:8080")).is_ok()
    {
        return Err("Unbound account could use global or direct networking".into());
    }
    let a_status = save("selftest-a", Some(&a_uri))?;
    record_probe("selftest-a", "192.0.2.10".into(), 42)?;
    let observed = status("selftest-a")?;
    if observed.last_exit_ip.as_deref() != Some("192.0.2.10")
        || observed.last_probe_latency_ms != Some(42)
        || observed.server_host.as_deref() != Some("127.0.0.1")
    {
        return Err("Proxy card metadata did not persist".into());
    }
    let b_status = save("selftest-b", Some(&format!("http://127.0.0.1:{}", b.port)))?;
    if a_status.local_port == b_status.local_port {
        return Err("Accounts shared a local listener".into());
    }
    let stored = std::fs::read_to_string(binding_path("selftest-a")?).map_err(|e| e.to_string())?;
    if stored.contains(&a_uri) || !stored.contains("AES-256-GCM") {
        return Err("Proxy binding was not encrypted".into());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("http://127.0.0.1:{}/isolation-test", direct.port);
    runtime.block_on(async {
        let ca = client("selftest-a", Duration::from_secs(3))?;
        let cb = client("selftest-b", Duration::from_secs(3))?;
        let mut transient_failures = 0usize;
        let mut successful_pairs = 0usize;
        for attempt in 0..30 {
            let (ra, rb) = tokio::join!(ca.get(&url).send(), cb.get(&url).send());
            let (Ok(ra), Ok(rb)) = (ra, rb) else {
                transient_failures += 1;
                continue;
            };
            let (Ok(ba), Ok(bb)) = (ra.text().await, rb.text().await) else {
                transient_failures += 1;
                continue;
            };
            if ba != "ACCOUNT_A" || bb != "ACCOUNT_B" {
                return Err(format!(
                    "Cross-account proxy routing detected: A={ba:?}, B={bb:?}"
                ));
            }
            successful_pairs += 1;
            if successful_pairs == 5 { break; }
        }
        if successful_pairs < 5 {
            return Err(format!("Account proxy transport did not stabilize: successful_pairs={successful_pairs}, transient_failures={transient_failures}"));
        }
        RUNTIMES.lock().unwrap().remove("selftest-a");
        if ca.get(&url).send().await.is_ok() {
            return Err("Proxy failure did not block the request".into());
        }
        let occupied = TcpListener::bind(("127.0.0.1", a_status.local_port.unwrap()))
            .map_err(|e| e.to_string())?;
        if client("selftest-a", Duration::from_secs(1)).is_ok() {
            return Err("Occupied proxy port was accepted".into());
        }
        drop(occupied);
        let restarted = client("selftest-a", Duration::from_secs(3))?;
        let body = restarted
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())?;
        if body != "ACCOUNT_A" {
            return Err("Restart lost account binding".into());
        }
        Ok::<(), String>(())
    })?;
    if pending_client(Some("pre-auth"), Duration::from_secs(1)).is_ok() {
        return Err("Authentication proceeded without a pending proxy".into());
    }
    save_pending(Some("pre-auth"), Some(&a_uri))?;
    lock_pending("pre-auth")?;
    if save_pending(
        Some("pre-auth"),
        Some(&format!("http://127.0.0.1:{}", b.port)),
    )
    .is_ok()
    {
        return Err("Authentication proxy changed during login".into());
    }
    runtime.block_on(async {
        let pre_auth = pending_client(Some("pre-auth"), Duration::from_secs(3))?;
        let mut matched = false;
        for _ in 0..20 {
            if let Ok(response) = pre_auth.get(&url).send().await {
                if response.text().await.unwrap_or_default() == "ACCOUNT_A" {
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            return Err("Pending authentication request did not use proxy A".to_string());
        }
        Ok::<(), String>(())
    })?;
    promote_pending(Some("pre-auth"), "selftest-promoted")?;
    if pending_uri(Some("pre-auth")).is_some() || endpoint("selftest-promoted")?.is_none() {
        return Err("Pending proxy did not become a persistent account binding".into());
    }
    if direct.hits.load(Ordering::SeqCst) != 0 {
        return Err("Direct traffic leak detected".into());
    }
    let first = ensure_isolated_instance("selftest-a")?;
    if ensure_isolated_instance("selftest-a")? != first {
        return Err("Isolation instance was duplicated".into());
    }
    let second = ensure_isolated_instance("selftest-b")?;
    if validate_isolated_binding(&isolation_dir("selftest-a")?, Some("selftest-b")).is_ok() {
        return Err("Cross-account isolation rebinding was allowed".into());
    }
    if first == second || isolation_dir("selftest-a")? == isolation_dir("selftest-b")? {
        return Err("Isolation directory was shared".into());
    }
    for id in ["selftest-a", "selftest-b"] {
        let dir = isolation_dir(id)?;
        if !is_isolated_profile(&dir)
            || dir.join("skills").exists()
            || dir.join("sessions").exists()
        {
            return Err("Isolation profile copied shared data".into());
        }
    }
    let original = endpoint("selftest-a")?;
    if save("selftest-a", Some("vless://secret@invalid:0")).is_ok()
        || endpoint("selftest-a")? != original
    {
        return Err("Rejected input changed a saved binding".into());
    }
    let default_home = super::codex_instance::get_default_codex_home()?;
    if std::env::var("CODEX_HOME").ok().is_none_or(|value| value.trim().is_empty())
        && Some(default_home.clone()) != dirs::home_dir().map(|home| home.join(".codex"))
    {
        return Err("Default client profile was redirected away from the system Codex home".into());
    }
    if profile_proxy(&default_home).is_ok() {
        return Err("Unbound default profile could launch without an account proxy".into());
    }
    super::codex_instance::update_default_settings(
        Some(Some(super::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID.into())),
        None, None, Some(false), None, None,
    )?;
    if profile_proxy(&default_home)?.is_some() {
        return Err("API Service desktop client inherited an account proxy".into());
    }
    let reality = "vless://11111111-2222-4333-8444-555555555555@192.0.2.2:8443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=example.com&fp=chrome&pbk=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&sid=2fe2&spx=%2F&type=tcp&headerType=none";
    save("selftest-vless", Some(reality))?;
    let hy2 = "hysteria2://synthetic-secret@192.0.2.10:443?sni=example.com&insecure=1#Synthetic-HY2";
    let tuic = "tuic://11111111-2222-4333-8444-555555555555%3Asynthetic-password@198.51.100.20:443?sni=example.com&alpn=h3&congestion_control=bbr#Synthetic-TUIC";
    for (scheme, link) in [("hy2", hy2), ("tuic", tuic)] {
        let config = parse(link)?.sing_box_config(32001).to_string();
        let mut check = Command::new(sing_box_path()?)
            .args(["check", "-c", "stdin"])
            .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().map_err(|_| "无法运行 sing-box 配置检查")?;
        check.stdin.take().ok_or("sing-box 配置检查管道不可用")?
            .write_all(config.as_bytes()).map_err(|_| "写入 sing-box 检查配置失败")?;
        if !check.wait().map_err(|_| "等待 sing-box 配置检查失败")?.success() {
            return Err(format!("sing-box rejected synthetic {scheme} config"));
        }
    }
    save("selftest-hy2", Some(hy2))?;
    save("selftest-tuic", Some(tuic))?;
    if !status("selftest-hy2")?.running || !status("selftest-tuic")?.running {
        return Err("Bundled sing-box did not start both protocol listeners".into());
    }
    std::env::set_var("COCKPIT_PROXY_SELFTEST_PROBE_URL", &url);
    let first_item = inventory_save(None, "主线路", "测试接管组", &a_uri)?
        .into_iter().find(|item| item.server_port == a.port).ok_or("库存主线路未保存")?;
    inventory_save(None, "备用线路", "测试接管组", &format!("http://127.0.0.1:{}", b.port))?;
    save("selftest-group", Some(&format!("inventory://{}", first_item.id)))?;
    if status("selftest-group")?.route_count != 2 { return Err("Proxy group did not bind both routes".into()); }
    a.stop.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(80));
    monitor_group_route_once("selftest-group")?;
    let switched = status("selftest-group")?;
    if switched.server_port != Some(b.port) || switched.failover_count == 0 {
        return Err("Proxy group did not fail over to backup route".into());
    }
    let backup_response = runtime.block_on(async {
        client("selftest-group", Duration::from_secs(3))?.get(&url).send().await
            .map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())
    })?;
    if backup_response != "ACCOUNT_B" { return Err("Failover request missed backup proxy".into()); }
    std::env::remove_var("COCKPIT_PROXY_SELFTEST_PROBE_URL");
    save("selftest-a", None)?;
    if endpoint("selftest-a")?.is_some() || profile_proxy(&isolation_dir("selftest-a")?).is_ok() {
        return Err("Unbound isolated profile was allowed to start".into());
    }
    shutdown();
    Ok(
        serde_json::json!({"passed": true, "checks": ["encrypted bindings", "two distinct listeners", "concurrent account routes", "pending auth fails closed", "pending auth proxy locked", "pending auth request uses account A exit", "pending proxy promoted to account binding", "no direct fallback", "inventory group binds two routes", "failed primary switches to backup", "backup route serves the request", "occupied port blocks", "stable restart", "independent empty profiles", "idempotent instance creation", "invalid update preserves binding", "system default client profile preserved", "default API Service client starts without an account proxy", "Reality Vision core accepts config", "bundled sing-box validates and starts HY2 and TUIC listeners", "unbound isolation blocks launch", "child cleanup"], "accountARequests": a.hits.load(Ordering::SeqCst), "accountBRequests": b.hits.load(Ordering::SeqCst), "directRequests": direct.hits.load(Ordering::SeqCst)}),
    )
}
