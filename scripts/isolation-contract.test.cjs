const { readFileSync } = require('node:fs');
const assert = require('node:assert/strict');
const { test } = require('node:test');
const path = require('node:path');
const read = (file) => readFileSync(path.join(__dirname, '..', file), 'utf8');
test('quota/profile/token refresh call the account-scoped network factory', () => {
  for (const file of ['codex_quota.rs', 'codex_account_check.rs']) {
    const source = read(`src-tauri/src/modules/${file}`);
    assert.ok(source.includes('account_proxy::client(&account.id'));
    assert.ok(!/reqwest::Client::(new|builder)/.test(source));
  }
  assert.match(read('src-tauri/src/modules/codex_account_runtime_switch.rs'), /refresh_access_token_for_account\(\s*&account.id/);
});
test('gateway configuration and live auth sync preserve account precedence', () => {
  for (const file of ['codex_local_access_sidecar_config.rs', 'codex_local_access_provider_gateway.rs', 'codex_local_access_http.rs']) {
    assert.ok(read(`src-tauri/src/modules/${file}`).includes('account_proxy::effective_proxy(&account.id'));
  }
  assert.ok(read('sidecars/cockpit-cliproxy/provider_gateway.go').includes('providerGatewayHTTPClient(gateway.ProxyURL)'));
});
test('isolated profiles skip shared data and history synchronization', () => {
  for (const file of ['codex_instance.rs', 'codex_thread_sync.rs']) assert.ok(read(`src-tauri/src/modules/${file}`).includes('account_proxy::is_isolated_profile'));
  assert.ok(read('src-tauri/src/modules/process_close_lifecycle.rs').includes('--proxy-server={proxy}'));
  assert.ok(read('src-tauri/src/commands/codex_instance.rs').includes('account_proxy::proxy_env'));
});
test('proxy entry is available in card and table views and saves no browser-side secrets', () => {
  const renderers = read('src/pages/useCodexAccountsRenderers.tsx');
  assert.ok(renderers.includes('<CodexAccountNetworkPanel'));
  assert.ok(renderers.includes('<CodexApiServiceProxySummary'));
  assert.ok(renderers.includes('<CodexAccountProxyButton account={account} />'));
  const source = read('src/components/codex/CodexAccountProxyButton.tsx');
  assert.ok(!source.includes('localStorage'));
  assert.ok(source.includes('createPortal'));
  assert.ok(source.includes("setDraft('')"));
});
test('account chain uses exact selected-account logs and labels probe IP separately', () => {
  const backend = read('src-tauri/src/commands/account_proxy.rs');
  assert.ok(backend.includes('.filter(|event| event.account_id == account_id)'));
  const panel = read('src/components/codex/CodexAccountNetworkPanel.tsx');
  assert.ok(panel.includes('出口 IP（上次测试）'));
  assert.ok(panel.includes('真实账号路由日志'));
});
test('custom installation cannot overwrite upstream installation', () => {
  const config = JSON.parse(read('src-tauri/tauri.conf.json'));
  assert.notEqual(config.identifier, 'com.jlcodes.cockpit-tools');
  assert.equal(config.bundle.createUpdaterArtifacts, false);
  assert.equal(config.bundle.windows.nsis.installerHooks, undefined);
  assert.ok(config.bundle.resources['proxy-core/xray.exe']);
  assert.ok(!read('src-tauri/src/lib.rs').includes('.plugin(tauri_plugin_updater::Builder::new().build())'));
  assert.ok(!read('src-tauri/src/modules/codex_account_storage_locks.rs').includes('migrate_codex_data_if_needed(&data_dir);'));
});
test('default desktop profile uses the system Codex home, not an isolated app profile', () => {
  const account = read('src-tauri/src/modules/codex_account_model_catalog.rs');
  assert.match(account, /pub fn get_codex_home\(\) -> PathBuf \{[\s\S]*resolve_codex_home_from_env\(\)[\s\S]*join\("\.codex"\)/);
  assert.ok(!account.includes('join("default-codex")'));
  const runtime = read('src-tauri/src/modules/process_codex_runtime.rs');
  assert.ok(runtime.includes('launch_codex_via_store_app_user_model_id'));
  assert.ok(runtime.includes('start_codex_with_args(&default_home.to_string_lossy(), extra_args)'));
});
test('adding an account fails closed until a pending proxy is supplied', () => {
  const oauth = read('src-tauri/src/modules/codex_oauth.rs');
  assert.ok(oauth.includes('pending_client(login_id'));
  assert.ok(oauth.includes('additional_browser_args'));
  assert.ok(read('src-tauri/src/modules/codex_account_import.rs').includes('refresh_access_token_with_pending_proxy'));
  assert.ok(read('src/pages/CodexAddAccountDialog.tsx').includes('添加账号代理（必填）'));
  assert.ok(read('src-tauri/src/modules/account_proxy.rs').includes('添加账号前必须先设置独立代理'));
});
test('all account cards share one remote IP switch and show inventory line names', () => {
  const panel = read('src/components/codex/CodexAccountNetworkPanel.tsx');
  assert.ok(panel.includes('useCodexProxyUiStore((state) => state.showRemoteIp)'));
  assert.ok(panel.includes('useCodexProxyUiStore((state) => state.toggleRemoteIp)'));
  assert.ok(panel.includes('proxy?.label ||'));
  assert.ok(panel.includes('showEndpoint ? remoteHost : maskEndpoint(remoteHost)'));
  const backend = read('src-tauri/src/modules/account_proxy.rs');
  assert.ok(backend.includes('label: inventory_label.or_else'));
});
test('HY2/TUIC use bundled sing-box and subscription import requires an explicit network choice', () => {
  const config = JSON.parse(read('src-tauri/tauri.conf.json'));
  assert.ok(config.bundle.resources['proxy-core/sing-box.exe']);
  assert.ok(config.bundle.resources['proxy-core/sing-box-1.14.1-source.zip']);
  const backend = read('src-tauri/src/modules/account_proxy.rs');
  assert.ok(backend.includes('ProxyCore::SingBox'));
  assert.ok(backend.includes('if !allow_direct'));
  assert.ok(backend.includes('parse_subscription(&text)'));
});
