import { useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { Eye, EyeOff, Globe, Loader2, ShieldCheck, X } from 'lucide-react';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { startInstance } from '../../services/codexInstanceService';
import type { CodexAccount } from '../../types/codex';
import type { CodexProxyStatus } from '../../types/codexProxyDashboard';
import { CodexProxyInventoryPicker } from './CodexProxyInventoryPicker';
import './CodexAccountProxyButton.css';

type Probe = { exitIp: string; latencyMs: number };

export function CodexAccountProxyButton({ account, compact = false }: { account: CodexAccount; compact?: boolean }) {
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<CodexProxyStatus | null>(null);
  const [draft, setDraft] = useState('');
  const [visible, setVisible] = useState(false);
  const [busy, setBusy] = useState('');
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const [probe, setProbe] = useState<Probe | null>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const field = useRef<HTMLInputElement>(null);
  const titleId = useId();
  const inputId = useId();
  useModalScrollLock(open);
  const close = () => { if (!busy) { setOpen(false); setDraft(''); setVisible(false); trigger.current?.focus(); } };
  useEscCloseTopmost(open && !busy, close);

  useEffect(() => {
    let live = true;
    const refresh = () => void invoke<CodexProxyStatus>('codex_get_account_proxy', { accountId: account.id })
      .then(value => { if (live) { setStatus(value); setError(''); } })
      .catch(e => { if (live) setError(String(e)); });
    refresh();
    const timer = window.setInterval(refresh, 10000);
    return () => { live = false; window.clearInterval(timer); };
  }, [account.id, open]);

  useEffect(() => { if (open) field.current?.focus(); }, [open]);

  const run = async (label: string, task: () => Promise<void>) => {
    if (busy) return;
    setBusy(label); setError(''); setMessage('');
    try { await task(); } catch (e) { setError(String(e)); } finally { setBusy(''); }
  };
  const save = () => run('保存代理', async () => {
    const value = await invoke<CodexProxyStatus>('codex_save_account_proxy', { accountId: account.id, proxyUri: draft });
    setStatus(value); setDraft(''); setVisible(false); setProbe(null);
    setMessage('节点已加密保存。启动 API 服务后，此账号的上游请求将使用这个节点。');
  });
  const test = () => run('测试出口', async () => {
    setProbe(null);
    setProbe(await invoke<Probe>('codex_test_account_proxy', { accountId: account.id }));
    setStatus(await invoke<CodexProxyStatus>('codex_get_account_proxy', { accountId: account.id }));
  });
  const launch = () => run('隔离启动', async () => {
    const instanceId = await invoke<string>('codex_prepare_isolated_instance', { accountId: account.id });
    await startInstance(instanceId);
    setMessage('隔离实例已启动。保持本工具在后台运行；退出本工具会断开独立代理。可在“Codex 多开”管理该实例。');
  });

  return <>
    <button ref={trigger} type="button" className={`account-proxy-trigger ${status?.enabled ? 'bound' : ''} ${status?.running ? 'running' : ''}`} title={error || (status?.enabled ? `独立代理：${status.groupLabel || status.label || '已绑定线路'}` : '为此账号绑定独立代理')} onClick={e => { e.stopPropagation(); setOpen(true); setMessage(''); }}>
      <ShieldCheck size={14} /><span>{compact ? (status?.enabled ? '代理设置' : '绑定代理') : status?.enabled ? status.label || '已绑定节点' : '绑定独立代理'}</span>
    </button>
    {open && createPortal(<div className="account-proxy-overlay" onClick={e => { e.stopPropagation(); if (e.target === e.currentTarget) close(); }}>
      <section className="account-proxy-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} onKeyDown={e => {
        if (e.key !== 'Tab') return;
        const controls = Array.from(e.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), [tabindex="0"]'));
        const first = controls[0], last = controls[controls.length - 1];
        if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last?.focus(); }
        else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first?.focus(); }
      }}>
        <header><div><span className="account-proxy-eyebrow">CODEX · 账号隔离 / V4</span><h2 id={titleId}><ShieldCheck size={23} />独立代理</h2></div><button type="button" aria-label="关闭" onClick={close} disabled={!!busy}><X size={20} /></button></header>
        <div className="account-proxy-body">
          <div className="account-proxy-identity">账号：{account.email || account.id}</div>
          <div className={`account-proxy-status ${status?.enabled ? 'bound' : ''}`}><Globe size={18} /><div><strong>{status?.enabled ? status.label : '尚未绑定独立代理'}</strong><small>{status?.enabled ? `${status.protocol?.toUpperCase()} · ${status.security?.toUpperCase() || '标准'} · ${status.serverHost || '节点'}:${status.serverPort || '-'} · 本机端口 ${status.localPort} · ${status.running ? '通道运行中' : '等待 API 服务'} ` : '未绑定时 API 请求会被阻断。'}</small></div></div>
          {status?.enabled && <div className="account-proxy-evidence"><span>出口 IP（上次检测）<strong>{status.lastExitIp || '待检测'}</strong></span><span>代理状态<strong>{status.running ? '进程运行中' : '尚未启动'}</strong></span></div>}
          <label htmlFor={inputId}>{status?.enabled ? '替换代理链接' : '粘贴代理链接'}</label>
          <div className="account-proxy-input"><input ref={field} id={inputId} type={visible ? 'text' : 'password'} value={draft} disabled={!!busy} autoComplete="off" spellCheck={false} placeholder="vless://UUID@服务器:端口?... 或 http(s)/socks5://..." onChange={e => { setDraft(e.target.value); setProbe(null); }} /><button type="button" aria-label={visible ? '隐藏链接' : '显示链接'} onClick={() => setVisible(!visible)}>{visible ? <EyeOff size={17} /> : <Eye size={17} />}</button></div>
          <CodexProxyInventoryPicker onSelect={(value) => { setDraft(value); setProbe(null); }} />
          {status?.groupLabel && <p className="account-proxy-hint">当前接管组：{status.groupLabel} · {status.routeCount} 条线路 · 已接管 {status.failoverCount} 次</p>}
          <p className="account-proxy-hint">支持 VLESS TCP/WS、Reality/TLS、Vision、Hysteria2、TUIC，以及 HTTP / HTTPS / SOCKS5。带 insecure=1 的节点会关闭 TLS 证书验证，请谨慎使用。保存后不回显节点密码。</p>
          <div className="account-proxy-scope"><strong>这张卡对应账号的上游出口</strong><p>API 服务按实际选中的账号使用各自节点；总服务卡不需要选一个统一代理。客户端访问本机 localhost，模型请求再由账号代理转发。</p><p>新增或重新授权账号时，请在“添加账号”窗口先填代理并测试出口。修改此绑定前请停止 API 服务。</p></div>
          {status && <details><summary>账号专用目录</summary><code>{status.isolationDir}</code></details>}
          <p className="account-proxy-hint">测试出口会通过该代理访问 api.ipify.org，仅查询公网 IP，不发送账号令牌。不同节点也可能共用同一出口 IP，请分别核对。</p>
          {probe && <div className="account-proxy-success" role="status">出口 IP：<strong>{probe.exitIp}</strong> · {probe.latencyMs} ms</div>}
          {message && <div className="account-proxy-success" role="status">{message}</div>}
          {error && <div className="account-proxy-error" role="alert">{error}</div>}
        </div>
        <footer>{busy && <span className="account-proxy-busy"><Loader2 size={16} className="loading-spinner" />{busy}…</span>}
          <button type="button" disabled={!!busy || !draft.trim()} onClick={() => void save()}>保存代理</button>
          <button type="button" disabled={!!busy || !status?.enabled || !!draft.trim()} onClick={() => void test()}>测试出口</button>
          <button type="button" disabled={!!busy || !status?.enabled || !!draft.trim()} onClick={() => void launch()}>可选：隔离启动</button>
        </footer>
      </section>
    </div>, document.body)}
  </>;
}
