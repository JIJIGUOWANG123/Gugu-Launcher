import { useEffect, useMemo, useRef, useState } from "react";
import { icons } from "../assets/icons";
import { HeadAvatar } from "../components/HeadAvatar";
import { Modal } from "../components/Modal";
import { useT } from "../lib/i18n";
import { api } from "../lib/tauri-api";
import type { Account, MsAuthStatus, MsLoginStart } from "../lib/types";

interface Props {
  accounts: Account[] | null;
  onRefresh: () => Promise<void>;
  notify: (message: string) => void;
  /** 跳到「设置 → 内容服务」填微软 Client ID */
  onOpenSettings: () => void;
}

const NAME_MIN = 3;
const NAME_MAX = 16;

/** 账号页：账号卡列表 + 「添加账号」悬浮窗（微软设备码 / 离线）。 */
export function AccountsPage({ accounts, onRefresh, notify, onOpenSettings }: Props) {
  const t = useT();
  const [msStatus, setMsStatus] = useState<MsAuthStatus | null>(null);

  useEffect(() => {
    let alive = true;
    void api
      .msAuthStatus()
      .then((status) => {
        if (alive) setMsStatus(status);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);
  const [addOpen, setAddOpen] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<Account | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const apply = async (account: Account) => {
    setBusyId(account.id);
    try {
      await api.accountApply(account.id);
      await onRefresh();
      notify(t("accounts.applied", { name: account.name }));
    } catch (error) {
      notify(String(error));
    } finally {
      setBusyId(null);
    }
  };

  const remove = async () => {
    if (!pendingDelete) return;
    const target = pendingDelete;
    setPendingDelete(null);
    try {
      await api.accountDelete(target.id);
      await onRefresh();
      notify(t("accounts.deleted", { name: target.name }));
    } catch (error) {
      notify(String(error));
    }
  };

  return (
    <div className="accounts-page">
      <section className="card accounts-card">
        <div className="accounts-card__head">
          <span className="detail-card__title">{t("nav.accounts")}</span>
          <button type="button" className="pill-button" data-action="account-add" onClick={() => setAddOpen(true)}>
            + {t("accounts.add")}
          </button>
        </div>

        {accounts === null ? (
          <div className="row__hint" data-accounts-loading>
            {t("common.loading")}
          </div>
        ) : accounts.length === 0 ? (
          <div className="empty-hint" data-accounts-empty>
            {t("accounts.empty")}
          </div>
        ) : (
          <div className="account-list">
            {accounts.map((account) => (
              <div
                key={account.id}
                className={"account-row" + (account.current ? " is-current" : "")}
                data-account={account.name}
                data-account-current={account.current ? "1" : "0"}
              >
                <HeadAvatar skin={account.skinDataUrl} size={40} className="account-row__avatar" />
                <div className="account-row__meta">
                  <div className="account-row__name">
                    {account.name}
                    {account.current ? (
                      <span className="account-row__check" title={t("accounts.current")}>
                        ✓
                      </span>
                    ) : null}
                  </div>
                  <div className="account-row__tags">
                    <span className={"account-badge account-badge--" + account.type}>
                      {t(account.type === "microsoft" ? "accounts.type.microsoft" : "accounts.type.offline")}
                    </span>
                    <span className="mono account-row__uuid">{account.uuid.slice(0, 8)}</span>
                  </div>
                </div>
                <div className="account-row__actions">
                  <button
                    type="button"
                    className="pill-outline"
                    data-action="account-apply"
                    disabled={account.current || busyId === account.id}
                    onClick={() => void apply(account)}
                  >
                    {t("accounts.apply")}
                  </button>
                  <button
                    type="button"
                    className="icon-button"
                    data-action="account-delete"
                    title={t("common.delete")}
                    onClick={() => setPendingDelete(account)}
                  >
                    <img src={icons.trash} alt={t("common.delete")} />
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}

        {msStatus && !msStatus.configured ? (
          <div className="notice" data-ms-client-missing>
            <strong>{t("accounts.ms.notConfiguredClientId")}</strong>
            <span>{msStatus.invalid ? t("settings.msClientIdInvalidStored") : t("accounts.ms.notConfiguredHint")}</span>
            <button type="button" className="link-button" data-action="ms-go-settings" onClick={onOpenSettings}>
              {t("accounts.ms.goSettings")}
            </button>
          </div>
        ) : null}

        <div className="row__hint">{t("accounts.launchHint")}</div>
      </section>

      {addOpen ? (
        <AddAccountModal
          accounts={accounts ?? []}
          onOpenSettings={onOpenSettings}
          onClose={() => setAddOpen(false)}
          onAdded={async (name) => {
            await onRefresh();
            setAddOpen(false);
            notify(t("accounts.applied", { name }));
          }}
          notify={notify}
        />
      ) : null}

      {pendingDelete ? (
        <Modal
          title={t("accounts.deleteTitle")}
          onClose={() => setPendingDelete(null)}
          footer={
            <>
              <button type="button" className="pill-outline" onClick={() => setPendingDelete(null)}>
                {t("common.cancel")}
              </button>
              <button type="button" className="pill-button" data-action="account-delete-confirm" onClick={() => void remove()}>
                {t("common.delete")}
              </button>
            </>
          }
        >
          <p>{t("accounts.deleteConfirm", { name: pendingDelete.name })}</p>
        </Modal>
      ) : null}
    </div>
  );
}

interface AddProps {
  accounts: Account[];
  onClose: () => void;
  onAdded: (name: string) => void | Promise<void>;
  notify: (message: string) => void;
  onOpenSettings: () => void;
}

/** 添加账号悬浮窗：微软设备码登录 + 离线账号两页。 */
function AddAccountModal({ accounts, onClose, onAdded, notify, onOpenSettings }: AddProps) {
  const t = useT();
  const [tab, setTab] = useState<"ms" | "offline">("ms");
  const [start, setStart] = useState<MsLoginStart | null>(null);
  const [polling, setPolling] = useState(false);
  const [msMessage, setMsMessage] = useState("");
  const [offlineName, setOfflineName] = useState("");
  const [offlineError, setOfflineError] = useState("");
  const timerRef = useRef<number | null>(null);
  const startedAtRef = useRef<number>(0);

  const names = useMemo(() => accounts.map((account) => account.name.toLowerCase()), [accounts]);

  // 打开悬浮窗先问「Client ID 配了吗」：没配就把登录按钮置灰，并发起登录由用户点按钮触发
  const [msStatus, setMsStatus] = useState<MsAuthStatus | null>(null);

  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const status = await api.msAuthStatus();
        if (!alive) return;
        setMsStatus(status);
      } catch (error) {
        if (alive) setMsMessage(String(error));
      }
    })();
    return () => {
      alive = false;
    };
  }, []);

  const beginMsLogin = async () => {
    setMsMessage("");
    try {
      const result = await api.msLoginStart();
      setStart(result);
      if (result.configured && result.deviceCode) {
        setPolling(true);
        startedAtRef.current = Date.now();
      }
    } catch (error) {
      setMsMessage(String(error));
    }
  };

  // onAdded 每次父组件渲染都是新函数，用 ref 拿最新值，避免把轮询 effect 重订阅（会把设备码立刻轮询完）
  const onAddedRef = useRef(onAdded);
  useEffect(() => {
    onAddedRef.current = onAdded;
  }, [onAdded]);

  // 设备码轮询：pending 就继续，done 收号，error 停
  const deviceCode = start?.deviceCode ?? "";
  const expiresIn = start?.expiresIn ?? 900;
  useEffect(() => {
    if (!polling || !deviceCode) return;
    let alive = true;
    const tick = async () => {
      if (Date.now() - startedAtRef.current > expiresIn * 1000) {
        setPolling(false);
        setMsMessage(t("accounts.ms.expired"));
        return;
      }
      try {
        const result = await api.msLoginPoll(deviceCode);
        if (!alive) return;
        if (result.state === "pending") return;
        setPolling(false);
        if (result.state === "done" && result.account) {
          await onAddedRef.current(result.account.name);
          return;
        }
        setMsMessage(result.message ?? t("accounts.ms.expired"));
      } catch (error) {
        if (!alive) return;
        setPolling(false);
        setMsMessage(String(error));
      }
    };
    timerRef.current = window.setInterval(() => void tick(), 2500);
    void tick();
    return () => {
      alive = false;
      if (timerRef.current !== null) {
        window.clearInterval(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [polling, deviceCode, expiresIn, t]);

  const submitOffline = async () => {
    const name = offlineName.trim();
    if (name.length < NAME_MIN || name.length > NAME_MAX) {
      setOfflineError(t("accounts.offline.invalid"));
      return;
    }
    if (names.includes(name.toLowerCase())) {
      setOfflineError(t("accounts.offline.exists"));
      return;
    }
    try {
      const account = await api.accountAddOffline(name);
      await api.accountApply(account.id);
      await onAdded(account.name);
    } catch (error) {
      setOfflineError(String(error));
    }
  };

  const copyCode = async () => {
    if (!start?.userCode) return;
    try {
      await navigator.clipboard.writeText(start.userCode);
      notify(start.userCode);
    } catch {
      /* 剪贴板不可用就算了，用户能看见设备码 */
    }
  };

  const msConfigured = Boolean(msStatus?.configured);
  const msRunning = Boolean(start?.configured && start.userCode);

  return (
    <Modal
      title={t("accounts.addTitle")}
      onClose={onClose}
      footer={
        tab === "offline" ? (
          <>
            <button type="button" className="pill-outline" onClick={onClose}>
              {t("common.cancel")}
            </button>
            <button type="button" className="pill-button" data-action="offline-add" onClick={() => void submitOffline()}>
              {t("accounts.offline.add")}
            </button>
          </>
        ) : null
      }
    >
      <div className="segmented">
        <button
          type="button"
          className={"segmented__item" + (tab === "ms" ? " is-active" : "")}
          data-account-tab="ms"
          onClick={() => setTab("ms")}
        >
          {t("accounts.tab.ms")}
        </button>
        <button
          type="button"
          className={"segmented__item" + (tab === "offline" ? " is-active" : "")}
          data-account-tab="offline"
          onClick={() => setTab("offline")}
        >
          {t("accounts.tab.offline")}
        </button>
      </div>

      {tab === "ms" ? (
        <div className="account-ms">
          <p className="row__hint">{t("accounts.ms.waiting")}</p>
          <div className="account-ms__actions">
            <button
              type="button"
              className="pill-button"
              data-action="ms-login-start"
              disabled={msStatus === null || !msConfigured || msRunning || polling}
              onClick={() => void beginMsLogin()}
            >
              {msConfigured ? t("accounts.ms.start") : t("accounts.ms.notConfiguredClientId")}
            </button>
            {msStatus !== null && !msConfigured ? (
              <button type="button" className="link-button" data-action="ms-go-settings" onClick={onOpenSettings}>
                {t("accounts.ms.goSettings")}
              </button>
            ) : null}
          </div>
          {msStatus !== null && !msConfigured ? (
            <div className="notice" data-ms-not-configured>
              <strong>{t("accounts.ms.notConfigured")}</strong>
              <span>{msStatus.invalid ? t("settings.msClientIdInvalidStored") : t("accounts.ms.notConfiguredHint")}</span>
            </div>
          ) : null}
          {start?.configured && start.userCode ? (
            <div className="device-code">
              <div className="device-code__row">
                <span className="device-code__label">{t("accounts.ms.code")}</span>
                <button type="button" className="device-code__value mono" data-ms-user-code onClick={() => void copyCode()}>
                  {start.userCode}
                </button>
              </div>
              <div className="device-code__row">
                <span className="device-code__label">{t("accounts.ms.url")}</span>
                <a className="device-code__link" href={start.verifyUrl ?? "#"} target="_blank" rel="noreferrer" data-ms-verify-url>
                  {start.verifyUrl}
                </a>
              </div>
              <div className="row__hint" data-ms-state>
                {polling ? t("accounts.ms.pending") : msMessage || t("accounts.ms.expired")}
              </div>
            </div>
          ) : null}
          {msMessage && !polling ? <div className="row__hint">{msMessage}</div> : null}
        </div>
      ) : (
        <div className="account-offline">
          <div className="field">
            <label htmlFor="offline-name">{t("accounts.offline.name")}</label>
            <input
              id="offline-name"
              type="text"
              value={offlineName}
              placeholder={t("accounts.offline.placeholder")}
              maxLength={NAME_MAX}
              onChange={(event) => {
                setOfflineName(event.target.value);
                setOfflineError("");
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") void submitOffline();
              }}
            />
            {offlineError ? (
              <div className="field-error" data-offline-error>
                {offlineError}
              </div>
            ) : (
              <div className="row__hint">{t("accounts.offline.hint")}</div>
            )}
          </div>
        </div>
      )}
    </Modal>
  );
}
