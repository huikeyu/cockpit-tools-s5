import { useEffect, useMemo, useRef, useState, type DragEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowRightLeft, Boxes, GripVertical, Plus, RefreshCw, Search, ShieldCheck, Trash2 } from 'lucide-react';
import type { CodexAccount } from '../../types/codex';
import type { CodexProxyGroupBinding, CodexProxyInventoryItem } from '../../types/codexProxyDashboard';
import './CodexProxyManager.css';

type Group = { id: string; label: string; routes: CodexProxyInventoryItem[]; bound: number };
type ImportReport = { imported: number; updated: number; skipped: number; alreadyInOtherGroup: number };
const ROW_HEIGHT = 58;
const VIEWPORT_HEIGHT = 464;

export function CodexProxyManager({ accounts, privacyMode = false }: { accounts: CodexAccount[]; privacyMode?: boolean }) {
  const [items, setItems] = useState<CodexProxyInventoryItem[]>([]);
  const [bindings, setBindings] = useState<CodexProxyGroupBinding[]>([]);
  const [selectedGroup, setSelectedGroup] = useState('');
  const [query, setQuery] = useState('');
  const [scrollTop, setScrollTop] = useState(0);
  const [busy, setBusy] = useState('');
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [dragOver, setDragOver] = useState('');
  const [editingId, setEditingId] = useState<string | null>(null);
  const [label, setLabel] = useState('');
  const [groupName, setGroupName] = useState('');
  const [uri, setUri] = useState('');
  const [newGroup, setNewGroup] = useState('');
  const [subscriptionUrl, setSubscriptionUrl] = useState('');
  const [subscriptionGroup, setSubscriptionGroup] = useState('');
  const [fetchViaId, setFetchViaId] = useState('');
  const [allowDirect, setAllowDirect] = useState(false);
  const [importReport, setImportReport] = useState<ImportReport | null>(null);
  const routeScroller = useRef<HTMLDivElement>(null);

  const refresh = async () => {
    const [inventory, accountBindings] = await Promise.all([
      invoke<CodexProxyInventoryItem[]>('codex_get_proxy_inventory'),
      invoke<CodexProxyGroupBinding[]>('codex_get_proxy_group_bindings', { accountIds: accounts.map(account => account.id) }),
    ]);
    setItems(inventory);
    setBindings(accountBindings);
  };
  useEffect(() => { void refresh().catch(value => setError(String(value))); }, [accounts.map(a => a.id).join('|')]);

  const groups = useMemo(() => {
    const byId = new Map<string, Group>();
    for (const item of items) {
      let group = byId.get(item.groupId);
      if (!group) { group = { id: item.groupId, label: item.groupLabel, routes: [], bound: 0 }; byId.set(item.groupId, group); }
      group.routes.push(item);
    }
    for (const binding of bindings) {
      if (!binding.groupId) continue;
      let group = byId.get(binding.groupId);
      if (!group) { group = { id: binding.groupId, label: binding.groupLabel || '已失效线路组', routes: [], bound: 0 }; byId.set(binding.groupId, group); }
      group.bound += 1;
    }
    return [...byId.values()].sort((a, b) => b.bound - a.bound || a.label.localeCompare(b.label, 'zh-CN'));
  }, [items, bindings]);
  useEffect(() => {
    if (!groups.some(group => group.id === selectedGroup)) setSelectedGroup(groups[0]?.id || '');
  }, [groups, selectedGroup]);
  const group = groups.find(value => value.id === selectedGroup);
  const routes = useMemo(() => (group?.routes || []).filter(item => {
    const term = query.trim().toLocaleLowerCase();
    return !term || `${item.label} ${item.protocol} ${item.serverHost}`.toLocaleLowerCase().includes(term);
  }), [group, query]);
  const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - 3);
  const end = Math.min(routes.length, start + Math.ceil(VIEWPORT_HEIGHT / ROW_HEIGHT) + 7);
  const shown = routes.slice(start, end);
  const accountById = useMemo(() => new Map(accounts.map(account => [account.id, account])), [accounts]);
  const sortedBindings = useMemo(() => [...bindings].sort((a, b) => Number(b.groupId === selectedGroup) - Number(a.groupId === selectedGroup)), [bindings, selectedGroup]);

  const run = async (name: string, work: () => Promise<void>) => {
    if (busy) return;
    setBusy(name); setError(''); setNotice('');
    try { await work(); } catch (value) { setError(String(value).replace(/^Error:\s*/, '')); }
    finally { setBusy(''); }
  };
  const chooseGroup = (id: string) => { setSelectedGroup(id); setScrollTop(0); routeScroller.current?.scrollTo(0, 0); };
  const moveRoute = (id: string, target: Group | string) => run('移动线路', async () => {
    const name = typeof target === 'string' ? target.trim() : target.label;
    if (!name) throw new Error('请先输入目标组名称');
    setItems(await invoke<CodexProxyInventoryItem[]>('codex_move_proxy_inventory_entry', { id, groupLabel: name }));
    setNotice('线路已移动。受影响账号的本地通道正在按新分组恢复。');
    setNewGroup('');
    void refresh().catch(value => setError(String(value)));
  });
  const bindAccount = (id: string, target: Group) => run('切换账号', async () => {
    const first = target.routes.find(route => route.enabled);
    if (!first) throw new Error('目标组没有可用线路，不能绑定');
    await invoke('codex_save_account_proxy', { accountId: id, proxyUri: `inventory://${first.id}` });
    setBindings(await invoke<CodexProxyGroupBinding[]>('codex_get_proxy_group_bindings', { accountIds: accounts.map(account => account.id) }));
    setNotice('账号已热切换到新分组，API 服务不需重启。切换瞬间的请求可能需要重试。');
  });
  const onDropGroup = (event: DragEvent, target: Group) => {
    event.preventDefault(); setDragOver('');
    const value = event.dataTransfer.getData('text/plain');
    if (value.startsWith('route:')) void moveRoute(value.slice(6), target);
    if (value.startsWith('account:')) void bindAccount(value.slice(8), target);
  };
  const resetEditor = () => { setEditingId(null); setLabel(''); setUri(''); setGroupName(group?.label || '默认线路组'); };
  const edit = (item: CodexProxyInventoryItem) => { setEditingId(item.id); setLabel(item.label); setGroupName(item.groupLabel); setUri(''); };
  const saveRoute = () => run('保存线路', async () => {
    if (!editingId && !uri.trim()) throw new Error('请填写代理链接');
    const next = await invoke<CodexProxyInventoryItem[]>('codex_save_proxy_inventory_entry', {
      id: editingId, label, groupLabel: groupName || group?.label || '默认线路组', proxyUri: uri,
    });
    setItems(next); resetEditor(); setNotice('线路已保存；名称编辑不会重启账号代理。');
  });
  const removeRoute = (item: CodexProxyInventoryItem) => {
    if (!window.confirm(`删除线路“${item.label}”？绑定该组的账号可能立即切换到备用线路；空组会阻断请求。`)) return;
    void run('删除线路', async () => {
      setItems(await invoke<CodexProxyInventoryItem[]>('codex_delete_proxy_inventory_entry', { id: item.id }));
      setNotice('线路已删除。空组仍可在这里查看并改绑账号。');
    });
  };
  const importSubscription = () => run('导入订阅', async () => {
    setImportReport(await invoke<ImportReport>('codex_import_proxy_subscription', {
      subscriptionUrl: subscriptionUrl.trim(), groupLabel: subscriptionGroup.trim(),
      fetchViaInventoryId: fetchViaId || null, allowDirect,
    }));
    setSubscriptionUrl('');
    setItems(await invoke<CodexProxyInventoryItem[]>('codex_get_proxy_inventory'));
  });

  return <section className="codex-proxy-manager" aria-label="代理管理">
    <header className="codex-proxy-manager-head">
      <div><span>ACCOUNT NETWORK CONTROL</span><h2><Boxes size={22} />代理管理</h2><p>拖动线路或账号到左侧分组；运行中的 API 服务可直接换组。任何不可用线路都会阻断直连。</p></div>
      <button type="button" className="btn btn-secondary" disabled={!!busy} onClick={() => void run('刷新', refresh)}><RefreshCw size={15} />刷新</button>
    </header>
    {error && <div className="codex-proxy-manager-alert is-error" role="alert">{error}</div>}
    {notice && <div className="codex-proxy-manager-alert" role="status">{notice}</div>}
    <div className="codex-proxy-manager-columns">
      <aside className="codex-proxy-manager-groups">
        <div className="codex-proxy-manager-section-title">线路组 <b>{groups.length}</b></div>
        <div className="codex-proxy-manager-group-list">
          {groups.map(value => <button key={value.id} type="button" className={`codex-proxy-manager-group${value.id === selectedGroup ? ' is-selected' : ''}${dragOver === value.id ? ' is-drop' : ''}${value.routes.length === 0 ? ' is-empty' : ''}`}
            onClick={() => chooseGroup(value.id)} onDragOver={event => { event.preventDefault(); setDragOver(value.id); }} onDragLeave={() => setDragOver('')} onDrop={event => onDropGroup(event, value)}>
            <strong>{value.label}</strong><small>{value.routes.length} 条线路 · {value.bound} 个账号{value.routes.length === 0 ? ' · 已阻断' : ''}</small>
          </button>)}
          {!groups.some(value => value.label === '未分组') && <div className="codex-proxy-manager-group is-staging" onDragOver={event => event.preventDefault()} onDrop={event => { event.preventDefault(); const value = event.dataTransfer.getData('text/plain'); if (value.startsWith('route:')) void moveRoute(value.slice(6), '未分组'); }}><strong>未分组 / 暂存</strong><small>拖入线路以从原组移出</small></div>}
          {groups.length === 0 && <p className="codex-proxy-manager-empty">暂无分组，请在中间添加线路。</p>}
        </div>
        <label className="codex-proxy-manager-new-group">新组名称<input value={newGroup} onChange={event => setNewGroup(event.target.value)} placeholder="拖入线路即可创建新组" onDragOver={event => event.preventDefault()} onDrop={event => {
          event.preventDefault(); const value = event.dataTransfer.getData('text/plain');
          if (value.startsWith('route:')) void moveRoute(value.slice(6), newGroup);
        }} /></label>
      </aside>
      <div className="codex-proxy-manager-routes">
        <div className="codex-proxy-manager-section-title"><span>{group?.label || '库存线路'} <b>{routes.length}</b></span><label><Search size={14} /><input aria-label="搜索线路" value={query} onChange={event => { setQuery(event.target.value); setScrollTop(0); routeScroller.current?.scrollTo(0, 0); }} placeholder="搜索名称或协议" /></label></div>
        <div className="codex-proxy-manager-virtual" ref={routeScroller} onScroll={event => setScrollTop(event.currentTarget.scrollTop)}>
          {routes.length === 0 ? <p className="codex-proxy-manager-empty">{group?.routes.length ? '没有匹配线路' : '当前组无线路。可拖入线路，或将绑定账号拖到其他组。'}</p> :
            <div style={{ height: routes.length * ROW_HEIGHT, position: 'relative' }}>{shown.map((item, index) => <div key={item.id} className="codex-proxy-manager-route" style={{ position: 'absolute', top: (start + index) * ROW_HEIGHT, height: ROW_HEIGHT - 5, left: 0, right: 0 }} draggable onDragStart={event => event.dataTransfer.setData('text/plain', `route:${item.id}`)}>
              <GripVertical size={15} className="drag-handle" /><div className="route-copy"><strong title={item.label}>{item.label}</strong><small>{item.protocol.toUpperCase()}{item.insecureTls ? ' · TLS 不安全' : ''}</small></div>
              <select aria-label={`移动 ${item.label} 到分组`} value="" disabled={!!busy} onChange={event => { const target = groups.find(value => value.id === event.target.value); if (target) void moveRoute(item.id, target); }}><option value="">移到…</option>{groups.filter(value => value.id !== item.groupId).map(value => <option key={value.id} value={value.id}>{value.label}</option>)}</select>
              <button type="button" title="编辑线路" aria-label={`编辑 ${item.label}`} onClick={() => edit(item)}><ArrowRightLeft size={14} /></button><button type="button" title="删除线路" aria-label={`删除 ${item.label}`} disabled={!!busy} onClick={() => removeRoute(item)}><Trash2 size={14} /></button>
            </div>)}</div>}
        </div>
        <details className="codex-proxy-manager-editor" open={editingId ? true : undefined}>
          <summary><Plus size={15} />{editingId ? '编辑线路' : '添加线路'}</summary>
          <div className="codex-proxy-manager-form"><input aria-label="线路名称" value={label} onChange={event => setLabel(event.target.value)} placeholder="线路备注名称" /><input aria-label="线路组名称" value={groupName} onChange={event => setGroupName(event.target.value)} placeholder={group?.label || '线路组名称'} /><input aria-label="代理链接" type="password" autoComplete="off" value={uri} onChange={event => setUri(event.target.value)} placeholder={editingId ? '留空保持原代理链接' : 'vless://… / hysteria2://… / tuic://…'} /><button type="button" className="btn btn-primary" disabled={!!busy} onClick={() => void saveRoute()}>保存</button>{editingId && <button type="button" className="btn btn-secondary" onClick={resetEditor}>取消</button>}</div>
        </details>
      </div>
      <aside className="codex-proxy-manager-accounts">
        <div className="codex-proxy-manager-section-title">账号绑定 <b>{bindings.filter(value => value.groupId === selectedGroup).length} / {bindings.length}</b></div>
        <div className="codex-proxy-manager-account-list">{sortedBindings.map(binding => {
          const account = accountById.get(binding.accountId);
          const displayed = account?.email || binding.accountId;
          return <div key={binding.accountId} className={`codex-proxy-manager-account${binding.groupId === selectedGroup ? ' is-in-group' : ''}${binding.blockedReason ? ' is-blocked' : ''}`} draggable onDragStart={event => event.dataTransfer.setData('text/plain', `account:${binding.accountId}`)}>
            <GripVertical size={14} className="drag-handle" /><div><strong title={privacyMode ? undefined : displayed}>{privacyMode ? '••••@••••' : displayed}</strong><small>{binding.blockedReason ? '线路不可用 · 已阻断直连' : binding.groupLabel || '未绑定线路组'}{binding.running ? ' · 运行中' : ''}</small></div>
            {group && binding.groupId !== group.id && <button type="button" disabled={!!busy || !group.routes.length} title={`绑定到 ${group.label}`} onClick={() => void bindAccount(binding.accountId, group)}><ShieldCheck size={15} /></button>}
          </div>;
        })}</div>
        <p className="codex-proxy-manager-note">拖动账号到左侧目标组，或先选择组再点击账号右侧绑定按钮。旧组为空也可以直接改绑；正在进行的请求可能短暂失败并需重试。</p>
      </aside>
    </div>
    <details className="codex-proxy-manager-import"><summary>从 V2Ray / Clash 订阅导入分组</summary><div className="codex-proxy-manager-form"><input aria-label="订阅 HTTPS 地址" type="password" autoComplete="off" value={subscriptionUrl} onChange={event => setSubscriptionUrl(event.target.value)} placeholder="HTTPS 订阅链接" /><input aria-label="导入组名称" value={subscriptionGroup} onChange={event => setSubscriptionGroup(event.target.value)} placeholder="目标线路组名称" /><select aria-label="订阅拉取网络" value={fetchViaId} onChange={event => setFetchViaId(event.target.value)}><option value="">未选库存代理</option>{items.filter(item => item.enabled).map(item => <option key={item.id} value={item.id}>{item.label}</option>)}</select>{!fetchViaId && <label><input type="checkbox" checked={allowDirect} onChange={event => setAllowDirect(event.target.checked)} />允许本机直连拉取订阅（可能暴露原始出口 IP）</label>}<button type="button" className="btn btn-secondary" disabled={!!busy || !subscriptionUrl.trim() || !subscriptionGroup.trim() || (!fetchViaId && !allowDirect)} onClick={() => void importSubscription()}>导入</button></div>{importReport && <p>新增 {importReport.imported} · 更新 {importReport.updated} · 不支持 {importReport.skipped} · 已在别组 {importReport.alreadyInOtherGroup}</p>}</details>
    {busy && <div className="codex-proxy-manager-busy" role="status">{busy}中…</div>}
  </section>;
}
