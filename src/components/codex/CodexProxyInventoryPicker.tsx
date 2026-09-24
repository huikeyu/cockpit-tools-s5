import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { Boxes, Check, Pencil, Plus, Trash2, X } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import type { CodexProxyInventoryItem } from '../../types/codexProxyDashboard';
import './CodexProxyInventoryPicker.css';

interface Props { onSelect: (value: string) => void; }

interface SubscriptionImportResult {
  imported: number;
  updated: number;
  skipped: number;
  alreadyInOtherGroup: number;
  insecureNodes: number;
  format: string;
  groupLabel: string;
}

export function CodexProxyInventoryPicker({ onSelect }: Props) {
  const [items, setItems] = useState<CodexProxyInventoryItem[]>([]);
  const [open, setOpen] = useState(false);
  const [label, setLabel] = useState('');
  const [editingId, setEditingId] = useState<string | null>(null);
  const [group, setGroup] = useState('默认线路组');
  const [uri, setUri] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [subscriptionUrl, setSubscriptionUrl] = useState('');
  const [subscriptionGroup, setSubscriptionGroup] = useState('');
  const [fetchViaId, setFetchViaId] = useState('');
  const [allowDirect, setAllowDirect] = useState(false);
  const [importReport, setImportReport] = useState<SubscriptionImportResult | null>(null);
  const refresh = async () => {
    try { setItems(await invoke<CodexProxyInventoryItem[]>('codex_get_proxy_inventory')); }
    catch (value) { setError(String(value).replace(/^Error:\s*/, '')); }
  };
  useEffect(() => { void refresh(); }, []);
  const save = async () => {
    if (!uri.trim() && !editingId) return;
    setBusy(true); setError('');
    try {
      setItems(await invoke<CodexProxyInventoryItem[]>('codex_save_proxy_inventory_entry', { id: editingId, label, groupLabel: group, proxyUri: uri }));
      setEditingId(null); setLabel(''); setUri('');
    } catch (value) { setError(String(value).replace(/^Error:\s*/, '')); }
    finally { setBusy(false); }
  };
  const remove = async (id: string) => {
    setBusy(true); setError('');
    if (!window.confirm('删除库存线路后，已绑定此组的账号将不再使用该线路。确定删除？')) { setBusy(false); return; }
    try { setItems(await invoke<CodexProxyInventoryItem[]>('codex_delete_proxy_inventory_entry', { id })); }
    catch (value) { setError(String(value).replace(/^Error:\s*/, '')); }
    finally { setBusy(false); }
  };
  const importSubscription = async () => {
    if (!subscriptionUrl.trim() || !subscriptionGroup.trim() || (!fetchViaId && !allowDirect)) return;
    setBusy(true); setError(''); setImportReport(null);
    try {
      const report = await invoke<SubscriptionImportResult>('codex_import_proxy_subscription', {
        subscriptionUrl: subscriptionUrl.trim(),
        groupLabel: subscriptionGroup.trim(),
        fetchViaInventoryId: fetchViaId || null,
        allowDirect,
      });
      setImportReport(report);
      setItems(await invoke<CodexProxyInventoryItem[]>('codex_get_proxy_inventory'));
      setSubscriptionUrl('');
    } catch (value) { setError(String(value).replace(/^Error:\s*/, '')); }
    finally { setBusy(false); }
  };
  return <>
    <button type="button" className="codex-proxy-inventory-trigger" onClick={() => { setOpen(true); void refresh(); }}><Boxes size={14} />从代理库存选择</button>
    {open && createPortal(
      <div className="codex-proxy-inventory-overlay" onClick={(event) => { if (event.target === event.currentTarget) setOpen(false); }}>
        <section className="codex-proxy-inventory-dialog" role="dialog" aria-modal="true" aria-label="代理库存">
          <header><div><span>V4 · 代理库存</span><h2><Boxes size={20} />线路与故障接管组</h2></div><button type="button" onClick={() => setOpen(false)} aria-label="关闭"><X size={18} /></button></header>
          <div className="codex-proxy-inventory-body">
            <p className="codex-proxy-inventory-hint">选择一条线路后，账号会绑定它所属的线路组；主线路失效时，服务会按同组其他线路接管。节点链接只保存在本机加密库存。</p>
            <div className="codex-proxy-inventory-list">
              {items.length === 0 && <p className="codex-proxy-inventory-empty">库存为空，请先添加线路。</p>}
              {items.map((item) => <div className="codex-proxy-inventory-item" key={item.id}>
                <div className="codex-proxy-inventory-item-copy"><strong>{item.label}</strong><span>{item.groupLabel} · {item.protocol.toUpperCase()} · {item.serverHost}:{item.serverPort}{item.insecureTls ? ' · ⚠ TLS 证书验证已关闭' : ''}</span></div>
                <button type="button" className="codex-proxy-inventory-use" onClick={() => { onSelect('inventory://' + item.id); setOpen(false); }}><Check size={14} />选择</button>
                <button type="button" className="codex-proxy-inventory-use" onClick={() => { setEditingId(item.id); setLabel(item.label); setGroup(item.groupLabel); setUri(''); }}><Pencil size={14} />编辑</button>
                <button type="button" className="codex-proxy-inventory-delete" onClick={() => void remove(item.id)} disabled={busy} aria-label={'删除 ' + item.label}><Trash2 size={14} /></button>
              </div>)}
            </div>
            <details className="codex-proxy-inventory-add" open={editingId ? true : undefined}><summary><Plus size={14} />{editingId ? '编辑库存线路' : '添加库存线路'}</summary>
              <label>线路名称<input value={label} onChange={(event) => setLabel(event.target.value)} placeholder="美国主线路" /></label>
              <label>故障接管组<input value={group} onChange={(event) => setGroup(event.target.value)} placeholder="美国线路组" /></label>
              <label>代理链接<input type="password" value={uri} onChange={(event) => setUri(event.target.value)} placeholder={editingId ? '留空表示保持原线路链接' : 'vless://... / http://... / socks5://...'} /></label>
              {uri.includes('insecure=1') && <small className="codex-proxy-inventory-warning">此链接关闭 TLS 证书验证，可能被中间人篡改；仅在你信任节点且无法使用有效证书时使用。</small>}
              <button type="button" className="btn btn-secondary" onClick={() => void save()} disabled={busy || (!editingId && !uri.trim())}>保存到库存</button>
              {editingId && <button type="button" className="btn btn-secondary" onClick={() => { setEditingId(null); setLabel(''); setUri(''); }}>取消编辑</button>}
            </details>
            <details className="codex-proxy-inventory-add codex-proxy-inventory-subscription"><summary><Plus size={14} />从 V2Ray / Clash 订阅导入到分组</summary>
              <label>订阅 HTTPS 链接<input type="password" autoComplete="off" value={subscriptionUrl} onChange={(event) => setSubscriptionUrl(event.target.value)} placeholder="https://订阅服务/订阅令牌" /></label>
              <label>导入分组名称<input value={subscriptionGroup} onChange={(event) => setSubscriptionGroup(event.target.value)} placeholder="例如：香港备用线路" /></label>
              <label>拉取订阅所用网络<select value={fetchViaId} onChange={(event) => setFetchViaId(event.target.value)}>
                <option value="">未选库存代理</option>
                {items.filter((item) => item.enabled).map((item) => <option key={item.id} value={item.id}>{item.label} · {item.groupLabel}</option>)}
              </select></label>
              {!fetchViaId && <label className="codex-proxy-inventory-direct"><input type="checkbox" checked={allowDirect} onChange={(event) => setAllowDirect(event.target.checked)} />明确允许本机网络拉取；订阅服务可能看到原始出口 IP</label>}
              <small>一次性导入，暂不自动刷新。仅保存可解析的 VLESS、Hysteria2、TUIC、HTTP(S)、SOCKS5 线路；其他类型会计数跳过。拉取时不会发送任何 Codex 账号凭据。</small>
              <button type="button" className="btn btn-secondary" onClick={() => void importSubscription()} disabled={busy || !subscriptionUrl.trim() || !subscriptionGroup.trim() || (!fetchViaId && !allowDirect)}>导入订阅</button>
            </details>
            {importReport && <p className="codex-proxy-inventory-result">{importReport.format} · 新增 {importReport.imported} 条，更新 {importReport.updated} 条，格式不支持 {importReport.skipped} 条，已在其他组 {importReport.alreadyInOtherGroup} 条。{importReport.insecureNodes > 0 ? `其中 ${importReport.insecureNodes} 条关闭了 TLS 证书验证，请谨慎使用。` : ''}</p>}
            {error && <p className="codex-proxy-inventory-error">{error}</p>}
          </div>
        </section>
      </div>, document.body)}
  </>;
}
