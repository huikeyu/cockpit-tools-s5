import { useEffect, useId, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { Activity, ArrowRight, CircleAlert, Clock3, Eye, EyeOff, Globe2, Radio, Route, Server, ShieldCheck, X } from 'lucide-react';
import type { CodexAccount } from '../../types/codex';
import type { CodexProxyAccountDashboard, CodexProxyRequestActivity, CodexProxyStatus } from '../../types/codexProxyDashboard';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { CodexAccountProxyButton } from './CodexAccountProxyButton';
import { computeCodexRequestHeat } from '../../utils/codexRequestHeat';
import { useCodexProxyUiStore } from '../../stores/useCodexProxyUiStore';
import './CodexAccountNetworkPanel.css';

interface Props {
  account: CodexAccount;
  inApiService: boolean;
  apiServiceRunning: boolean;
  apiServicePort: number | null;
}

function ago(timestamp: number | null | undefined): string {
  if (!timestamp) return '尚无记录';
  const seconds = Math.max(0, Math.floor((Date.now() - timestamp) / 1000));
  if (seconds < 60) return `${seconds} 秒前`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)} 分钟前`;
  return new Date(timestamp).toLocaleString();
}

function connectionLabel(proxy: CodexProxyStatus): string {
  return [proxy.protocol?.toUpperCase(), proxy.security?.toUpperCase(), proxy.transport?.toUpperCase()]
    .filter(Boolean).join(' · ') || '代理协议待确认';
}

function protocolLabel(proxy: CodexProxyStatus | null | undefined): string {
  if (proxy?.protocol === 'hysteria2' || proxy?.protocol === 'hy2') return 'HY2';
  return proxy?.protocol?.toUpperCase() || '—';
}

function maskEndpoint(host: string): string {
  if (!host || host === '未配置') return host;
  return host.includes(':') ? '••••:••••' : host.includes('.') && /^\d/.test(host) ? '•••.•••.•••.•••' : '••••••';
}

function activityLabel(event: CodexProxyRequestActivity): string {
  if (event.success) return event.httpStatus ? `HTTP ${event.httpStatus}` : '成功';
  return event.httpStatus ? `HTTP ${event.httpStatus}` : event.errorCategory || '请求失败';
}

function tokenLabel(value: number): string {
  if (!value) return '0 tok';
  if (value >= 1000) return (value / 1000).toFixed(value >= 10000 ? 0 : 1) + 'K tok';
  return value + ' tok';
}

function detailedActivity(event: CodexProxyRequestActivity): string {
  const model = event.requestedModel && event.requestedModel !== event.model
    ? event.requestedModel + ' → ' + event.model : event.model;
  const route = [event.gatewayMode, event.serviceTier, event.reasoningEffort].filter(Boolean).join(' · ');
  return (model || '模型请求') + ' · ' + activityLabel(event) + ' · ' + event.latencyMs + ' ms · ' + tokenLabel(event.totalTokens) + (route ? ' · ' + route : '') + ' · ' + ago(event.timestamp);
}

export function CodexAccountNetworkPanel({ account, inApiService, apiServiceRunning, apiServicePort }: Props) {
  const [dashboard, setDashboard] = useState<CodexProxyAccountDashboard | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [loadError, setLoadError] = useState('');
  const [detailsOpen, setDetailsOpen] = useState(false);
  const showEndpoint = useCodexProxyUiStore((state) => state.showRemoteIp);
  const toggleEndpoint = useCodexProxyUiStore((state) => state.toggleRemoteIp);
  const loading = useRef(false);
  const panelRef = useRef<HTMLElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  useModalScrollLock(detailsOpen);
  useEscCloseTopmost(detailsOpen, () => { setDetailsOpen(false); triggerRef.current?.focus(); });

  useEffect(() => {
    let active = true;
    const refresh = async () => {
      if (loading.current) return;
      loading.current = true;
      try {
        const next = await invoke<CodexProxyAccountDashboard>('codex_get_account_proxy_dashboard', { accountId: account.id });
        if (active) { setDashboard(next); setLoadError(''); }
      } catch (error) {
        if (active) setLoadError(String(error).replace(/^Error:\s*/, ''));
      } finally {
        loading.current = false;
      }
    };
    void refresh();
    const timer = window.setInterval(() => { setNow(Date.now()); void refresh(); }, 4000);
    return () => { active = false; window.clearInterval(timer); };
  }, [account.id]);

  const proxy = dashboard?.proxy;
  const latest = dashboard?.recentRequests[0];
  const { level: heatLevel, strength: heatStrength } = computeCodexRequestHeat(dashboard?.recentRequests ?? [], now);
  const recent = Boolean(latest && now - latest.timestamp < 12_000);
  const tone = !proxy?.enabled ? 'unbound'
    : !inApiService ? 'saved'
      : !apiServiceRunning ? 'waiting'
        : proxy.running ? 'running' : 'blocked';
  const toneText = tone === 'running' ? (recent ? '刚有 API 请求' : '代理通道运行中')
    : tone === 'blocked' ? '代理离线 · 请求阻断'
      : tone === 'waiting' ? '等待 API 服务启动'
        : tone === 'saved' ? '已绑定 · 未加入 API 服务' : '未绑定 · 请求阻断';
  const effectiveHeat = tone === 'running' ? heatStrength : 0;
  useEffect(() => {
    const card = panelRef.current?.closest<HTMLElement>('.codex-account-card');
    if (!card) return;
    card.style.setProperty('--codex-heat-low', String((effectiveHeat * .07).toFixed(4)));
    card.style.setProperty('--codex-heat-high', String((effectiveHeat * .19).toFixed(4)));
    return () => {
      card.style.removeProperty('--codex-heat-low');
      card.style.removeProperty('--codex-heat-high');
    };
  }, [effectiveHeat]);
  const endpoint = proxy?.serverHost
    ? `${proxy.serverHost}${proxy.serverPort ? `:${proxy.serverPort}` : ''}`
    : '未配置';
  const routeLabel = proxy?.label || (dashboard ? '尚未绑定线路' : '正在读取线路');
  const remoteHost = proxy?.serverHost || '未配置';
  const endpointDisplay = showEndpoint ? remoteHost : maskEndpoint(remoteHost);
  const hasFlow = inApiService && apiServiceRunning && Boolean(proxy?.running);

  return <>
    <section ref={panelRef} className={`codex-proxy-network-card is-${tone} is-heat-${heatLevel}`} style={{ '--proxy-heat-alpha': 0.035 + effectiveHeat * 0.18 } as CSSProperties} aria-label={`${account.email} 的独立代理网络状态`}>
      <div className="codex-proxy-network-header">
        <span className="codex-proxy-network-icon"><Route size={16} /></span>
        <div className="codex-proxy-network-heading">
          <span title={proxy?.groupLabel || undefined}>独立出口{proxy?.routeCount ? ` · ${proxy.routeCount} 线` : ''}</span>
          <strong title={routeLabel}>{routeLabel}</strong>
        </div>
        <span className={`codex-proxy-network-state is-${tone}`}><i />{toneText}</span>
      </div>
      <div className="codex-proxy-network-facts">
        <div><span>节点协议</span><strong>{protocolLabel(proxy)}</strong></div>
        <div className="codex-proxy-network-fact-endpoint"><span>远端节点 <button type="button" onClick={toggleEndpoint} aria-label={showEndpoint ? '隐藏全部账号远端 IP' : '显示全部账号远端 IP'} title={showEndpoint ? '隐藏全部账号远端 IP' : '显示全部账号远端 IP'}>{showEndpoint ? <EyeOff size={11} /> : <Eye size={11} />}</button></span><strong title={showEndpoint ? remoteHost : undefined}>{endpointDisplay}</strong></div>
      </div>
      <div className="codex-proxy-network-bottom">
        <span className="codex-proxy-network-last" title={latest ? detailedActivity(latest) : undefined}>
          {latest ? <><Activity size={13} /><span className="codex-proxy-request-summary"><b>{activityLabel(latest)} · {latest.latencyMs} ms</b><small>{tokenLabel(latest.totalTokens)} · {ago(latest.timestamp)}</small></span></>
            : <><Clock3 size={13} />{dashboard?.logsAvailable === false ? '请求日志暂不可用' : '暂无该账号的 API 请求'}</>}
        </span>
        <div className="codex-proxy-network-actions"><button ref={triggerRef} type="button" onClick={() => setDetailsOpen(true)}>查看链路</button><CodexAccountProxyButton account={account} compact /></div>
        {latest && <strong className="codex-proxy-model-mark" title={latest.model}>{latest.model || '模型请求'}</strong>}
      </div>
      {loadError && <p className="codex-proxy-network-error"><CircleAlert size={12} />{loadError}</p>}
    </section>
    {detailsOpen && createPortal(
      <div className="codex-proxy-chain-overlay" onClick={(event) => { if (event.target === event.currentTarget) setDetailsOpen(false); }}>
        <section className="codex-proxy-chain-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId}>
          <header>
            <div><span className="codex-proxy-chain-eyebrow"><Radio size={14} /> 每 4 秒刷新 · 真实账号路由日志</span>
              <h2 id={titleId}>{routeLabel}</h2><p>{account.email} · {toneText}</p></div>
            <button type="button" aria-label="关闭链路" onClick={() => { setDetailsOpen(false); triggerRef.current?.focus(); }}><X size={20} /></button>
          </header>
          <div className="codex-proxy-chain-body">
            <div className={`codex-proxy-chain-route ${hasFlow && recent ? 'is-active' : ''}`}>
              <div><Globe2 size={19} /><strong>客户端</strong><small>需指向本机 API</small></div>
              <ArrowRight size={19} />
              <div><Server size={19} /><strong>本机 API</strong><small>{apiServicePort ? `localhost:${apiServicePort}` : 'localhost'}</small></div>
              <ArrowRight size={19} />
              <div><ShieldCheck size={19} /><strong>选中账号</strong><small>{account.email}</small></div>
              <ArrowRight size={19} />
              <div><Route size={19} /><strong>独立代理</strong><small>{proxy?.localPort ? `127.0.0.1:${proxy.localPort}` : '未监听'}</small></div>
              <ArrowRight size={19} />
              <div><Activity size={19} /><strong>上游</strong><small>{endpoint}</small></div>
            </div>
            <div className="codex-proxy-chain-kpis">
              <div><span>节点</span><strong>{proxy?.label || '未绑定'}</strong></div>
              <div><span>协议</span><strong>{proxy ? connectionLabel(proxy) : '—'}</strong></div>
              <div><span>出口 IP（上次测试）</span><strong>{proxy?.lastExitIp || '待检测'}</strong></div>
              <div><span>最近测试</span><strong>{ago(proxy?.lastProbeAt)}</strong></div>
              <div><span>本次运行</span><strong>{proxy?.running ? `${Math.floor((proxy.uptimeSeconds || 0) / 60)} 分钟` : '未运行'}</strong></div>
              <div><span>记录请求</span><strong>{dashboard?.logsAvailable ? dashboard.loggedRequests : '—'}</strong></div>
              <div><span>故障接管组</span><strong>{proxy?.groupLabel || '未使用接管组'}</strong></div>
              <div><span>组内线路</span><strong>{proxy?.routeCount || 0} 条</strong></div>
              <div><span>累计接管</span><strong>{proxy?.failoverCount || 0} 次</strong></div>
            </div>
            <div className="codex-proxy-chain-log-head"><h3>最近完成的请求</h3><span>按真实选中账号过滤</span></div>
            <div className="codex-proxy-chain-events">
              {dashboard?.recentRequests.length ? dashboard.recentRequests.map((event, index) =>
                <div key={`${event.timestamp}-${index}`} className={`codex-proxy-chain-event ${event.success ? 'is-success' : 'is-error'}`}>
                  <span className="codex-proxy-chain-event-dot" />
                  <div><strong>{event.model || '模型请求'}</strong>
                    {event.upstreamModel && event.upstreamModel !== event.model && <small>→ {event.upstreamModel}</small>}
                    <span>{detailedActivity(event)} · {event.requestKind} · {tokenLabel(event.inputTokens)} in / {tokenLabel(event.outputTokens)} out</span>
                    <small>{event.requestId ? 'request ' + event.requestId : ''}{event.apiKeyLabel ? ' · key ' + event.apiKeyLabel : ''}{event.clientInstanceId ? ' · client ' + event.clientInstanceId : ''}{event.estimatedCostUsd ? ' · $' + event.estimatedCostUsd.toFixed(4) : ''}{event.errorMessage ? ' · ' + event.errorMessage : ''}</small></div>
                  <b>{activityLabel(event)}</b>
                </div>)
                : <p className="codex-proxy-chain-empty">还没有该账号的 API 请求。启动 API 服务并发送一次模型请求后，这里会显示实际路由结果。</p>}
            </div>
            <p className="codex-proxy-chain-evidence">链路显示本地 API 服务的账号选择、代理进程和请求结果。出口 IP 来自单独的代理检测；节点供应商可能动态更换出口。</p>
          </div>
        </section>
      </div>, document.body)}
  </>;
}
