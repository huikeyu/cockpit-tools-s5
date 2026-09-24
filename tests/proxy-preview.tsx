// Visual QA only. Reserved documentation IPs and synthetic accounts; no real requests.
import React from 'react';
import { createRoot } from 'react-dom/client';
import { CodexAccountNetworkPanel } from '../src/components/codex/CodexAccountNetworkPanel';
import { CodexApiServiceProxySummary } from '../src/components/codex/CodexApiServiceProxySummary';
import { useCodexAccountStore } from '../src/stores/useCodexAccountStore';
import type { CodexAccount } from '../src/types/codex';
import type { CodexProxyAccountDashboard, CodexProxyStatus } from '../src/types/codexProxyDashboard';

const accounts = [
  { id: 'account-a', email: 'alpha@example.invalid', account_name: '账号 A' },
  { id: 'account-b', email: 'bravo@example.invalid', account_name: '账号 B' },
  { id: 'account-c', email: 'charlie@example.invalid', account_name: '账号 C' },
  { id: 'account-d', email: 'delta@example.invalid', account_name: '账号 D' },
  { id: 'account-e', email: 'echo@example.invalid', account_name: '账号 E' },
] as CodexAccount[];
const now = Date.now();
const statuses: Record<string, CodexProxyStatus> = {
  'account-a': { enabled: true, label: '样例节点 · 区域甲 · 多线路低延迟优化 · 固定出口测试', protocol: 'vless', serverHost: '192.0.2.20', serverPort: 8443, security: 'reality', transport: 'tcp', localPort: 31801, running: true, uptimeSeconds: 587, lastExitIp: '192.0.2.10', lastProbeAt: now - 60_000, lastProbeLatencyMs: 148, required: true, isolationDir: 'UI-FIXTURE/a', groupId: 'group-a', groupLabel: '区域甲接管组', routeCount: 2, activeRouteIndex: 0, failoverCount: 0 },
  'account-b': { enabled: true, label: '东京直连 · Tokyo B', protocol: 'vless', serverHost: '198.51.100.20', serverPort: 443, security: 'tls', transport: 'ws', localPort: 31802, running: true, uptimeSeconds: 587, lastExitIp: '198.51.100.10', lastProbeAt: now - 480_000, lastProbeLatencyMs: 97, required: true, isolationDir: 'UI-FIXTURE/b', groupId: 'group-b', groupLabel: '东京接管组', routeCount: 3, activeRouteIndex: 0, failoverCount: 0 },
  'account-c': { enabled: true, label: '欧洲高可用 · EU C', protocol: 'socks5', serverHost: '203.0.113.20', serverPort: 1080, security: null, transport: null, localPort: 31803, running: false, uptimeSeconds: null, lastExitIp: '203.0.113.10', lastProbeAt: now - 1_200_000, lastProbeLatencyMs: 256, required: true, isolationDir: 'UI-FIXTURE/c', groupId: 'group-c', groupLabel: '欧洲接管组', routeCount: 2, activeRouteIndex: 1, failoverCount: 1 },
};
statuses['account-d'] = { ...statuses['account-b'], label: '备用节点 · 区域丁', serverHost: '192.0.2.40', lastExitIp: '192.0.2.41', groupLabel: '区域丁接管组' };
statuses['account-e'] = { ...statuses['account-c'], label: '备用节点 · 区域戊', serverHost: '198.51.100.40', lastExitIp: '198.51.100.41', groupLabel: '区域戊接管组' };
const dashboards: Record<string, CodexProxyAccountDashboard> = Object.fromEntries(accounts.map((account) => [account.id, {
  proxy: statuses[account.id], loggedRequests: account.id === 'account-a' ? 128 : account.id === 'account-b' ? 44 : 0,
  recentRequests: account.id === 'account-a' ? [
    { timestamp: now - 4_000, requestId: 'req-a-001', model: 'gpt-6-sol', requestedModel: 'gpt-6-sol', upstreamModel: 'gpt-6-sol', requestKind: 'text', gatewayMode: 'oauth', serviceTier: 'default', reasoningEffort: 'high', success: true, httpStatus: 200, latencyMs: 1360, errorCategory: '', errorMessage: '', inputTokens: 820, outputTokens: 2140, totalTokens: 2960, estimatedCostUsd: 0.04, apiKeyLabel: '主 API', clientInstanceId: 'default' },
    { timestamp: now - 120_000, requestId: 'req-a-002', model: 'gpt-5.6-sol', requestedModel: 'gpt-5.6-sol', upstreamModel: 'gpt-5.6-sol', requestKind: 'text', gatewayMode: 'oauth', serviceTier: 'default', reasoningEffort: 'medium', success: true, httpStatus: 200, latencyMs: 984, errorCategory: '', errorMessage: '', inputTokens: 420, outputTokens: 1120, totalTokens: 1540, estimatedCostUsd: 0.02, apiKeyLabel: '主 API', clientInstanceId: 'default' },
  ] : account.id === 'account-b' ? [
    { timestamp: now - 55_000, requestId: 'req-b-001', model: 'gpt-6-luna', requestedModel: 'gpt-6-luna', upstreamModel: 'gpt-6-luna', requestKind: 'text', gatewayMode: 'oauth', serviceTier: 'priority', reasoningEffort: 'low', success: false, httpStatus: 429, latencyMs: 742, errorCategory: 'rate_limited', errorMessage: 'upstream rate limited', inputTokens: 210, outputTokens: 0, totalTokens: 210, estimatedCostUsd: 0, apiKeyLabel: '备用 API', clientInstanceId: 'default' },
  ] : [], logsAvailable: true,
}]));

