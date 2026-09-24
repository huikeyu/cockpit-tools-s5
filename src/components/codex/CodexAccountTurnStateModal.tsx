import { CircleAlert, Info, RefreshCw, ShieldAlert, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useEscCloseTopmost } from "../../hooks/useEscClose";
import { useModalScrollLock } from "../../hooks/useModalScrollLock";
import type { CodexAccount } from "../../types/codex";
import type {
  CodexAccountTurnStateStatus,
  CodexTurnStateObservation,
} from "../../types/codexLocalAccess";
import "./CodexAccountTurnStateModal.css";

interface CodexAccountTurnStateModalProps {
  /** 可检测账号（仅 OAuth / Agent Identity，调用方已过滤）。 */
  accounts: CodexAccount[];
  statusMap: Record<string, CodexAccountTurnStateStatus>;
  probingIds: string[];
  errors: Record<string, string>;
  maskAccountText: (value: string) => string;
  onProbe: (accountId: string) => void;
  onProbeAll: (accountIds: string[]) => void;
  onClose: () => void;
}

function resolveStatusTone(status: string, suspected: boolean): string {
  if (suspected || status === "suspected") return "suspected";
  if (status === "normal") return "normal";
  if (status === "abnormal") return "abnormal";
  return "unknown";
}

export function CodexAccountTurnStateModal({
  accounts,
  statusMap,
  probingIds,
  errors,
  maskAccountText,
  onProbe,
  onProbeAll,
  onClose,
}: CodexAccountTurnStateModalProps) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language || "zh-CN";
  const formatObservedAt = (milliseconds: number) =>
    new Date(milliseconds).toLocaleString(locale, {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    });
  useModalScrollLock(true);
  useEscCloseTopmost(true, onClose);

  const probingIdSet = new Set(probingIds);
  const busy = probingIds.length > 0;
  const checkableIds = accounts.map((account) => account.id);

  const statusLabel = (status: string, suspected: boolean) => {
    if (suspected || status === "suspected") {
      return t("codex.turnState.statusSuspected", "疑似风控");
    }
    if (status === "normal") return t("codex.turnState.statusNormal", "正常");
    if (status === "abnormal") return t("codex.turnState.statusAbnormal", "异常");
    return t("codex.turnState.statusUnknown", "未检测");
  };

  const reasonLabel = (reason?: string | null) => {
    switch ((reason || "").trim()) {
      case "turnStateSuspected":
        return t("codex.turnState.reasonSuspected", "state 为 312");
      case "turnStateAbnormal":
        return t("codex.turnState.reasonAbnormal", "state 长度异常");
      case "turnStateMissing":
        return t("codex.turnState.reasonMissing", "响应未返回 state");
      default:
        return "";
    }
  };

  const observationLabel = (observation: CodexTurnStateObservation) => {
    if (typeof observation.length === "number" && observation.length > 0) {
      return t("codex.turnState.stateLength", {
        length: observation.length,
        defaultValue: "state {{length}}",
      });
    }
    return t("codex.turnState.stateMissing", "未返回 state");
  };

  const describeStatus = (status?: CodexAccountTurnStateStatus) => {
    if (!status) return "";
    const parts: string[] = [];
    if (typeof status.lastLength === "number" && status.lastLength > 0) {
      parts.push(
        t("codex.turnState.stateLength", {
          length: status.lastLength,
          defaultValue: "state {{length}}",
        }),
      );
    } else if (status.lastClass === "missing") {
      parts.push(t("codex.turnState.stateMissing", "未返回 state"));
    }
    const reason = reasonLabel(status.reason);
    if (reason) parts.push(reason);
    return parts.join(" · ");
  };

  return (
    <div className="modal-overlay">
      <div className="modal codex-turn-state-modal">
        <div className="modal-header">
          <div className="codex-turn-state-title">
            <ShieldAlert size={16} />
            <span>{t("codex.turnState.modalTitle", "账号风控检测")}</span>
          </div>
          <button
            type="button"
            className="btn btn-secondary icon-only"
            onClick={onClose}
            aria-label={t("common.close", "关闭")}
          >
            <X size={14} />
          </button>
        </div>
        <div className="modal-body">
          <div className="codex-turn-state-toolbar">
            <button
              type="button"
              className="btn btn-primary"
              onClick={() => onProbeAll(checkableIds)}
              disabled={busy || checkableIds.length === 0}
            >
              {busy ? (
                <RefreshCw size={14} className="loading-spinner" />
              ) : (
                <ShieldAlert size={14} />
              )}
              <span>
                {busy
                  ? t("codex.turnState.checkingCount", {
                      count: probingIds.length,
                      defaultValue: "检测中 {{count}}",
                    })
                  : t("codex.turnState.checkAll", "检测全部")}
              </span>
            </button>
            <span className="codex-turn-state-count">
              {t("codex.turnState.accountCount", {
                count: accounts.length,
                defaultValue: "{{count}} 个可检测账号",
              })}
            </span>
          </div>
          <div className="codex-turn-state-list">
            {accounts.length === 0 ? (
              <div className="codex-turn-state-empty">
                <Info size={14} />
                <span>
                  {t(
                    "codex.turnState.emptyAccounts",
                    "没有可检测的 OAuth 账号（API Key 账号没有上游 state）。",
                  )}
                </span>
              </div>
            ) : (
              accounts.map((account) => {
                const status = statusMap[account.id];
                const tone = resolveStatusTone(
                  status?.status || "unknown",
                  Boolean(status?.suspected),
                );
                const probing = probingIdSet.has(account.id);
                const error = errors[account.id];
                const detail = describeStatus(status);
                return (
                  <div
                    key={account.id}
                    className={`codex-turn-state-row codex-turn-state-row--${tone}`}
                  >
                    <div className="codex-turn-state-row-main">
                      <span className={`codex-turn-state-pill ${tone}`}>
                        {probing ? (
                          <RefreshCw size={11} className="loading-spinner" />
                        ) : tone === "suspected" || tone === "abnormal" ? (
                          <CircleAlert size={11} />
                        ) : null}
                        {statusLabel(status?.status || "unknown", Boolean(status?.suspected))}
                      </span>
                      <span className="codex-turn-state-email" title={account.email}>
                        {maskAccountText(account.email)}
                      </span>
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={() => onProbe(account.id)}
                        disabled={probing}
                      >
                        {probing
                          ? t("codex.turnState.checking", "检测中")
                          : t("codex.turnState.checkOne", "检测")}
                      </button>
                    </div>
                    <div className="codex-turn-state-row-meta">
                      {status?.lastObservedAt ? (
                        <span>
                          {t("codex.turnState.lastChecked", {
                            time: formatObservedAt(status.lastObservedAt),
                            defaultValue: "最近检测 {{time}}",
                          })}
                        </span>
                      ) : (
                        <span>{t("codex.turnState.neverChecked", "尚未检测")}</span>
                      )}
                      {detail ? <span>{detail}</span> : null}
                      {status?.lastHttpStatus ? (
                        <span>
                          {t("codex.apiService.logs.httpStatus", {
                            status: status.lastHttpStatus,
                            defaultValue: "HTTP {{status}}",
                          })}
                        </span>
                      ) : null}
                      {status?.observations?.length
                        ? status.observations
                            .slice(-3)
                            .reverse()
                            .map((observation, index) => (
                              <span
                                key={`${account.id}-${observation.observedAt}-${index}`}
                                className={`codex-turn-state-mini ${
                                  observation.class === "normal"
                                    ? "normal"
                                    : observation.class === "suspected"
                                      ? "suspected"
                                      : "abnormal"
                                }`}
                              >
                                {observationLabel(observation)}
                              </span>
                            ))
                        : null}
                    </div>
                    {error ? (
                      <div className="codex-turn-state-row-error">
                        {t("codex.turnState.checkFailed", {
                          message: error,
                          defaultValue: "检测失败：{{message}}",
                        })}
                      </div>
                    ) : null}
                  </div>
                );
              })
            )}
          </div>
        </div>
        <div className="modal-footer">
          <button type="button" className="btn btn-secondary" onClick={onClose}>
            {t("common.close", "关闭")}
          </button>
        </div>
      </div>
    </div>
  );
}
