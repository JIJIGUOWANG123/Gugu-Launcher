import { useEffect, useMemo, useRef, useState } from "react";
import { icons } from "../assets/icons";
import { HeadAvatar } from "../components/HeadAvatar";
import { Modal } from "../components/Modal";
import { copyText } from "../lib/clipboard";
import { useT } from "../lib/i18n";
import { api } from "../lib/tauri-api";
import type { Account, AuthlibStatus, MsAuthStatus, MsLoginStart } from "../lib/types";

interface Props {
  accounts: Account[] | null;
  onRefresh: () => Promise<void>;
  notify: (message: string) => void;
  /** 跳到「设置 → 内容服务」填微软 Client ID */
  onOpenSettings: () => void;
}

const NAME_MIN = 3;
const NAME_MAX = 16;

/** 第三方账号卡只显示站点域名，完整地址塞进 title（hover 可见）。 */
function siteHost(server: string): string {
  if (!server) return "";
  try {
    return new URL(server).host;
  } catch {
    return server;
  }
}

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

  /** 第三方账号「刷新登录」：重新换一次 accessToken，顺带把最新皮肤拉回来。 */
  const refreshLogin = async (account: Account) => {
    setBusyId(account.id);
    try {
      const refreshed = await api.accountRefreshThirdparty({ id: account.id });
      await onRefresh();
      notify(t("accounts.tp.refreshed", { name: refreshed.name }));
    } catch (error) {
      notify(error instanceof Error ? error.message : String(error));
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
                      {t(
                        account.type === "microsoft"
                          ? "accounts.type.microsoft"
                          : account.type === "thirdparty"
                            ? "accounts.type.thirdparty"
                            : "accounts.type.offline",
                      )}
                    </span>
                    {account.type === "thirdparty" ? (
                      <span className="account-row__site" data-account-site title={account.server ?? ""}>
                        {siteHost(account.server ?? "")}
                      </span>
                    ) : null}
                    <span className="mono account-row__uuid">{account.uuid.slice(0, 8)}</span>
                  </div>
                </div>
                <div className="account-row__actions">
                  {account.type === "thirdparty" ? (
                    <button
                      type="button"
                      className="pill-outline"
                      data-action="account-refresh"
                      data-account-refresh={account.name}
                      disabled={busyId === account.id}
                      onClick={() => void refreshLogin(account)}
                    >
                      {t("accounts.tp.refresh")}
                    </button>
                  ) : null}
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
          onAdded={async (name, kind) => {
            await onRefresh();
            setAddOpen(false);
            notify(
              kind === "ms"
                ? t("accounts.ms.loggedIn", { name })
                : kind === "thirdparty"
                  ? t("accounts.tp.added", { name })
                  : t("accounts.applied", { name }),
            );
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
  /** kind 用来区分 toast 文案（微软登录 →「已登录 X」，第三方 →「已添加 X」） */
  onAdded: (name: string, kind?: "ms" | "offline" | "thirdparty") => void | Promise<void>;
  notify: (message: string) => void;
  onOpenSettings: () => void;
}

/** 剩余有效时间：mm:ss */
function formatRemaining(seconds: number): string {
  const clamped = Math.max(0, seconds);
  const mm = String(Math.floor(clamped / 60)).padStart(2, "0");
  const ss = String(clamped % 60).padStart(2, "0");
  return mm + ":" + ss;
}

/** 添加账号悬浮窗：微软设备码登录 + 离线账号 + 第三方（皮肤站）三页。 */
function AddAccountModal({ accounts, onClose, onAdded, notify, onOpenSettings }: AddProps) {
  const t = useT();
  const [tab, setTab] = useState<"ms" | "offline" | "thirdparty">("ms");
  // 第三方（皮肤站）账号：地址默认 LittleSkin，密码只在本页内存里放着
  const [tpServer, setTpServer] = useState("https://littleskin.cn/api/yggdrasil");
  const [tpUser, setTpUser] = useState("");
  const [tpPass, setTpPass] = useState("");
  const [tpShowPass, setTpShowPass] = useState(false);
  const [tpError, setTpError] = useState("");
  const [tpBusy, setTpBusy] = useState(false);
  // authlib-injector：第三方登录的前置依赖，未安装时在弹窗底部提示下载
  const [authlib, setAuthlib] = useState<AuthlibStatus | null>(null);
  const [authlibError, setAuthlibError] = useState("");
  const [authlibBusy, setAuthlibBusy] = useState(false);
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

  // 设备码有效期倒计时（秒），每秒刷一次
  const [remaining, setRemaining] = useState(0);

  /** 打开登录页：地址来自后端（verificationUri / verifyUrl 两个名字都认）。 */
  const openVerifyPage = async (value: { verificationUri?: string | null; verifyUrl?: string | null } | null) => {
    const url = value?.verificationUri ?? value?.verifyUrl ?? "";
    if (!url) return;
    try {
      await api.openExternal(url);
    } catch {
      /* 打不开浏览器不影响用户手动点链接 */
    }
  };

  const beginMsLogin = async () => {
    setMsMessage("");
    try {
      const result = await api.msLoginStart();
      setStart(result);
      if (result.configured && result.deviceCode) {
        setPolling(true);
        startedAtRef.current = Date.now();
        setRemaining(result.expiresIn ?? 900);
        // 自动开浏览器 + 自动复制设备码（复制失败也不拦流程）
        await openVerifyPage(result);
        const copied = await copyText(result.userCode ?? "");
        notify(copied ? t("accounts.ms.codeCopied") : t("accounts.ms.copyFailed"));
      }
    } catch (error) {
      setMsMessage(String(error));
    }
  };

  useEffect(() => {
    if (!polling) return;
    const total = start?.expiresIn ?? 900;
    const tick = () => {
      const left = total - Math.floor((Date.now() - startedAtRef.current) / 1000);
      setRemaining(Math.max(0, left));
    };
    tick();
    const id = window.setInterval(tick, 1000);
    return () => window.clearInterval(id);
  }, [polling, start?.expiresIn]);

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
          await onAddedRef.current(result.account.name, "ms");
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

  // authlib-injector 状态：第三方账号登录前必须就绪
  useEffect(() => {
    let alive = true;
    void api
      .authlibStatus()
      .then((status) => {
        if (alive) setAuthlib(status);
      })
      .catch((error) => {
        if (alive) setAuthlibError(String(error));
      });
    return () => {
      alive = false;
    };
  }, []);

  const ensureAuthlib = async () => {
    setAuthlibBusy(true);
    setAuthlibError("");
    try {
      await api.authlibEnsure();
      notify(t("accounts.tp.authlibStarted"));
      setAuthlib(await api.authlibStatus());
    } catch (error) {
      setAuthlibError(t("accounts.tp.authlibFailed", { reason: String(error) }));
    } finally {
      setAuthlibBusy(false);
    }
  };

  const submitThirdparty = async () => {
    setTpError("");
    setTpBusy(true);
    try {
      const account = await api.accountAddThirdparty({
        server: tpServer.trim(),
        username: tpUser.trim(),
        password: tpPass,
      });
      // 加进来就设为当前账号，和另外两个标签一致
      await api.accountApply(account.id);
      await onAdded(account.name, "thirdparty");
    } catch (error) {
      setTpError(error instanceof Error ? error.message : String(error));
    } finally {
      setTpBusy(false);
    }
  };

  const copyCode = async () => {
    if (!start?.userCode) return;
    const copied = await copyText(start.userCode);
    notify(copied ? t("accounts.ms.codeCopied") : t("accounts.ms.copyFailed"));
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
        ) : tab === "thirdparty" ? (
          <>
            <button type="button" className="pill-outline" onClick={onClose}>
              {t("common.cancel")}
            </button>
            <button
              type="button"
              className="pill-button"
              data-action="thirdparty-login"
              disabled={tpBusy}
              onClick={() => void submitThirdparty()}
            >
              {tpBusy ? t("accounts.tp.loggingIn") : t("accounts.tp.login")}
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
        <button
          type="button"
          className={"segmented__item" + (tab === "thirdparty" ? " is-active" : "")}
          data-account-tab="thirdparty"
          onClick={() => setTab("thirdparty")}
        >
          {t("accounts.tab.thirdparty")}
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
                <button
                  type="button"
                  className="device-code__value device-code__value--xl mono"
                  data-ms-user-code
                  title={t("accounts.ms.recopy")}
                  onClick={() => void copyCode()}
                >
                  {start.userCode}
                </button>
              </div>
              <div className="device-code__tools">
                <button type="button" className="pill-outline" data-action="ms-recopy" onClick={() => void copyCode()}>
                  {t("accounts.ms.recopy")}
                </button>
                <button type="button" className="pill-outline" data-action="ms-reopen" onClick={() => void openVerifyPage(start)}>
                  {t("accounts.ms.reopen")}
                </button>
              </div>
              <div className="device-code__row">
                <span className="device-code__label">{t("accounts.ms.url")}</span>
                <a
                  className="device-code__link"
                  href={start.verificationUri ?? start.verifyUrl ?? "#"}
                  target="_blank"
                  rel="noreferrer"
                  data-ms-verify-url
                >
                  {start.verificationUri ?? start.verifyUrl}
                </a>
              </div>
              <div className="row__hint" data-ms-countdown>
                {t("accounts.ms.countdown", { time: formatRemaining(remaining) })}
              </div>
              <div className="row__hint" data-ms-state>
                {polling ? t("accounts.ms.pending") : msMessage || t("accounts.ms.expired")}
              </div>
            </div>
          ) : null}
          {msMessage && !polling ? <div className="row__hint">{msMessage}</div> : null}
        </div>
      ) : tab === "offline" ? (
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
      ) : (
        <div className="account-thirdparty" data-tp-form>
          <div className="field">
            <label htmlFor="tp-server">{t("accounts.tp.server")}</label>
            <input
              id="tp-server"
              type="text"
              value={tpServer}
              data-tp-server
              placeholder="https://littleskin.cn/api/yggdrasil"
              onChange={(event) => {
                setTpServer(event.target.value);
                setTpError("");
              }}
            />
            <div className="row__hint" data-tp-server-hint>
              {t("accounts.tp.serverHint")}
            </div>
          </div>

          <div className="field">
            <label htmlFor="tp-user">{t("accounts.tp.username")}</label>
            <input
              id="tp-user"
              type="text"
              value={tpUser}
              data-tp-username
              placeholder={t("accounts.tp.usernamePlaceholder")}
              onChange={(event) => {
                setTpUser(event.target.value);
                setTpError("");
              }}
            />
          </div>

          <div className="field">
            <label htmlFor="tp-pass">{t("accounts.tp.password")}</label>
            <div className="field-pair">
              <input
                id="tp-pass"
                type={tpShowPass ? "text" : "password"}
                value={tpPass}
                data-tp-password
                data-tp-password-visible={tpShowPass ? "1" : "0"}
                onChange={(event) => {
                  setTpPass(event.target.value);
                  setTpError("");
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter") void submitThirdparty();
                }}
              />
              <button
                type="button"
                className="pill-outline"
                data-tp-eye
                aria-pressed={tpShowPass}
                title={t(tpShowPass ? "accounts.tp.passwordHide" : "accounts.tp.passwordShow")}
                onClick={() => setTpShowPass((value) => !value)}
              >
                {t(tpShowPass ? "accounts.tp.passwordHide" : "accounts.tp.passwordShow")}
              </button>
            </div>
            <div className="row__hint">{t("accounts.tp.hint")}</div>
          </div>

          {tpError ? (
            <div className="field-error" data-tp-error>
              {tpError}
            </div>
          ) : null}

          {authlib && !authlib.installed ? (
            <div className="notice" data-authlib-missing>
              <strong>{t("accounts.tp.authlibMissing")}</strong>
              <span>{t("accounts.tp.authlibMissingHint")}</span>
              <button
                type="button"
                className="link-button"
                data-action="authlib-ensure"
                disabled={authlibBusy}
                onClick={() => void ensureAuthlib()}
              >
                {t("accounts.tp.authlibDownload")}
              </button>
            </div>
          ) : authlib && authlib.installed ? (
            <div className="row__hint" data-authlib-ready>
              {t("accounts.tp.authlibReady", { version: authlib.version ?? "" })}
            </div>
          ) : null}
          {authlibError ? (
            <div className="field-error" data-authlib-error>
              {authlibError}
            </div>
          ) : null}
        </div>
      )}
    </Modal>
  );
}
