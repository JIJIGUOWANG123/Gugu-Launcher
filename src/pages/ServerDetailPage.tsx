import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { icons } from "../assets/icons";
import { Modal } from "../components/Modal";
import { formatBytes } from "../lib/format";
import { useT } from "../lib/i18n";
import { api, subscribeLauncherEvents } from "../lib/tauri-api";
import type {
  ServerConfigResult,
  ServerConsoleState,
  ServerContentItem,
  ServerInstance,
  ServerSaveItem,
  ServerState,
} from "../lib/types";

interface Props {
  server: ServerInstance;
  onBack: () => void;
  notify: (message: string) => void;
  /** 「从资源页安装」：切到资源页 */
  onOpenContent: () => void;
  /** 删掉之后回列表 */
  onDeleted: () => void;
}

type ServerTab = "mods" | "plugins" | "saves" | "config" | "logs";

const TABS: Array<{ key: ServerTab; labelKey: string }> = [
  { key: "mods", labelKey: "server.detail.mods" },
  { key: "plugins", labelKey: "server.detail.plugins" },
  { key: "saves", labelKey: "server.detail.saves" },
  { key: "config", labelKey: "server.detail.config" },
  { key: "logs", labelKey: "server.detail.logs" },
];

/** server.properties 里做成表单的常用项（其余内容留在「以文本编辑」里）。 */
const CONFIG_FIELDS: Array<{
  key: string;
  label: string;
  type: "text" | "number" | "bool" | "select";
  options?: string[];
}> = [
  { key: "server-port", label: "端口", type: "number" },
  { key: "motd", label: "描述（MOTD）", type: "text" },
  { key: "max-players", label: "最大玩家数", type: "number" },
  { key: "online-mode", label: "正版验证", type: "bool" },
  { key: "difficulty", label: "难度", type: "select", options: ["peaceful", "easy", "normal", "hard"] },
  { key: "gamemode", label: "游戏模式", type: "select", options: ["survival", "creative", "adventure", "spectator"] },
  { key: "level-name", label: "世界名", type: "text" },
  { key: "view-distance", label: "视距（区块）", type: "number" },
  { key: "simulation-distance", label: "模拟距离（区块）", type: "number" },
  { key: "pvp", label: "允许 PvP", type: "bool" },
  { key: "allow-flight", label: "允许飞行", type: "bool" },
  { key: "white-list", label: "白名单", type: "bool" },
];

const MAX_LOG_LINES = 500;
/** 控制台历史只留最近 20 条，按 serverId 分开存 localStorage。 */
const CONSOLE_HISTORY_MAX = 20;
const CONSOLE_QUICK = ["list", "save-all", "stop", "whitelist list", "tps", "seed"];

/** 把「以文本编辑」里的 key=value 解析成 patch（注释与空行忽略，未识别的键交给后端原样保留）。 */
function parseProperties(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const eq = trimmed.indexOf("=");
    if (eq <= 0) continue;
    out[trimmed.slice(0, eq).trim()] = trimmed.slice(eq + 1).trim();
  }
  return out;
}

/** 服务端机箱图标：和多人页的卡片同一个形状。 */
function ServerGlyph({ size = 34 }: { size?: number }) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" aria-hidden="true">
      <rect x="3" y="4" width="18" height="7" rx="2" />
      <rect x="3" y="13" width="18" height="7" rx="2" />
      <circle cx="7" cy="7.5" r="0.9" fill="currentColor" stroke="none" />
      <circle cx="7" cy="16.5" r="0.9" fill="currentColor" stroke="none" />
      <path d="M11 7.5h6M11 16.5h6" />
    </svg>
  );
}

