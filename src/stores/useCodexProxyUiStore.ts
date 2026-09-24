import { create } from 'zustand';

const STORAGE_KEY = 'v4-codex-show-remote-ip';
function initialVisibility(): boolean {
  try { return localStorage.getItem(STORAGE_KEY) !== 'false'; }
  catch { return true; }
}

interface CodexProxyUiState {
  showRemoteIp: boolean;
  toggleRemoteIp: () => void;
}

/** One visibility switch shared by every Codex account card. No IP is stored here. */
export const useCodexProxyUiStore = create<CodexProxyUiState>((set) => ({
  showRemoteIp: initialVisibility(),
  toggleRemoteIp: () => set((state) => {
    const showRemoteIp = !state.showRemoteIp;
    try { localStorage.setItem(STORAGE_KEY, String(showRemoteIp)); } catch { /* storage optional */ }
    return { showRemoteIp };
  }),
}));
