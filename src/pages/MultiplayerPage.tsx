import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { icons } from "../assets/icons";
import { ServersPanel } from "../components/ServersPanel";
import { useDownloads } from "../lib/downloads-store";
import { useT } from "../lib/i18n";
import { api, subscribeLauncherEvents } from "../lib/tauri-api";
import type { TeraState, TeraStatus } from "../lib/types";

interface Props {
  notify: (message: string) => void;
}

const MAX_LOG_LINES = 200;
const CODE_PATTERN = /^[A-Za-z0-9-]{6,12}$/;

/**
 * 联机房间 tab（陶瓦联机）：未安装只给安装入口；装好之后才出现「启动主机 / 加入联机」。
 * 日志来自 tera://log，状态来自 tera://state，最多留 200 行。
 */
function RoomsTab({ notify }: Props) {
  const t = useT();
  const { refresh: refreshDownloads } = useDownloads();
  const [status, setStatus] = useState<TeraStatus | null>(null);
  const [installing, setInstalling] = useState(false);
  const [state, setState] = useState<TeraState>({ hosting: false, roomCode: null, players: [] });
  const [mode, setMode] = useState<"none" | "host" | "join">("none");
  const [roomName, setRoomName] = useState("");
  const [port, setPort] = useState("");
  const [code, setCode] = useState("");
  const [joinError, setJoinError] = useState("");
  const [busy, setBusy] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const [logOpen, setLogOpen] = useState(false);
  const logRef = useRef<HTMLPreElement | null>(null);

  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await api.teraStatus());
    } catch (error) {
      notify(String(error));
    }
  }, [notify]);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  // 事件总线：日志与状态各一条流
  useEffect(() => {
    let off: (() => void) | null = null;
    let alive = true;
    void subscribeLauncherEvents((event) => {
      if (event.kind === "tera-log") {
        setLogs((prev) => [...prev, event.payload.line].slice(-MAX_LOG_LINES));
        return;
      }
      if (event.kind === "tera-state") {
        setState(event.payload);
      }
    })
      .then((unsubscribe) => {
        if (alive) off = unsubscribe;
        else unsubscribe();
      })
      .catch(() => undefined);
    return () => {
      alive = false;
      off?.();
    };
  }, []);

  // 安装中轮询状态：mock 会在 1.6s 后置为已安装，真后端由后端决定
  useEffect(() => {
    if (!installing) return;
    const timer = window.setInterval(() => {
      void api
        .teraStatus()
        .then((next) => {
          setStatus(next);
          if (next.installed) {
            setInstalling(false);
            notify(t("multi.installed") + " · " + t("multi.version", { version: next.version ?? "" }));
          }
        })
        .catch(() => undefined);
    }, 900);
    return () => window.clearInterval(timer);
  }, [installing, notify, t]);

  useEffect(() => {
    if (!logOpen) return;
    const node = logRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [logs.length, logOpen]);

  const installed = Boolean(status?.installed);

  const install = async () => {
    setBusy(true);
    try {
      await api.teraInstall();
      setInstalling(true);
      refreshDownloads();
      notify(t("multi.installStarted"));
    } catch (error) {
      notify(String(error));
    } finally {
      setBusy(false);
    }
  };

  const startHost = async () => {
    setBusy(true);
    try {
      const next = await api.teraStartHost({
        roomName: roomName.trim() || undefined,
        port: port.trim() ? Number(port.trim()) : undefined,
      });
      setState(next);
      setMode("none");
      notify(t("multi.hostStarted", { code: next.roomCode ?? "" }));
    } catch (error) {
      notify(String(error));
    } finally {
      setBusy(false);
    }
  };

  const join = async () => {
    const trimmed = code.trim();
    if (!CODE_PATTERN.test(trimmed)) {
      setJoinError(t("multi.codeInvalid"));
      return;
    }
    setBusy(true);
    setJoinError("");
    try {
      const next = await api.teraJoin(trimmed);
      setState(next);
      setMode("none");
      notify(t("multi.joined", { code: next.roomCode ?? trimmed }));
    } catch (error) {
      setJoinError(t("multi.joinFailed", { message: String(error) }));
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    setBusy(true);
    try {
      await api.teraStop();
      setState({ hosting: false, roomCode: null, players: [] });
      notify(t("multi.stopped"));
    } catch (error) {
      notify(String(error));
    } finally {
      setBusy(false);
    }
  };

  const copyCode = async () => {
    if (!state.roomCode) return;
    try {
      await navigator.clipboard.writeText(state.roomCode);
      notify(t("multi.copied"));
    } catch {
      notify(t("multi.copyFailed"));
    }
  };

  const statusText = useMemo(() => {
    if (status === null) return t("common.loading");
    if (!installed) return t("multi.notInstalled");
    return t("multi.installed") + " · " + t("multi.version", { version: status.version ?? "" });
  }, [status, installed, t]);

  return (
    <div className="multi-rooms">
      <section className="card multi-intro">
        <div className="multi-intro__head">
          <span className="detail-card__title">{t("multi.introTitle")}</span>
          <span className="chip">{t("multi.subtitle")}</span>
        </div>
        <p className="multi-intro__body">{t("multi.intro")}</p>
        <p className="row__hint">{t("multi.introHint")}</p>
      </section>

      <section className="card multi-status">
        <div className="multi-status__row">
          <div>
            <div className="detail-card__title">{t("multi.status")}</div>
            <div className={installed ? "multi-status__value is-ok" : "multi-status__value"} data-tera-status>
              {statusText}
            </div>
          </div>
          <div className="multi-status__actions">
            {!installed ? (
              <button
                type="button"
                className="pill-button"
                data-action="tera-install"
                disabled={busy || installing}
                onClick={() => void install()}
              >
                {installing ? t("multi.installing") : t("multi.install")}
              </button>
            ) : (
              <>
                <button
                  type="button"
                  className="pill-outline"
                  data-action="tera-host"
                  onClick={() => setMode(mode === "host" ? "none" : "host")}
                >
                  {t("multi.host")}
                </button>
                <button
                  type="button"
                  className="pill-outline"
                  data-action="tera-join"
                  onClick={() => setMode(mode === "join" ? "none" : "join")}
                >
                  {t("multi.join")}
                </button>
              </>
            )}
          </div>
        </div>

        {!installed ? <div className="row__hint" data-tera-need-install>{t("multi.needInstall")}</div> : null}

        {installed && mode === "host" ? (
          <div className="multi-panel" data-tera-panel="host">
            <div className="multi-panel__title">{t("multi.hostTitle")}</div>
            <div className="multi-form">
              <div className="field">
                <label htmlFor="tera-room">{t("multi.roomName")}</label>
                <input
                  id="tera-room"
                  type="text"
                  value={roomName}
                  data-tera-room-name
                  placeholder={t("multi.roomNamePlaceholder")}
                  onChange={(event) => setRoomName(event.target.value)}
                />
              </div>
              <div className="field">
                <label htmlFor="tera-port">{t("multi.port")}</label>
                <input
                  id="tera-port"
                  type="text"
                  inputMode="numeric"
                  value={port}
                  data-tera-port
                  placeholder={t("multi.portPlaceholder")}
                  onChange={(event) => setPort(event.target.value.replace(/[^0-9]/g, ""))}
                />
              </div>
            </div>
            <div className="multi-panel__actions">
              <button type="button" className="pill-button" data-action="tera-start-host" disabled={busy} onClick={() => void startHost()}>
                {t("multi.startHost")}
              </button>
            </div>
          </div>
        ) : null}

        {installed && mode === "join" ? (
          <div className="multi-panel" data-tera-panel="join">
            <div className="multi-panel__title">{t("multi.joinTitle")}</div>
            <div className="field">
              <label htmlFor="tera-code">{t("multi.code")}</label>
              <input
                id="tera-code"
                type="text"
                value={code}
                data-tera-code
                placeholder={t("multi.codePlaceholder")}
                onChange={(event) => {
                  setCode(event.target.value);
                  setJoinError("");
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter") void join();
                }}
              />
              {joinError ? (
                <div className="field-error" data-tera-join-error>
                  {joinError}
                </div>
              ) : null}
            </div>
            <div className="multi-panel__actions">
              <button type="button" className="pill-button" data-action="tera-join-now" disabled={busy} onClick={() => void join()}>
                {t("multi.joinNow")}
              </button>
            </div>
          </div>
        ) : null}

        {installed && state.roomCode ? (
          <div className="multi-room" data-tera-room>
            <div className="multi-room__code-row">
              <span className="multi-room__label">{t("multi.roomCode")}</span>
              <code className="multi-room__code" data-tera-code-value>
                {state.roomCode}
              </code>
              <button type="button" className="pill-outline pill-outline--tiny" data-action="tera-copy" onClick={() => void copyCode()}>
                {t("multi.copy")}
              </button>
              <span className={"multi-room__mode" + (state.hosting ? " is-host" : "")}>
                {state.hosting ? t("multi.host") : t("multi.join")}
              </span>
            </div>
            <div className="multi-room__players">
              <span className="multi-room__label">{t("multi.players")}</span>
              {state.players.length === 0 ? (
                <span className="row__hint">{t("multi.playersEmpty")}</span>
              ) : (
                <span className="multi-players" data-tera-players>
                  {state.players.map((player) => (
                    <span className="multi-player" key={player}>
                      {player}
                    </span>
                  ))}
                </span>
              )}
            </div>
            <div className="multi-room__actions">
              <button type="button" className="pill-outline pill-outline--danger" data-action="tera-stop" disabled={busy} onClick={() => void stop()}>
                {t("multi.stop")}
              </button>
            </div>
          </div>
        ) : null}
      </section>

      <section className="card multi-log">
        <button type="button" className="multi-log__head" data-action="tera-log-toggle" onClick={() => setLogOpen((open) => !open)}>
          <span className="detail-card__title">{t("multi.log")}</span>
          <span className="row__hint">
            {t("multi.logHint")} · {logs.length}
          </span>
          <span className="multi-log__toggle">
            {logOpen ? t("multi.collapse") : t("multi.expand")}
            <span className={"multi-log__chevron" + (logOpen ? " is-open" : "")} aria-hidden="true" />
          </span>
        </button>
        {logOpen ? (
          logs.length === 0 ? (
            <div className="empty-hint">{t("multi.logEmpty")}</div>
          ) : (
            <pre className="log-view multi-log__view" ref={logRef} data-tera-log>
              {logs.map((line, index) => (
                <div className="log-line" key={index}>
                  {line}
                </div>
              ))}
            </pre>
          )
        ) : null}
      </section>

      <section className="card card-row multi-help">
        <img src={icons.alert} alt="" width={16} height={16} />
        <span className="row__hint">{t("multi.introHint")}</span>
      </section>
    </div>
  );
}

/**
 * 多人页：上面两个 tab（服务器 / 联机房间），默认停在「服务器」。
 * 「+ 创建服务器」在服务器 tab 的右上角（ServersPanel 里）。
 */
export function MultiplayerPage({ notify }: Props) {
  const t = useT();
  const [tab, setTab] = useState<"servers" | "rooms">("servers");

  return (
    <div className="multi-page">
      <nav className="multi-tabs" role="tablist" aria-label={t("multi.title")}>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "servers"}
          className={"multi-tab" + (tab === "servers" ? " is-active" : "")}
          data-multi-tab="servers"
          onClick={() => setTab("servers")}
        >
          {t("multi.tab.servers")}
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "rooms"}
          className={"multi-tab" + (tab === "rooms" ? " is-active" : "")}
          data-multi-tab="rooms"
          onClick={() => setTab("rooms")}
        >
          {t("multi.tab.rooms")}
        </button>
      </nav>

      {tab === "servers" ? <ServersPanel notify={notify} /> : <RoomsTab notify={notify} />}
    </div>
  );
}
