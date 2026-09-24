// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--proxy-self-test") {
        let root = std::env::temp_dir().join(format!("cockpit-proxy-selftest-{}", uuid::Uuid::new_v4()));
        std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &root);
        let result = antigravity_cockpit_tools_lib::run_proxy_self_test();
        let success = result.is_ok();
        let report = match result { Ok(value) => value, Err(error) => serde_json::json!({"passed": false, "error": error}) };
        if let Some(report_path) = args.get(2) {
            if std::fs::write(report_path, serde_json::to_vec_pretty(&report).unwrap()).is_err() { std::process::exit(2); }
        }
        std::process::exit(if success { 0 } else { 1 });
    }
    // The custom build never imports or mutates the upstream application's account store.
    if std::env::var_os("COCKPIT_TOOLS_DATA_DIR").is_none() {
        if let Some(root) = dirs::data_local_dir() {
            std::env::set_var("COCKPIT_TOOLS_DATA_DIR", root.join("CockpitTools-Isolation").join("V2"));
        }
    }
    antigravity_cockpit_tools_lib::run()
}