(window as unknown as { __TAURI_INTERNALS__: object }).__TAURI_INTERNALS__ = {
  invoke: async (command: string, args: { accountId: string; proxyUri?: string }) => {
    const id = args.accountId;
    if (command === 'codex_get_account_proxy') return statuses[id];
    if (command === 'codex_get_account_proxy_dashboard') return dashboards[id];
    if (command === 'codex_get_proxy_inventory') return [];
    if (command === 'codex_local_access_query_request_logs') return { events: [] };
    if (command === 'codex_save_proxy_inventory_entry') return [];
    if (command === 'codex_delete_proxy_inventory_entry') return [];
    if (command === 'codex_import_proxy_subscription') return { imported: 0, updated: 0, skipped: 0, alreadyInOtherGroup: 0, insecureNodes: 0, format: 'fixture', groupLabel: 'fixture' };
    if (command === 'codex_save_account_proxy') return statuses[id];
    if (command === 'codex_test_account_proxy') return { exitIp: statuses[id].lastExitIp, latencyMs: 123 };
    if (command === 'codex_prepare_isolated_instance') return 'ui-fixture-instance';
    if (command === 'codex_start_instance') return {};
    throw `Unsupported UI fixture operation: ${command}`;
  },
};
useCodexAccountStore.setState({ accounts });
createRoot(document.getElementById('root')!).render(<React.StrictMode>
  <main style={{ padding: 28, minHeight: '100vh', boxSizing: 'border-box', fontFamily: 'system-ui', color: '#172033', background: '#f2f6fb' }}>
    <h1 style={{ margin: '0 0 4px', fontSize: 24 }}>Codex 账号多出口 · V4 视觉验收</h1>
    <p style={{ margin: '0 0 20px', color: '#64748b', fontSize: 12 }}>此页仅使用模拟请求和保留 IP，不连接真实账号或网络节点。</p>
    <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill,minmax(min(100%,270px),1fr))', gap: 12, alignItems: 'stretch' }}>
      <div style={{ padding: 17, minWidth: 0, borderRadius: 16, background: '#fff', boxShadow: '0 8px 28px #24344612' }}>
        <h2 style={{ fontSize: 16, margin: '0 0 90px' }}>API 服务 · 账号池</h2>
        <CodexApiServiceProxySummary accountIds={accounts.map((account) => account.id)} running port={32785} />
      </div>
      {accounts.map((account) => <div key={account.id} style={{ padding: 17, minWidth: 0, borderRadius: 16, background: '#fff', boxShadow: '0 8px 28px #24344612' }}>
        <h2 style={{ fontSize: 16, margin: '0 0 150px', overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' }}>{account.email}</h2>
        <CodexAccountNetworkPanel account={account} inApiService apiServiceRunning apiServicePort={32785} />
      </div>)}
    </div>
  </main>
</React.StrictMode>);
