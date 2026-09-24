import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowRight, Network } from 'lucide-react';
import { useCodexAccountStore } from '../../stores/useCodexAccountStore';
import type { CodexProxyStatus } from '../../types/codexProxyDashboard';
import './CodexApiServiceProxySummary.css';

interface Props {
  accountIds: string[];
  running: boolean;
  port: number | null;
}

export function CodexApiServiceProxySummary({ accountIds, running }: Props) {
  const accounts = useCodexAccountStore((state) => state.accounts);
  const [statuses, setStatuses] = useState<Record<string, CodexProxyStatus>>({});
  const [lastRequestAt, setLastRequestAt] = useState<Record<string, number>>({});
  const [loadError, setLoadError] = useState(false);
  const uniqueIds = useMemo(() => Array.from(new Set(accountIds.filter(Boolean))), [accountIds.join('|')]);
  const membershipKey = uniqueIds.join('|');

  useEffect(() => {
    let active = true;
    const refresh = async () => {
      if (uniqueIds.length === 0) { if (active) setStatuses({}); return; }
      const result = await Promise.allSettled(uniqueIds.map((accountId) =>
        invoke<CodexProxyStatus>('codex_get_account_proxy', { accountId })));
      const activityResult = await invoke<{ events: Array<{ accountId: string; timestamp: number }> }>(
        'codex_local_access_query_request_logs', { page: 1, pageSize: 100 }
      ).catch(() => ({ events: [] }));
      if (!active) return;
      const next: Record<string, CodexProxyStatus> = {};
      result.forEach((item, index) => { if (item.status === 'fulfilled') next[uniqueIds[index]] = item.value; });
      setStatuses(next);
      const activity: Record<string, number> = {};
      activityResult.events.forEach((event) => {
        if (uniqueIds.includes(event.accountId)) activity[event.accountId] = Math.max(activity[event.accountId] || 0, event.timestamp);
      });
      setLastRequestAt(activity);
      setLoadError(result.some((item) => item.status === 'rejected'));
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), 6000);
    return () => { active = false; window.clearInterval(timer); };
  }, [membershipKey]);

  const orderedIds = [...uniqueIds].sort((a, b) =>
    (lastRequestAt[b] || 0) - (lastRequestAt[a] || 0) ||
    Number(Boolean(statuses[b]?.running)) - Number(Boolean(statuses[a]?.running)));
  const visibleIds = orderedIds.slice(0, 3);
  const bound = uniqueIds.filter((id) => statuses[id]?.enabled).length;
  return <section className="codex-api-pool-proxy-summary" aria-label="API 服务账号池独立代理">
    <div className="codex-api-pool-proxy-header">
      <span className="codex-api-pool-proxy-icon"><Network size={16} /></span>
      <div><strong>账号独立网络</strong><small>API 服务按实际账号选择其专属代理</small></div>
      <span className={`codex-api-pool-proxy-count ${bound === uniqueIds.length && uniqueIds.length > 0 ? 'is-complete' : 'has-gap'}`}>{bound}/{uniqueIds.length} 已绑定</span>
    </div>
    <div className="codex-api-pool-proxy-members">
      {uniqueIds.length === 0 && <span className="codex-api-pool-proxy-empty">账号池为空；添加账号后可在各账号卡片绑定与测试节点。</span>}
      {visibleIds.map((id) => {
        const status = statuses[id];
        const name = accounts.find((account) => account.id === id)?.account_name || accounts.find((account) => account.id === id)?.email || id;
        const state = !status?.enabled ? 'missing' : running && status.running ? 'online' : 'saved';
        return <div className={`codex-api-pool-proxy-member is-${state}`} key={id} title={`${name} · ${status?.groupLabel || status?.label || '未绑定代理'}`}>
          <i /><span className="codex-api-pool-proxy-account">{name}</span>
          <ArrowRight size={12} /><strong>{status?.groupLabel || status?.label || '未绑定代理'}</strong>
          <small>{state === 'online' ? '运行中' : state === 'saved' ? '已保存' : '阻断'}</small>
        </div>;
      })}
    </div>
    {loadError && <p className="codex-api-pool-proxy-warning">部分账号代理状态暂时无法读取，请打开对应账号卡片检查。</p>}
  </section>;
}