/** 服务端详情：和实例详情同构（顶部信息 + 五个 tab）。 */
export function ServerDetailPage({ server, onBack, notify, onOpenContent, onDeleted }: Props) {
  const t = useT();
  const [tab, setTab] = useState<ServerTab>("mods");
  const [state, setState] = useState<ServerState>(server.state);
  const [players, setPlayers] = useState(server.players ?? 0);
  const [busy, setBusy] = useState(false);
  const [pendingDelete, setPendingDelete] = useState(false);

  const [mods, setMods] = useState<ServerContentItem[] | null>(null);
  const [plugins, setPlugins] = useState<ServerContentItem[] | null>(null);
  const [saves, setSaves] = useState<ServerSaveItem[] | null>(null);
  const [config, setConfig] = useState<ServerConfigResult | null>(null);
  const [configFields, setConfigFields] = useState<Record<string, string>>({});
  const [configText, setConfigText] = useState("");
  const [textMode, setTextMode] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const logRef = useRef<HTMLPreElement | null>(null);

  // 控制台：能否输入由 server_console_state 决定（事件推来的运行状态也会刷新它）
  const [consoleState, setConsoleState] = useState<ServerConsoleState | null>(null);
  const [consoleCommand, setConsoleCommand] = useState("");
  const [consoleHint, setConsoleHint] = useState<{ text: string; error: boolean } | null>(null);
  const [history, setHistory] = useState<string[]>([]);
  // -1 = 不在翻历史（正在编辑新指令）
  const [historyIndex, setHistoryIndex] = useState(-1);
  const historyKey = "gugu.server.console.history." + server.id;

  const stateLabel = useMemo(() => {
    if (state === "running") return t("server.state.running", { count: players });
    if (state === "starting") return t("server.state.starting");
    if (state === "stopping") return t("server.state.stopping");
    return t("server.state.stopped");
  }, [state, players, t]);

  // 状态与日志都走事件总线（和多人页卡片同一套）
  useEffect(() => {
    let off: (() => void) | null = null;
    let alive = true;
    void subscribeLauncherEvents((event) => {
      if (event.kind === "server-state" && event.payload.id === server.id) {
        setState(event.payload.state);
        if (typeof event.payload.players === "number") setPlayers(event.payload.players);
      }
      if (event.kind === "server-log" && event.payload.id === server.id) {
        setLogs((prev) => [...prev, event.payload.line].slice(-MAX_LOG_LINES));
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
  }, [server.id]);

  const loadContent = useCallback(async () => {
    try {
      const [modList, pluginList] = await Promise.all([
        api.serverContentList({ serverId: server.id, kind: "mod" }),
        api.serverContentList({ serverId: server.id, kind: "plugin" }),
      ]);
      setMods(modList);
      setPlugins(pluginList);
    } catch (error) {
      notify(String(error));
    }
  }, [notify, server.id]);

  const loadSaves = useCallback(async () => {
    try {
      setSaves(await api.serverSavesList({ serverId: server.id }));
    } catch (error) {
      notify(String(error));
    }
  }, [notify, server.id]);

  const loadConfig = useCallback(async () => {
    try {
      const result = await api.serverConfigRead({ serverId: server.id });
      setConfig(result);
      setConfigFields(result.fields);
      setConfigText(result.text);
    } catch (error) {
      notify(String(error));
    }
  }, [notify, server.id]);

  useEffect(() => {
    try {
      const raw = window.localStorage.getItem(historyKey);
      const parsed = raw ? (JSON.parse(raw) as unknown) : [];
      if (Array.isArray(parsed)) setHistory(parsed.filter((item) => typeof item === "string").slice(-CONSOLE_HISTORY_MAX));
    } catch {
      /* 坏数据当没有历史 */
    }
  }, [historyKey]);

  useEffect(() => {
    void api
      .serverConsoleState({ serverId: server.id })
      .then(setConsoleState)
      .catch(() => setConsoleState(null));
  }, [server.id, state]);

  useEffect(() => {
    void loadContent();
    void loadSaves();
    void loadConfig();
    void api
      .serverLogsTail({ serverId: server.id, lines: 400 })
      .then((lines) => setLogs(lines.slice(-MAX_LOG_LINES)))
      .catch(() => undefined);
  }, [loadConfig, loadContent, loadSaves, server.id]);

  useEffect(() => {
    const node = logRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [logs, tab]);

  const toggleRun = async () => {
    setBusy(true);
    try {
      if (state === "running" || state === "starting") await api.serverStop(server.id);
      else await api.serverStart(server.id);
    } catch (error) {
      notify(String(error));
    } finally {
      setBusy(false);
    }
  };

  const toggleFile = async (kind: "mod" | "plugin", item: ServerContentItem) => {
    try {
      await api.serverContentToggle({ serverId: server.id, kind, fileName: item.fileName, enabled: !item.enabled });
      await loadContent();
    } catch (error) {
      notify(String(error));
    }
  };

  /** 能不能敲指令：优先后端给的 stdinOpen，取不到就看运行状态。 */
  const canInput = consoleState ? consoleState.running && consoleState.stdinOpen : state === "running";

  const pushHistory = (command: string) => {
    setHistory((prev) => {
      const next = [...prev.filter((item) => item !== command), command].slice(-CONSOLE_HISTORY_MAX);
      try {
        window.localStorage.setItem(historyKey, JSON.stringify(next));
      } catch {
        /* 隐私模式写不进去就只在内存里留 */
      }
      return next;
    });
  };

  const sendCommand = async () => {
    const command = consoleCommand.trim().replace(/^\//, "");
    if (!command) return;
    try {
      await api.serverSendCommand({ serverId: server.id, command });
      pushHistory(command);
      setConsoleCommand("");
      setHistoryIndex(-1);
      setConsoleHint({ text: t("server.console.sent", { command }), error: false });
      // 回显行由界面补：后端只推执行结果，这里保证「> 指令」一定有一行且能高亮
      setLogs((prev) => [...prev, "> " + command].slice(-MAX_LOG_LINES));
    } catch (error) {
      setConsoleHint({ text: error instanceof Error ? error.message : String(error), error: true });
    }
  };

  /** 上/下箭头翻历史：-1 表示回到正在编辑的这一行。 */
  const recallHistory = (delta: number) => {
    if (history.length === 0) return;
    const next = historyIndex === -1 ? (delta < 0 ? history.length - 1 : -1) : Math.min(history.length - 1, Math.max(-1, historyIndex + delta));
    setHistoryIndex(next);
    setConsoleCommand(next === -1 ? "" : (history[next] ?? ""));
  };

  const copyLogs = async () => {
    try {
      await navigator.clipboard.writeText(logs.join("\n"));
      notify(t("server.logCopied"));
    } catch {
      notify(t("server.logCopyFailed"));
    }
  };

  const saveConfig = async () => {
    try {
      // 表单模式发整份常用项；文本模式先解析成 key=value 再写回
      const patch = textMode ? parseProperties(configText) : configFields;
      await api.serverConfigWrite({ serverId: server.id, patch });
      await loadConfig();
      notify(t("server.detail.configSaved"));
    } catch (error) {
      notify(String(error));
    }
  };

  const backupSave = async (save: ServerSaveItem) => {
    try {
      const result = await api.serverSavesBackup({ serverId: server.id, name: save.name });
      notify(t("server.detail.backedUp", { path: result.path }));
    } catch (error) {
      notify(String(error));
    }
  };

  const removeServer = async () => {
    try {
      await api.serverDelete(server.id);
      setPendingDelete(false);
      notify(t("server.deleted", { name: server.name }));
      onDeleted();
    } catch (error) {
      notify(String(error));
    }
  };

  const fileRow = (kind: "mod" | "plugin", item: ServerContentItem) => (
    <div className="srv-row" key={item.fileName} data-server-file={item.fileName} data-server-file-enabled={item.enabled ? "1" : "0"}>
      <span className="srv-row__name" title={item.fileName}>
        {item.fileName}
      </span>
      <span className="srv-row__meta">{formatBytes(item.sizeBytes)}</span>
      <span className="srv-row__meta">
        {item.provider ? <span className={"source-badge source-badge--" + item.provider}>{t("content.source." + item.provider)}</span> : "—"}
      </span>
      <label className="switch switch--mini">
        <input
          type="checkbox"
          checked={item.enabled}
          data-server-file-toggle={item.fileName}
          aria-label={item.fileName}
          onChange={() => void toggleFile(kind, item)}
        />
        <span className="switch__track" aria-hidden="true">
          <span className="switch__dot" />
        </span>
      </label>
    </div>
  );

  return (
    <div className="detail server-detail" data-server-detail={server.id}>
      <section className="card detail-head">
        <button className="icon-button detail-head__back" title={t("server.detail.back")} aria-label={t("server.detail.back")} onClick={onBack}>
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M15 5l-7 7 7 7" />
          </svg>
        </button>
        <span className="detail-head__icon server-detail__glyph" aria-hidden="true">
          <ServerGlyph size={40} />
        </span>
        <div className="detail-head__meta">
          <div className="detail-head__name" data-server-name>
            {server.name}
          </div>
          <div className="instance-card__version">
            <span className="version-badge" data-server-core>
              {t("server.core." + server.core)}
            </span>
            <span className="loader-badge" data-server-version>
              {server.mcVersion}
            </span>
            <span className="row__hint" data-server-port>
              {t("server.port", { port: server.port })}
            </span>
            <span className="row__hint" data-server-memory>
              {t("server.memory", { mb: server.memoryMb })}
            </span>
            <span className={"server-state server-state--" + state} data-server-state-label>
              {stateLabel}
            </span>
          </div>
        </div>
        <button type="button" className="launch-button detail-head__launch" data-action="server-detail-toggle" disabled={busy} onClick={() => void toggleRun()}>
          <span className="play-triangle" aria-hidden="true" />
          {state === "running" || state === "starting" ? t("server.stop") : t("server.start")}
        </button>
        <button
          type="button"
          className="pill-outline detail-head__export"
          data-action="server-detail-folder"
          onClick={() => void api.serverOpenFolder(server.id).catch((error) => notify(String(error)))}
        >
          <img src={icons.folder} alt="" />
          {t("server.openFolder")}
        </button>
        <button type="button" className="icon-button icon-button--danger" data-action="server-detail-delete" title={t("common.delete")} onClick={() => setPendingDelete(true)}>
          <img src={icons.trash} alt={t("common.delete")} />
        </button>
      </section>

      <nav className="detail-tabs" role="tablist">
        {TABS.map((item) => (
          <button
            key={item.key}
            type="button"
            role="tab"
            aria-selected={tab === item.key}
            className={"detail-tab" + (tab === item.key ? " is-active" : "")}
            data-server-tab={item.key}
            onClick={() => setTab(item.key)}
          >
            {t(item.labelKey)}
          </button>
        ))}
      </nav>

      {tab === "mods" || tab === "plugins" ? (
        <section className="card srv-panel" data-server-panel={tab}>
          <div className="srv-toolbar">
            <span className="detail-card__title">{t(tab === "mods" ? "server.detail.mods" : "server.detail.plugins")}</span>
            <div className="srv-toolbar__actions">
              <button type="button" className="pill-outline" data-action="server-install-content" onClick={onOpenContent}>
                {t("server.detail.installContent")}
              </button>
              <button
                type="button"
                className="pill-outline"
                data-action="server-open-content-folder"
                onClick={() => void api.serverOpenFolder(server.id).catch((error) => notify(String(error)))}
              >
                <img src={icons.folder} alt="" />
                {t("server.openFolder")}
              </button>
            </div>
          </div>
          {(tab === "mods" ? mods : plugins) === null ? (
            <div className="row__hint">{t("common.loading")}</div>
          ) : (tab === "mods" ? mods : plugins)!.length === 0 ? (
            <div className="empty-hint" data-server-files-empty>
              {t("server.detail.empty")}
            </div>
          ) : (
            <div className="srv-rows">
              <div className="srv-row srv-row--head">
                <span>{t("common.name")}</span>
                <span>{t("server.detail.size")}</span>
                <span>{t("content.sourceLabel")}</span>
                <span>{t("server.detail.switchOn")}</span>
              </div>
              {(tab === "mods" ? mods! : plugins!).map((item) => fileRow(tab === "mods" ? "mod" : "plugin", item))}
            </div>
          )}
        </section>
      ) : null}

      {tab === "saves" ? (
        <section className="card srv-panel" data-server-panel="saves">
          <div className="srv-toolbar">
            <span className="detail-card__title">{t("server.detail.saves")}</span>
            <button type="button" className="pill-outline" onClick={() => void api.serverOpenFolder(server.id).catch((error) => notify(String(error)))}>
              <img src={icons.folder} alt="" />
              {t("server.openFolder")}
            </button>
          </div>
          {saves === null ? (
            <div className="row__hint">{t("common.loading")}</div>
          ) : saves.length === 0 ? (
            <div className="empty-hint" data-server-saves-empty>
              {t("server.detail.empty")}
            </div>
          ) : (
            <div className="srv-rows">
              <div className="srv-row srv-row--head">
                <span>{t("common.name")}</span>
                <span>{t("server.detail.size")}</span>
                <span>{t("server.detail.modified")}</span>
                <span />
              </div>
              {saves.map((save) => (
                <div className="srv-row" key={save.name} data-server-save={save.name}>
                  <span className="srv-row__name">{save.name}</span>
                  <span className="srv-row__meta">{formatBytes(save.sizeBytes)}</span>
                  <span className="srv-row__meta">{save.updatedAt.replace("T", " ").slice(0, 16)}</span>
                  <span className="srv-row__actions">
                    <button type="button" className="pill-outline pill-outline--tiny" data-action="server-save-backup" onClick={() => void backupSave(save)}>
                      {t("server.detail.saveBackup")}
                    </button>
                  </span>
                </div>
              ))}
            </div>
          )}
        </section>
      ) : null}

      {tab === "config" ? (
        <section className="card srv-panel" data-server-panel="config">
          <div className="srv-toolbar">
            <span className="detail-card__title">{t("server.detail.config")}</span>
            <div className="srv-toolbar__actions">
              <button type="button" className="pill-outline" data-action="server-config-mode" onClick={() => setTextMode((mode) => !mode)}>
                {textMode ? t("server.detail.configForm") : t("server.detail.configText")}
              </button>
              <button type="button" className="pill-button" data-action="server-config-save" onClick={() => void saveConfig()}>
                {t("server.detail.configSave")}
              </button>
            </div>
          </div>
          {config === null ? (
            <div className="row__hint">{t("common.loading")}</div>
          ) : textMode ? (
            <textarea
              className="srv-config__text mono"
              data-server-config-text
              value={configText}
              spellCheck={false}
              onChange={(event) => setConfigText(event.target.value)}
            />
          ) : (
            <div className="srv-config" data-server-config-form>
              {CONFIG_FIELDS.map((field) => {
                const value = configFields[field.key] ?? "";
                return (
                  <div className="field" key={field.key}>
                    <label htmlFor={"cfg-" + field.key}>{field.label}</label>
                    {field.type === "bool" ? (
                      <label className="switch switch--mini">
                        <input
                          id={"cfg-" + field.key}
                          type="checkbox"
                          data-server-config-field={field.key}
                          checked={value === "true"}
                          onChange={(event) => setConfigFields((prev) => ({ ...prev, [field.key]: event.target.checked ? "true" : "false" }))}
                        />
                        <span className="switch__track" aria-hidden="true">
                          <span className="switch__dot" />
                        </span>
                      </label>
                    ) : field.type === "select" ? (
                      <select
                        id={"cfg-" + field.key}
                        data-server-config-field={field.key}
                        value={value}
                        onChange={(event) => setConfigFields((prev) => ({ ...prev, [field.key]: event.target.value }))}
                      >
                        {(field.options ?? []).map((option) => (
                          <option key={option} value={option}>
                            {option}
                          </option>
                        ))}
                      </select>
                    ) : (
                      <input
                        id={"cfg-" + field.key}
                        type={field.type === "number" ? "number" : "text"}
                        data-server-config-field={field.key}
                        value={value}
                        onChange={(event) => setConfigFields((prev) => ({ ...prev, [field.key]: event.target.value }))}
                      />
                    )}
                  </div>
                );
              })}
            </div>
          )}
          <div className="row__hint">{t("server.detail.configHint")}</div>
        </section>
      ) : null}

      {tab === "logs" ? (
        <section className="card srv-panel" data-server-panel="logs">
          <div className="srv-toolbar">
            <span className="detail-card__title">{t("server.detail.logs")}</span>
            <div className="srv-toolbar__actions">
              <button type="button" className="pill-outline" data-action="server-log-copy-2" onClick={() => void copyLogs()}>
                {t("server.logCopy")}
              </button>
              <button type="button" className="pill-outline" data-action="server-log-clear" onClick={() => setLogs([])}>
                {t("server.detail.logsClear")}
              </button>
            </div>
          </div>
          <div className="srv-console" data-server-console>
            <div className="srv-console__row">
              <span className="srv-console__prompt" aria-hidden="true">
                &gt;
              </span>
              <input
                type="text"
                className="srv-console__input mono"
                data-server-console-input
                placeholder={t("server.console.placeholder")}
                value={consoleCommand}
                disabled={!canInput}
                aria-label={t("server.console.placeholder")}
                onChange={(event) => {
                  setConsoleCommand(event.target.value);
                  setHistoryIndex(-1);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    void sendCommand();
                  } else if (event.key === "ArrowUp") {
                    event.preventDefault();
                    recallHistory(-1);
                  } else if (event.key === "ArrowDown") {
                    event.preventDefault();
                    recallHistory(1);
                  }
                }}
              />
              <button
                type="button"
                className="pill-button"
                data-action="server-console-send"
                disabled={!canInput}
                onClick={() => void sendCommand()}
              >
                {t("server.console.send")}
              </button>
            </div>
            <div className="chip-row srv-console__quick">
              {CONSOLE_QUICK.map((item) => (
                <button
                  key={item}
                  type="button"
                  className="chip chip--button"
                  data-console-quick={item}
                  disabled={!canInput}
                  onClick={() => {
                    setConsoleCommand(item);
                    setHistoryIndex(-1);
                  }}
                >
                  {item}
                </button>
              ))}
            </div>
            {consoleHint?.error ? (
              <div className="field-error" data-server-console-error>
                {consoleHint.text}
              </div>
            ) : !canInput ? (
              <div className="row__hint" data-server-console-offline>
                {t("server.console.offline")}
              </div>
            ) : (
              <div className="row__hint" data-server-console-hint>
                {consoleHint ? consoleHint.text : t("server.console.historyHint")}
              </div>
            )}
          </div>

          {logs.length === 0 ? (
            <div className="empty-hint" data-server-logs-empty>
              {t("server.logEmpty")}
            </div>
          ) : (
            <pre className="log-view srv-logs" ref={logRef} data-server-logs>
              {logs.map((line, index) => (
                <div
                  className={"log-line" + (line.startsWith("> ") ? " log-line--input" : "")}
                  key={index}
                  data-console-echo={line.startsWith("> ") ? "1" : undefined}
                >
                  {line}
                </div>
              ))}
            </pre>
          )}
        </section>
      ) : null}

      {pendingDelete ? (
        <Modal
          title={t("server.deleteTitle")}
          onClose={() => setPendingDelete(false)}
          footer={
            <>
              <button type="button" className="pill-outline" onClick={() => setPendingDelete(false)}>
                {t("common.cancel")}
              </button>
              <button type="button" className="pill-button" data-action="server-detail-delete-confirm" onClick={() => void removeServer()}>
                {t("common.delete")}
              </button>
            </>
          }
        >
          <p>{t("server.deleteConfirm", { name: server.name })}</p>
        </Modal>
      ) : null}
    </div>
  );
}
