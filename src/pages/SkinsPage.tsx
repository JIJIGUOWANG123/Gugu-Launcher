import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { icons } from "../assets/icons";
import { type SkinAction, SkinViewer3D } from "../components/SkinViewer3D";
import { Modal } from "../components/Modal";
import { TextureCrop } from "../components/TextureCrop";
import { formatBytes } from "../lib/format";
import { useT } from "../lib/i18n";
import { fileToDataUrl, isSkinSize, readImageSize, stripDataUrlPrefix } from "../lib/image-file";
import { api } from "../lib/tauri-api";
import type { CapeItem, Settings, SkinItem, SkinLookupResult, SkinModel } from "../lib/types";

interface Props {
  notify: (message: string) => void;
  settings: Settings | null;
  onChangeSettings: (patch: Partial<Settings>) => void;
}

const ACTIONS: Array<{ key: SkinAction; labelKey: string }> = [
  { key: "idle", labelKey: "skin.action.idle" },
  { key: "walk", labelKey: "skin.action.walk" },
  { key: "run", labelKey: "skin.action.run" },
  { key: "none", labelKey: "skin.action.none" },
];

/** 皮肤页：左边 3D 预览，右边已保存皮肤列表，下面披风。 */
export function SkinsPage({ notify, settings, onChangeSettings }: Props) {
  const t = useT();
  const [skins, setSkins] = useState<SkinItem[] | null>(null);
  const [capes, setCapes] = useState<CapeItem[] | null>(null);
  const [currentPath, setCurrentPath] = useState("");
  const [currentCape, setCurrentCape] = useState("");
  const [preview, setPreview] = useState<SkinItem | null>(null);
  const [previewCape, setPreviewCape] = useState<CapeItem | null>(null);
  const [model, setModel] = useState<SkinModel>("default");
  const [action, setAction] = useState<SkinAction>("idle");
  const [autoRotate, setAutoRotate] = useState(false);
  const [dark, setDark] = useState(false);
  const [resetToken, setResetToken] = useState(0);
  const [busy, setBusy] = useState(false);
  const [dragOver, setDragOver] = useState(false);
  const [tab, setTab] = useState<"saved" | "official">("saved");
  const fileRef = useRef<HTMLInputElement | null>(null);

  // 内置 Steve / Alex（列表前两项，不落盘、不可删）
  const [builtin, setBuiltin] = useState<{ classic: SkinItem; slim: SkinItem } | null>(null);
  // 从正版账号获取
  const [mojangOpen, setMojangOpen] = useState(false);
  const [mojangName, setMojangName] = useState("");
  const [mojangBusy, setMojangBusy] = useState(false);
  const [mojangError, setMojangError] = useState("");
  const [mojangResult, setMojangResult] = useState<SkinItem | null>(null);

  // 从皮肤站获取（只预览，不登录）
  const [tpServer, setTpServer] = useState(settings?.thirdpartyAuthServer ?? "");
  const [tpName, setTpName] = useState("");
  const [tpBusy, setTpBusy] = useState(false);
  const [tpSaving, setTpSaving] = useState(false);
  const [tpError, setTpError] = useState("");
  const [tpResult, setTpResult] = useState<SkinLookupResult | null>(null);

  useEffect(() => {
    if (!tpServer && settings?.thirdpartyAuthServer) setTpServer(settings.thirdpartyAuthServer);
  }, [settings?.thirdpartyAuthServer, tpServer]);

  const load = useCallback(async () => {
    try {
      const [list, current, capeList, capeCurrent, classic, slim] = await Promise.all([
        api.skinsList(),
        api.skinCurrent(),
        api.capesList(),
        api.capeCurrent(),
        api.skinDefault("default"),
        api.skinDefault("slim"),
      ]);
      setSkins(list);
      setCapes(capeList);
      setCurrentPath(current);
      setCurrentCape(capeCurrent);
      setBuiltin({ classic: classic, slim: slim });
      // 没选皮肤（或选的那张已经不在了）→ 直接用内置 Steve，不留空白
      const active = list.find((item) => item.path === current) ?? classic;
      setPreview((prev) => {
        if (!prev) return active;
        if (prev.builtin) return prev;
        return list.find((item) => item.path === prev.path) ?? active;
      });
      setPreviewCape(capeList.find((cape) => cape.name === capeCurrent) ?? null);
    } catch (error) {
      notify(String(error));
    }
  }, [notify]);

  useEffect(() => {
    void load();
  }, [load]);

  const currentSkin = useMemo(() => (skins ?? []).find((item) => item.path === currentPath) ?? null, [skins, currentPath]);
  // 列表 = 内置 Steve / Alex（前端常量，唯一来源）+ 后端返回的本地皮肤。
  // 后端若也返回了内置项（builtin 标记或 builtin:// 路径），这里过滤掉，避免同一个 Steve 出现两次。
  const allSkins = useMemo(() => {
    const head = builtin ? [builtin.classic, builtin.slim] : [];
    const seen = new Set(head.map((item) => "builtin:" + item.name.toLowerCase()));
    const locals = (skins ?? []).filter((item) => {
      const isBuiltinLike =
        item.builtin === true ||
        (item.path ?? "").startsWith("builtin://") ||
        ["steve", "alex", "steve（默认）", "alex（纤细）"].includes(item.name.toLowerCase());
      if (isBuiltinLike) return false;
      const key = "local:" + (item.path || item.name).toLowerCase();
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
    return [...head, ...locals];
  }, [builtin, skins]);
  // 官方皮肤 tab 只放「按正版玩家名拉取」的皮肤，与「已保存」分工明确
  const officialSkins = useMemo(() => (skins ?? []).filter((item) => item.source === "mojang"), [skins]);
  const previewDataUrl = preview?.dataUrl ?? currentSkin?.dataUrl ?? builtin?.classic.dataUrl ?? null;

  /** 选文件 / 拖进来：先本地预览，点「应用」才真正落地。 */
  const acceptFile = useCallback(
    async (file: File | null | undefined) => {
      if (!file) return;
      try {
        const dataUrl = await fileToDataUrl(file);
        const size = await readImageSize(dataUrl);
        if (!isSkinSize(size)) {
          notify(t("skin.badPng"));
          return;
        }
        const staged: SkinItem = {
          name: file.name.replace(/\.png$/i, ""),
          path: file.name,
          size: file.size,
          dataUrl,
        };
        setPreview(staged);
        setTab("saved");
      } catch (error) {
        notify(String(error));
      }
    },
    [notify, t],
  );

  const isStaged = preview !== null && !(skins ?? []).some((item) => item.path === preview.path);

  const saveAndApply = async () => {
    if (!preview) {
      notify(t("skin.needFile"));
      return;
    }
    setBusy(true);
    try {
      const saved = isStaged
        ? await api.skinUpload(stripDataUrlPrefix(preview.dataUrl))
        : preview;
      await api.skinApply(saved.path);
      await load();
      notify(t("skin.savedToast", { name: saved.name }));
    } catch (error) {
      notify(String(error));
    } finally {
      setBusy(false);
    }
  };

  const saveOnly = async () => {
    if (!preview) {
      notify(t("skin.needFile"));
      return;
    }
    setBusy(true);
    try {
      const saved = isStaged ? await api.skinUpload(stripDataUrlPrefix(preview.dataUrl)) : preview;
      await load();
      notify(t("skin.savedLocalDone", { path: saved.path }));
    } catch (error) {
      notify(String(error));
    } finally {
      setBusy(false);
    }
  };

  const resetDraft = () => {
    setPreview(currentSkin);
    setPreviewCape(capes?.find((cape) => cape.name === currentCape) ?? null);
  };

  /** 从正版玩家名取皮肤（uapis 公开接口）：成功后直接预览，再决定要不要存到本地。 */
  const fetchMojangSkin = async () => {
    const name = mojangName.trim();
    if (!name) {
      setMojangError(t("skin.mojang.emptyName"));
      return;
    }
    setMojangBusy(true);
    setMojangError("");
    try {
      const found = await api.skinFetchMojang({ username: name });
      setMojangResult(found);
      setPreview(found);
      setPreviewCape(null);
      setTab("official");
      notify(t("skin.mojang.loaded", { name: found.name }));
    } catch (error) {
      setMojangResult(null);
      setMojangError(String(error instanceof Error ? error.message : error));
    } finally {
      setMojangBusy(false);
    }
  };

  const saveMojangSkin = async () => {
    if (!mojangResult) return;
    setMojangBusy(true);
    try {
      await load();
      setTab("saved");
      notify(t("skin.thirdpartySaved", { name: mojangResult.name }));
    } catch (error) {
      setMojangError(String(error));
    } finally {
      setMojangBusy(false);
    }
  };

  const applyCape = async (cape: CapeItem | null) => {
    try {
      await api.capeApply(cape ? cape.name : "");
      setCurrentCape(cape ? cape.name : "");
      setPreviewCape(cape);
      notify(cape ? t("cape.applied", { name: cape.name }) : t("cape.cleared"));
    } catch (error) {
      notify(String(error));
    }
  };

  /** 记一下上次用的皮肤站地址（顺便在成功时也写一次）。 */
  const rememberServer = (value: string) => {
    onChangeSettings({ thirdpartyAuthServer: value.trim() });
  };

  const fetchFromStation = async () => {
    const server = tpServer.trim();
    if (!server) {
      setTpError(t("skin.thirdpartyNeedServer"));
      return;
    }
    if (!tpName.trim()) {
      setTpError(t("skin.thirdpartyNeedName"));
      return;
    }
    setTpBusy(true);
    setTpError("");
    try {
      const found = await api.skinLookupThirdparty({ server, name: tpName.trim() });
      setTpResult(found);
      rememberServer(server);
      setPreview({ name: found.name, path: found.name + ".png", size: 0, dataUrl: found.skinUrl });
      setPreviewCape(
        found.capeUrl
          ? { name: "皮肤站披风", path: "", size: 0, dataUrl: found.capeUrl }
          : null,
      );
      notify(t("skin.thirdpartyLoaded", { name: found.name }));
    } catch (error) {
      setTpResult(null);
      setTpError(String(error instanceof Error ? error.message : error));
    } finally {
      setTpBusy(false);
    }
  };

  const saveFromStation = async () => {
    if (!tpResult) return;
    setTpSaving(true);
    try {
      const saved = await api.skinImportFromUrl({ url: tpResult.skinUrl, name: tpResult.name });
      await load();
      notify(t("skin.thirdpartySaved", { name: saved.name }));
    } catch (error) {
      setTpError(String(error));
    } finally {
      setTpSaving(false);
    }
  };

  return (
    <div className="skin-page">
      <section
        className={"card skin-panel" + (dragOver ? " is-dragover" : "")}
        onDragOver={(event) => {
          event.preventDefault();
          setDragOver(true);
        }}
        onDragLeave={() => setDragOver(false)}
        onDrop={(event) => {
          event.preventDefault();
          setDragOver(false);
          void acceptFile(event.dataTransfer.files?.[0]);
        }}
      >
        <SkinViewer3D
          skin={previewDataUrl}
          model={model}
          action={action}
          autoRotate={autoRotate}
          dark={dark}
          cape={previewCape?.dataUrl ?? null}
          resetToken={resetToken}
        />

        <div className="skin-panel__row">
          <span className="skin-panel__label">{t("skin.current")}</span>
          <span className="skin-panel__value" data-skin-current>
            {preview?.name ?? t("skin.none")}
          </span>
        </div>

        <div className="skin-panel__row">
          <span className="skin-panel__label">{t("skin.action")}</span>
          <div className="segmented segmented--sm">
            {ACTIONS.map((item) => (
              <button
                key={item.key}
                type="button"
                className={"segmented__item" + (action === item.key ? " is-active" : "")}
                data-skin-action={item.key}
                onClick={() => setAction(item.key)}
              >
                {t(item.labelKey)}
              </button>
            ))}
          </div>
        </div>

        <div className="skin-panel__row">
          <span className="skin-panel__label">{t("skin.autoRotate")}</span>
          <label className="switch">
            <input
              type="checkbox"
              checked={autoRotate}
              aria-label={t("skin.autoRotate")}
              onChange={(event) => setAutoRotate(event.target.checked)}
            />
            <span className="switch__track" aria-hidden="true">
              <span className="switch__dot" />
            </span>
          </label>
        </div>

        <div className="skin-panel__row">
          <span className="skin-panel__label">{t("skin.background")}</span>
          <div className="segmented segmented--sm">
            <button type="button" className={"segmented__item" + (dark ? "" : " is-active")} onClick={() => setDark(false)}>
              {t("skin.bg.light")}
            </button>
            <button type="button" className={"segmented__item" + (dark ? " is-active" : "")} data-skin-bg="dark" onClick={() => setDark(true)}>
              {t("skin.bg.dark")}
            </button>
          </div>
        </div>

        <div className="skin-panel__actions">
          <button type="button" className="pill-outline" onClick={() => setResetToken((value) => value + 1)}>
            <img src={icons.refresh} alt="" />
            {t("skin.resetView")}
          </button>
          <button type="button" className="pill-outline" disabled={busy} onClick={() => void saveOnly()}>
            <img src={icons.download} alt="" />
            {t("skin.saveLocal")}
          </button>
        </div>

        <div className="skin-panel__foot">
          <div className="segmented segmented--sm">
            <button
              type="button"
              className={"segmented__item" + (model === "default" ? " is-active" : "")}
              data-skin-model="default"
              onClick={() => {
                setModel("default");
                if (preview?.builtin && builtin) setPreview(builtin.classic);
              }}
            >
              {t("skin.model.classic")}
            </button>
            <button
              type="button"
              className={"segmented__item" + (model === "slim" ? " is-active" : "")}
              data-skin-model="slim"
              onClick={() => {
                setModel("slim");
                if (preview?.builtin && builtin) setPreview(builtin.slim);
              }}
            >
              {t("skin.model.slim")}
            </button>
          </div>
          <div className="skin-panel__foot-actions">
            <button type="button" className="pill-outline" onClick={resetDraft}>
              {t("skin.reset")}
            </button>
            <button type="button" className="pill-button" data-action="skin-apply" disabled={busy} onClick={() => void saveAndApply()}>
              {t("skin.apply")}
            </button>
          </div>
        </div>
      </section>

      <section className="card skin-library">
        <div className="skin-library__head">
          <div className="segmented segmented--sm">
            <button
              type="button"
              className={"segmented__item" + (tab === "saved" ? " is-active" : "")}
              data-skin-tab="saved"
              onClick={() => setTab("saved")}
            >
              {t("skin.saved")}
            </button>
            <button
              type="button"
              className={"segmented__item" + (tab === "official" ? " is-active" : "")}
              data-skin-tab="official"
              onClick={() => setTab("official")}
            >
              {t("skin.official")}
            </button>
          </div>
          <div className="skin-library__tools">
            <span className="row__hint">{t("skin.count", { count: tab === "saved" ? allSkins.length : 0 })}</span>
            <button
              type="button"
              className="pill-outline"
              data-action="skin-mojang-open-2"
              onClick={() => {
                setMojangError("");
                setMojangOpen(true);
              }}
            >
              <img src={icons.check} alt="" />
              {t("skin.byName")}
            </button>
            <button type="button" className="pill-outline" onClick={() => void load()}>
              <img src={icons.refresh} alt="" />
              {t("skin.refresh")}
            </button>
          </div>
        </div>

        {skins === null ? (
          <div className="row__hint" data-skin-loading>
            {t("common.loading")}
          </div>
        ) : null}
        {tab === "saved" ? (
          <div className="skin-grid">
            <button type="button" className="skin-tile skin-tile--upload" onClick={() => fileRef.current?.click()}>
              <span className="skin-tile__plus" aria-hidden="true">
                +
              </span>
              <span className="skin-tile__name">{t("skin.upload")}</span>
              <span className="skin-tile__meta">{t("skin.uploadHint")}</span>
            </button>
            {allSkins.map((item) => (
              <button
                type="button"
                key={item.path}
                className={
                  "skin-tile" +
                  (preview?.path === item.path ? " is-active" : "") +
                  (currentPath === item.path ? " is-current" : "") +
                  (item.builtin ? " is-builtin" : "")
                }
                data-skin-item={item.name}
                data-skin-name={item.name}
                data-skin-builtin={item.builtin ? "1" : "0"}
                onClick={() => setPreview(item)}
              >
                <TextureCrop
                  className="skin-tile__img"
                  src={item.dataUrl}
                  sx={8}
                  sy={8}
                  sw={8}
                  sh={8}
                  width={64}
                  height={64}
                />
                <span className="skin-tile__name">
                  {item.name}
                  {item.builtin ? <span className="chip chip--tiny">{t("skin.builtin")}</span> : null}
                </span>
                <span className="skin-tile__meta">{formatBytes(item.size)}</span>
              </button>
            ))}
            {isStaged && preview ? (
              <div className="skin-tile is-active is-staged">
                <TextureCrop
                  className="skin-tile__img"
                  src={preview.dataUrl}
                  sx={8}
                  sy={8}
                  sw={8}
                  sh={8}
                  width={64}
                  height={64}
                />
                <span className="skin-tile__name">{preview.name}</span>
                <span className="skin-tile__meta">{t("skin.uploadHint")}</span>
              </div>
            ) : null}
          </div>
        ) : officialSkins.length === 0 ? (
          <div className="empty-hint" data-skin-official-empty>
            {t("skin.officialEmpty")}
          </div>
        ) : (
          <div className="skin-grid">
            {officialSkins.map((item) => (
              <button
                type="button"
                key={item.path}
                className={"skin-tile" + (preview?.path === item.path ? " is-active" : "")}
                data-skin-item={item.name}
                data-skin-name={item.name}
                data-skin-source="mojang"
                onClick={() => setPreview(item)}
              >
                <TextureCrop className="skin-tile__img" src={item.dataUrl} sx={8} sy={8} sw={8} sh={8} width={64} height={64} />
                <span className="skin-tile__name">
                  {item.name}
                  <span className="chip chip--tiny">{t("skin.source.mojang")}</span>
                </span>
                <span className="skin-tile__meta">{formatBytes(item.size)}</span>
              </button>
            ))}
          </div>
        )}

        <input
          ref={fileRef}
          type="file"
          accept="image/png"
          className="visually-hidden"
          data-skin-file
          onChange={(event) => {
            void acceptFile(event.target.files?.[0]);
            event.target.value = "";
          }}
        />
        <div className="row__hint">{t("skin.dragHint")}</div>
      </section>

      <section className="card skin-thirdparty">
        <div className="skin-library__head">
          <span className="detail-card__title">{t("skin.thirdparty")}</span>
          {tpResult ? (
            <button
              type="button"
              className="link-button"
              data-action="skin-thirdparty-clear"
              onClick={() => {
                setTpResult(null);
                setTpError("");
                resetDraft();
              }}
            >
              {t("skin.thirdpartyClear")}
            </button>
          ) : null}
        </div>

        <div className="skin-thirdparty__form">
          <div className="field">
            <label htmlFor="tp-server">{t("skin.thirdpartyServer")}</label>
            <input
              id="tp-server"
              type="text"
              data-skin-tp-server
              placeholder={t("skin.thirdpartyServerPlaceholder")}
              value={tpServer}
              onChange={(event) => {
                setTpServer(event.target.value);
                setTpError("");
              }}
              onBlur={(event) => rememberServer(event.target.value)}
            />
            <div className="row__hint">{t("skin.thirdpartyServerNote")}</div>
          </div>
          <div className="field">
            <label htmlFor="tp-name">{t("skin.thirdpartyName")}</label>
            <input
              id="tp-name"
              type="text"
              data-skin-tp-name
              placeholder={t("skin.thirdpartyNamePlaceholder")}
              value={tpName}
              onChange={(event) => {
                setTpName(event.target.value);
                setTpError("");
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") void fetchFromStation();
              }}
            />
          </div>
          <div className="skin-thirdparty__actions">
            <button
              type="button"
              className="pill-button"
              data-action="skin-tp-fetch"
              disabled={tpBusy}
              onClick={() => void fetchFromStation()}
            >
              {tpBusy ? t("skin.thirdpartyFetching") : t("skin.thirdpartyFetch")}
            </button>
            {tpResult ? (
              <button
                type="button"
                className="pill-outline"
                data-action="skin-tp-save"
                disabled={tpSaving}
                onClick={() => void saveFromStation()}
              >
                {t("skin.thirdpartySave")}
              </button>
            ) : null}
          </div>
          {tpError ? (
            <div className="field-error" data-skin-tp-error>
              {tpError}
            </div>
          ) : null}
          {tpResult ? (
            <div className="skin-thirdparty__result" data-skin-tp-result>
              <span className="mono">{tpResult.uuid.slice(0, 8)}</span>
              <span className="row__hint">{tpResult.name}</span>
              {tpResult.capeUrl ? (
                <span className="chip chip--tiny" data-skin-tp-cape>
                  {t("skin.thirdpartyCape")}
                </span>
              ) : (
                <span className="row__hint" data-skin-tp-nocape>
                  {t("skin.thirdpartyNoCape")}
                </span>
              )}
            </div>
          ) : null}
        </div>

        <div className="row__hint">{t("skin.thirdpartyHint")}</div>
      </section>

      <section className="card cape-library">
        <div className="skin-library__head">
          <span className="detail-card__title">{t("cape.title")}</span>
          <span className="row__hint">
            {t("cape.current")}：{currentCape || t("cape.none")}
          </span>
        </div>
        {capes === null ? (
          <div className="row__hint">{t("common.loading")}</div>
        ) : capes.length === 0 ? (
          <div className="empty-hint">{t("cape.empty")}</div>
        ) : (
          <div className="cape-grid">
            <button
              type="button"
              className={"cape-tile" + (previewCape === null ? " is-active" : "")}
              data-cape-item="none"
              onClick={() => setPreviewCape(null)}
            >
              <span className="cape-tile__none">{t("cape.none")}</span>
            </button>
            {capes.map((cape) => (
              <button
                type="button"
                key={cape.name}
                className={"cape-tile" + (previewCape?.name === cape.name ? " is-active" : "")}
                data-cape-item={cape.name}
                onClick={() => setPreviewCape(cape)}
              >
                <TextureCrop
                  className="cape-tile__img"
                  src={cape.dataUrl}
                  sx={1}
                  sy={1}
                  sw={10}
                  sh={16}
                  width={40}
                  height={64}
                />
                <span className="skin-tile__name">{cape.name}</span>
              </button>
            ))}
          </div>
        )}
        <div className="cape-library__actions">
          <button type="button" className="pill-outline" onClick={() => void applyCape(null)}>
            {t("cape.clear")}
          </button>
          <button
            type="button"
            className="pill-button"
            data-action="cape-apply"
            onClick={() => void applyCape(previewCape)}
          >
            {t("cape.apply")}
          </button>
        </div>
      </section>

      {mojangOpen ? (
        <Modal
          title={t("skin.mojang.title")}
          onClose={() => setMojangOpen(false)}
          footer={
            <>
              <button type="button" className="pill-outline" onClick={() => setMojangOpen(false)}>
                {t("common.cancel")}
              </button>
              {mojangResult ? (
                <button type="button" className="pill-button" data-action="skin-mojang-save" disabled={mojangBusy} onClick={() => void saveMojangSkin()}>
                  {t("skin.thirdpartySave")}
                </button>
              ) : null}
            </>
          }
        >
          <div className="field">
            <label htmlFor="mojang-name">{t("skin.mojang.name")}</label>
            <div className="field-pair">
              <input
                id="mojang-name"
                type="text"
                data-skin-mojang-name
                placeholder={t("skin.mojang.placeholder")}
                value={mojangName}
                onChange={(event) => {
                  setMojangName(event.target.value);
                  setMojangError("");
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter") void fetchMojangSkin();
                }}
              />
              <button type="button" className="pill-button" data-action="skin-mojang-fetch" disabled={mojangBusy} onClick={() => void fetchMojangSkin()}>
                {mojangBusy ? t("skin.mojang.fetching") : t("skin.mojang.fetch")}
              </button>
            </div>
            {mojangError ? (
              <div className="field-error" data-skin-mojang-error>
                {mojangError}
              </div>
            ) : (
              <div className="row__hint">{t("skin.mojang.note")}</div>
            )}
          </div>
          {mojangResult ? (
            <div className="skin-thirdparty__result" data-skin-mojang-result>
              <span className="row__hint">{mojangResult.name}</span>
              <span className="mono">{mojangResult.uuid ?? "—"}</span>
            </div>
          ) : null}
        </Modal>
      ) : null}
    </div>
  );
}
