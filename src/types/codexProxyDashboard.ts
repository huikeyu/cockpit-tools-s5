export interface CodexProxyStatus {
  enabled: boolean;
  label: string | null;
  protocol: string | null;
  serverHost: string | null;
  serverPort: number | null;
  security: string | null;
  transport: string | null;
  localPort: number | null;
  running: boolean;
  uptimeSeconds: number | null;
  lastExitIp: string | null;
  lastProbeAt: number | null;
  lastProbeLatencyMs: number | null;
  required: boolean;
  isolationDir: string;
  groupId: string | null;
  groupLabel: string | null;
  routeCount: number;
  activeRouteIndex: number;
  failoverCount: number;
  blockedReason: string | null;
}

export interface CodexProxyGroupBinding {
  accountId: string;
  groupId: string | null;
  groupLabel: string | null;
  running: boolean;
  blockedReason: string | null;
}

export interface CodexProxyInventoryItem {
  id: string;
  label: string;
  groupId: string;
  groupLabel: string;
  enabled: boolean;
  protocol: string;
  serverHost: string;
  serverPort: number;
  insecureTls: boolean;
}

export interface CodexProxyRequestActivity {
  timestamp: number;
  requestId: string;
  model: string;
  requestedModel: string;
  upstreamModel: string;
  requestKind: string;
  gatewayMode: string | null;
  serviceTier: string | null;
  reasoningEffort: string | null;
  success: boolean;
  httpStatus: number | null;
  latencyMs: number;
  errorCategory: string;
  errorMessage: string;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  estimatedCostUsd: number;
  apiKeyLabel: string;
  clientInstanceId: string;
}

export interface CodexProxyAccountDashboard {
  proxy: CodexProxyStatus;
  loggedRequests: number;
  recentRequests: CodexProxyRequestActivity[];
  logsAvailable: boolean;
}
