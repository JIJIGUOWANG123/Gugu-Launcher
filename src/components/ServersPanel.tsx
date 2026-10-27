import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { icons } from "../assets/icons";
import { formatBytes } from "../lib/format";
import { useDownloads } from "../lib/downloads-store";
import { useT } from "../lib/i18n";
import { api, subscribeLauncherEvents } from "../lib/tauri-api";
import type { ServerCore, ServerCreateRequest, ServerInstance, ServerVersion } from "../lib/types";
import { Modal } from "./Modal";
import { ServerDetailPage } from "../pages/ServerDetailPage";

interface Props {
  notify: (message: string) => void;
  /** 服务端详情里「从资源页安装」要跳到资源页 */
  onOpenContent: () => void;
}

const MAX_LOG_LINES = 500;
const QUICK_VERSIONS = ["1.20.1", "1.19.4", "1.18.2"];
const EULA_URL = "https://aka.ms/MinecraftEULA";
const FASTMIRROR_URL = "https://www.fastmirror.net/#/download";

/** 服务器机箱图标（内联 SVG，装饰用）。 */
function ServerGlyph({ size = 72 }: { size?: number }) {
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

/**
 * 多人 → 服务器：空态 / 卡片列表 / 展开日志 / 删除二次确认 / 创建服务器弹窗。
 * 状态与日志都来自 server://state、server://log 事件（mock 里同一套事件）。
 */
export function ServersPanel({ notify, onOpenContent }: Props) {
  const t = useT();
  const { refresh: refreshDownloads } = useDownloads();
  const [servers, setServers] = useState<ServerInstance[] | null>(null);
  const [logs, setLogs] = useState<Record<string, string[]>>({});
  const [expanded, setExpanded] = useState<string | null>(null);
  const [pendingDelete, setPendingDelete] = useState<ServerInstance | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  // 点卡片进服务端详情（和实例详情的进出方式一致）
  const [detail, setDetail] = useState<ServerInstance | null>(null);
  const logRefs = useRef<Record<string, HTMLPreElement | null>>({});

  const reload = useCallback(async () => {
    try {
      setServers(await api.serverList());
    } catch (error) {
      notify(String(error));
    }
  }, [notify]);

  useEffect(() => {
    void reload();
  }, [reload]);

  useEffect(() => {
    let off: (() => void) | null = null;
    let alive = true;
    void subscribeLauncherEvents((event) => {
      if (event.kind === "server-log") {
        const { id, line } = event.payload;
        setLogs((prev) => ({ ...prev, [id]: [...(prev[id] ?? []), line].slice(-MAX_LOG_LINES) }));
        return;
      }
      if (event.kind === "server-state") {
        const { id, state, players } = event.payload;
        setServers((prev) =>
          (prev ?? []).map((server) => (server.id === id ? { ...server, state, players: players ?? server.players } : server)),
        );
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

  useEffect(() => {
    if (!expanded) return;
    const node = logRefs.current[expanded];
    if (node) node.scrollTop = node.scrollHeight;
  }, [logs, expanded]);

  const stateLabel = (server: ServerInstance) => {
    if (server.state === "running") return t("server.state.running", { count: server.players ?? 0 });
    if (server.state === "starting") return t("server.state.starting");
    if (server.state === "stopping") return t("server.state.stopping");
    return t("server.state.stopped");
  };

  const toggleRun = async (server: ServerInstance) => {
    try {
      if (server.state === "running" || server.state === "starting") {
        await api.serverStop(server.id);
        return;
      }
      await api.serverStart(server.id);
      setExpanded(server.id);
    } catch (error) {
      notify(String(error));
    }
  };

  const remove = async () => {
    if (!pendingDelete) return;
    const target = pendingDelete;
    setPendingDelete(null);
    try {
      await api.serverDelete(target.id);
      await reload();
      notify(t("server.deleted", { name: target.name }));
    } catch (error) {
      notify(String(error));
    }
  };

  const copyLog = async (id: string) => {
    try {
      await navigator.clipboard.writeText((logs[id] ?? []).join("\n"));
      notify(t("server.logCopied"));
    } catch {
      notify(t("server.logCopyFailed"));
    }
  };

  const list = servers ?? [];

  if (detail) {
    return (
      <ServerDetailPage
        server={detail}
        onBack={() => {
          setDetail(null);
          void reload();
        }}
        notify={notify}
        onOpenContent={onOpenContent}
        onDeleted={() => {
          setDetail(null);
          void reload();
        }}
      />
    );
  }

  return (
    <div className="servers-panel">
      <div className="servers-panel__head">
        <span className="row__hint">{t("server.count", { count: list.length })}</span>
        <button type="button" className="pill-button" data-action="server-create-open" onClick={() => setCreateOpen(true)}>
          + {t("server.create")}
        </button>
      </div>

      {servers === null ? (
        <div className="row__hint">{t("common.loading")}</div>
      ) : list.length === 0 ? (
        <div className="server-empty" data-server-empty>
          <span className="server-empty__glyph">
            <ServerGlyph />
          </span>
          <div className="server-empty__title">{t("server.empty")}</div>
          <button type="button" className="pill-button" data-action="server-create-first" onClick={() => setCreateOpen(true)}>
            {t("server.createFirst")}
          </button>
        </div>
      ) : (
        <div className="server-list">
          {list.map((server) => {
            const open = expanded === server.id;
            return (
              <article className="card server-card" key={server.id} data-server={server.id} data-server-state={server.state}>
                <div
                  className="server-card__row"
                  data-action="server-open-detail"
                  onClick={(event) => {
                    // 快捷按钮自己处理点击，别顺带进详情
                    if ((event.target as HTMLElement).closest("button")) return;
                    setDetail(server);
                  }}
                >
                  <span className="server-card__glyph" aria-hidden="true">
                    <ServerGlyph size={34} />
                  </span>
                  <div className="server-card__meta">
                    <div className="server-card__title">
                      <span className="server-card__name">{server.name}</span>
                      <span className="chip chip--tiny" data-server-core={server.core}>
                        {t("server.core." + server.core)}
                      </span>
                      <span className="chip chip--tiny">{server.mcVersion}</span>
                    </div>
                    <div className="server-card__sub">
                      <span>{t("server.port", { port: server.port })}</span>
                      <span>{t("server.memory", { mb: server.memoryMb })}</span>
                      <span className={"server-state server-state--" + server.state} data-server-state-label>
                        {stateLabel(server)}
                      </span>
                    </div>
                  </div>
                  <div className="server-card__actions">
                    <button
                      type="button"
                      className="pill-outline pill-outline--tiny"
                      data-action="server-toggle"
                      onClick={() => void toggleRun(server)}
                    >
                      {server.state === "running" || server.state === "starting" ? t("server.stop") : t("server.start")}
                    </button>
                    <button
                      type="button"
                      className="pill-outline pill-outline--tiny"
                      data-action="server-open-folder"
                      onClick={() => void api.serverOpenFolder(server.id).catch((error) => notify(String(error)))}
                    >
                      {t("server.openFolder")}
                    </button>
                    <button
                      type="button"
                      className="pill-outline pill-outline--tiny"
                      data-action="server-log"
                      onClick={() => setExpanded(open ? null : server.id)}
                    >
                      {t("server.log")}
                    </button>
                    <button
                      type="button"
                      className="icon-button icon-button--danger"
                      data-action="server-delete"
                      title={t("common.delete")}
                      onClick={() => setPendingDelete(server)}
                    >
                      <img src={icons.trash} alt={t("common.delete")} />
                    </button>
                  </div>
                </div>

                {open ? (
                  <div className="server-log" data-server-log>
                    <div className="server-log__head">
                      <span className="row__hint">
                        {t("server.logHint")} · {(logs[server.id] ?? []).length}
                      </span>
                      <button type="button" className="pill-outline pill-outline--tiny" data-action="server-log-copy" onClick={() => void copyLog(server.id)}>
                        {t("server.logCopy")}
                      </button>
                    </div>
                    {(logs[server.id] ?? []).length === 0 ? (
                      <div className="empty-hint">{t("server.logEmpty")}</div>
                    ) : (
                      <pre
                        className="log-view server-log__view"
                        ref={(node) => {
                          logRefs.current[server.id] = node;
                        }}
                      >
                        {(logs[server.id] ?? []).map((line, index) => (
                          <div className="log-line" key={index}>
                            {line}
                          </div>
                        ))}
                      </pre>
                    )}
                  </div>
                ) : null}
              </article>
            );
          })}
        </div>
      )}

      {createOpen ? (
        <CreateServerModal
          defaultName={t("server.defaultName", { index: list.length + 1 })}
          onClose={() => setCreateOpen(false)}
          onCreated={async (name) => {
            setCreateOpen(false);
            await reload();
            refreshDownloads();
            notify(t("server.created", { name }));
          }}
          notify={notify}
        />
      ) : null}

      {pendingDelete ? (
        <Modal
          title={t("server.deleteTitle")}
          onClose={() => setPendingDelete(null)}
          footer={
            <>
              <button type="button" className="pill-outline" onClick={() => setPendingDelete(null)}>
                {t("common.cancel")}
              </button>
              <button type="button" className="pill-button" data-action="server-delete-confirm" onClick={() => void remove()}>
                {t("common.delete")}
              </button>
            </>
          }
        >
          <p>{t("server.deleteConfirm", { name: pendingDelete.name })}</p>
        </Modal>
      ) : null}
    </div>
  );
}

interface CreateProps {
  defaultName: string;
  onClose: () => void;
  onCreated: (name: string) => void | Promise<void>;
  notify: (message: string) => void;
}

/** 创建服务器弹窗：名称 / 核心 / 版本 / 内存 / 端口 / EULA。 */
export function CreateServerModal({ defaultName, onClose, onCreated, notify }: CreateProps) {
  const t = useT();
  const [name, setName] = useState(defaultName);
  const [cores, setCores] = useState<ServerCore[]>([]);
  const [core, setCore] = useState("paper");
  const [versions, setVersions] = useState<ServerVersion[]>([]);
  const [version, setVersion] = useState("1.20.1");
  const [memory, setMemory] = useState(2048);
  const [port, setPort] = useState("25565");
  const [eula, setEula] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void api.serverCores().then(setCores).catch(() => setCores([]));
  }, []);

  useEffect(() => {
    let alive = true;
    void api
      .serverVersions({ core })
      .then((list) => {
        if (!alive) return;
        setVersions(list);
        if (list.length > 0 && !list.some((item) => item.version === version)) setVersion(list[0].version);
      })
      .catch(() => {
        if (alive) setVersions([]);
      });
    return () => {
      alive = false;
    };
  }, [core]);

  const chosen = useMemo(() => versions.find((item) => item.version === version) ?? null, [versions, version]);

  const submit = async () => {
    if (!eula) {
      setError(t("server.needEula"));
      return;
    }
    setBusy(true);
    setError("");
    try {
      const payload: ServerCreateRequest = {
        name: name.trim() || defaultName,
        core,
        mcVersion: version,
        memoryMb: memory,
        port: Number(port) || 25565,
        eula,
      };
      await api.serverCreate(payload);
      await onCreated(payload.name);
    } catch (err) {
      setError(String(err instanceof Error ? err.message : err));
    } finally {
      setBusy(false);
    }
  };

  const openExternal = async (url: string) => {
    try {
      await api.openExternal(url);
    } catch (err) {
      notify(String(err));
    }
  };

  return (
    <Modal
      title={t("server.createTitle")}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="pill-outline" onClick={onClose}>
            {t("common.cancel")}
          </button>
          <button type="button" className="pill-button" data-action="server-create-submit" disabled={!eula || busy} onClick={() => void submit()}>
            {busy ? t("server.creating") : t("server.createSubmit")}
          </button>
        </>
      }
    >
      <div className="field">
        <label htmlFor="server-name">{t("server.name")}</label>
        <input
          id="server-name"
          type="text"
          data-server-name
          value={name}
          onChange={(event) => setName(event.target.value)}
        />
      </div>

      <div className="field">
        <label>{t("server.coreLabel")}</label>
        <div className="server-core-picker" role="radiogroup" aria-label={t("server.coreLabel")}>
          {cores.map((item) => (
            <button
              key={item.id}
              type="button"
              role="radio"
              aria-checked={core === item.id}
              className={"server-core" + (core === item.id ? " is-active" : "")}
              data-server-core-option={item.id}
              onClick={() => setCore(item.id)}
            >
              <span className="server-core__name">{item.name}</span>
              {item.latestVersion ? <span className="server-core__ver">{item.latestVersion}</span> : null}
            </button>
          ))}
        </div>
      </div>

      <div className="field">
        <label htmlFor="server-version">{t("server.version")}</label>
        <div className="server-quick">
          {QUICK_VERSIONS.map((item) => (
            <button
              key={item}
              type="button"
              className={"chip chip--button" + (version === item ? " is-active" : "")}
              data-server-quick={item}
              onClick={() => {
                setVersion(item);
                setError("");
              }}
            >
              {item}
            </button>
          ))}
        </div>
        <select id="server-version" data-server-version value={version} onChange={(event) => setVersion(event.target.value)}>
          {versions.length === 0 ? <option value={version}>{version}</option> : null}
          {versions.map((item) => (
            <option key={item.version + (item.build ?? "")} value={item.version}>
              {item.version}
              {item.build ? " · build " + item.build : ""}
              {item.sizeBytes ? " · " + formatBytes(item.sizeBytes) : ""}
            </option>
          ))}
        </select>
      </div>

      <div className="field">
        <label htmlFor="server-memory">
          {t("server.memoryLabel")}
          <span className="field__value">{memory} MB</span>
        </label>
        <input
          id="server-memory"
          type="range"
          min={1024}
          max={8192}
          step={512}
          value={memory}
          data-server-memory
          onChange={(event) => setMemory(Number(event.target.value))}
        />
      </div>

      <div className="field">
        <label htmlFor="server-port">{t("server.portLabel")}</label>
        <input
          id="server-port"
          type="text"
          inputMode="numeric"
          value={port}
          data-server-port
          onChange={(event) => setPort(event.target.value.replace(/[^0-9]/g, ""))}
        />
        <div className="row__hint" data-server-port-hint>
          {t("server.portHint")}
        </div>
      </div>

      <label className="server-eula">
        <input type="checkbox" checked={eula} data-server-eula onChange={(event) => setEula(event.target.checked)} />
        <span>
          {t("server.eula")}
          <button type="button" className="link-button" data-action="server-eula-link" onClick={() => void openExternal(EULA_URL)}>
            {t("server.eulaLink")}
          </button>
        </span>
      </label>

      {error ? (
        <div className="field-error" data-server-error>
          {error}
        </div>
      ) : null}

      {chosen ? <div className="row__hint">{t("server.sizeHint", { size: formatBytes(chosen.sizeBytes ?? 0) })}</div> : null}

      <button type="button" className="link-button" data-action="server-fastmirror" onClick={() => void openExternal(FASTMIRROR_URL)}>
        {t("server.fastmirror")}
      </button>
    </Modal>
  );
}
