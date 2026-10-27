import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { icons } from "../assets/icons";
import { HeadAvatar } from "../components/HeadAvatar";
import { type SkinAction, SkinViewer3D } from "../components/SkinViewer3D";
import { Modal } from "../components/Modal";
import { TextureCrop } from "../components/TextureCrop";
import { formatBytes } from "../lib/format";
import { useT } from "../lib/i18n";
import { BUILTIN_SKINS, OFFICIAL_SKINS } from "../lib/officialSkins";
import { fileToDataUrl, hasDiskPath, imageUrlToBase64, isSkinSize, readImageSize, stripDataUrlPrefix } from "../lib/image-file";
import { api } from "../lib/tauri-api";
import type { CapesListResult, Settings, SkinItem, SkinModel } from "../lib/types";

interface Props {
  notify: (message: string) => void;
  settings: Settings | null;
  onChangeSettings: (patch: Partial<Settings>) => void;
  /** 「去账号页」：披风只属于正版账号，不支持的账号要能一键跳过去 */
  onOpenAccounts: () => void;
}

/** 3D 预览用的披风贴图：只关心名字和图片。 */
interface CapePreview {
  name: string;
  dataUrl: string;
}

/** 内置 Steve（经典）/ Alex（纤细）：打包资源直接给，不依赖后端 skin_default。 */
const BUILTIN = { classic: BUILTIN_SKINS[0], slim: BUILTIN_SKINS[1] };

const ACTIONS: Array<{ key: SkinAction; labelKey: string }> = [
  { key: "idle", labelKey: "skin.action.idle" },
  { key: "walk", labelKey: "skin.action.walk" },
  { key: "run", labelKey: "skin.action.run" },
  { key: "none", labelKey: "skin.action.none" },
];

/** 皮肤页：左边 3D 预览（左下按钮组里开披风设置），右边已保存 / 官方皮肤。 */
export function SkinsPage({ notify, onOpenAccounts }: Props) {
  const t = useT();
  const [skins, setSkins] = useState<SkinItem[] | null>(null);
  const [capes, setCapes] = useState<CapesListResult | null>(null);
  const [currentPath, setCurrentPath] = useState("");
  const [preview, setPreview] = useState<SkinItem | null>(null);
  const [previewCape, setPreviewCape] = useState<CapePreview | null>(null);
  const [model, setModel] = useState<SkinModel>("default");
  const [action, setAction] = useState<SkinAction>("idle");
  const [autoRotate, setAutoRotate] = useState(false);
  const [dark, setDark] = useState(false);
  const [resetToken, setResetToken] = useState(0);
  const [busy, setBusy] = useState(false);
  const [accountName, setAccountName] = useState("");
  const [isMicrosoft, setIsMicrosoft] = useState(false);
  const [uploadOpen, setUploadOpen] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [uploadError, setUploadError] = useState("");
  const [dragOver, setDragOver] = useState(false);
  const [tab, setTab] = useState<"saved" | "official">("saved");
  // 披风设置弹窗：draft 是弹窗里选中但还没应用的 id（null = 不使用披风）
  const [capeOpen, setCapeOpen] = useState(false);
  const [capeDraft, setCapeDraft] = useState<string | null>(null);
  const [capeSaving, setCapeSaving] = useState(false);
  // 本地皮肤删除：pending = 等二次确认，deleting = 正在删（卡片半透明）
  const [pendingDeleteSkin, setPendingDeleteSkin] = useState<SkinItem | null>(null);
  const [deletingName, setDeletingName] = useState<string | null>(null);
  const [deleteError, setDeleteError] = useState("");
  // 恢复正版皮肤：restoring = 取回中（按钮禁用），restoreError 里放人话
  const [restoring, setRestoring] = useState(false);
  const [restoreError, setRestoreError] = useState("");
  const [restoreNeedsAccounts, setRestoreNeedsAccounts] = useState(false);
  const fileRef = useRef<HTMLInputElement | null>(null);

  // 内置 Steve / Alex（列表前两项，不落盘、不可删）
  const builtin = BUILTIN;
  // 从正版账号获取
  const [mojangOpen, setMojangOpen] = useState(false);
  const [mojangName, setMojangName] = useState("");
  const [mojangBusy, setMojangBusy] = useState(false);
  const [mojangError, setMojangError] = useState("");
  const [mojangResult, setMojangResult] = useState<SkinItem | null>(null);

  useEffect(() => {
    void api
      .accountsList()
      .then((accounts) => {
        const active = accounts.find((item) => item.current) ?? null;
        setAccountName(active?.name ?? "");
        setIsMicrosoft(active?.type === "microsoft");
      })
      .catch(() => undefined);
  }, []);

  const load = useCallback(async () => {
    // 三个命令互不牵连：披风失败不该把「已保存皮肤」清空，内置 Steve / Alex 更是永远要在
    const [skinResult, currentResult, capeResult] = await Promise.allSettled([
      api.skinsList(),
      api.skinCurrent(),
      api.capesList(),
    ]);
    const list = skinResult.status === "fulfilled" ? skinResult.value : [];
    const current = currentResult.status === "fulfilled" ? currentResult.value : "";
    const capeList = capeResult.status === "fulfilled" ? capeResult.value : null;
    setSkins(list);
    setCapes(capeList);
    setCurrentPath(current);
    // 没选皮肤（或选的那张已经不在了）→ 直接用内置 Steve，不留空白
    const active = list.find((item) => item.path === current) ?? BUILTIN.classic;
    setPreview((prev) => {
      if (!prev) return active;
      if (prev.builtin) return prev;
      return list.find((item) => item.path === prev.path) ?? active;
    });
    const activeCape = capeList?.items.find((cape) => cape.id === capeList.active) ?? null;
    setPreviewCape(activeCape ? { name: activeCape.name, dataUrl: activeCape.url } : null);
    const failed = [skinResult, capeResult].find((item) => item.status === "rejected");
    if (failed && failed.status === "rejected") notify(String(failed.reason));
  }, [notify]);

  useEffect(() => {
    void load();
  }, [load]);

  const currentSkin = useMemo(() => (skins ?? []).find((item) => item.path === currentPath) ?? null, [skins, currentPath]);
  // 列表 = 内置 Steve / Alex（前端常量，唯一来源）+ 后端返回的本地皮肤。
  // 后端若也返回了内置项（builtin 标记或 builtin:// 路径），这里过滤掉，避免同一个 Steve 出现两次。
  // 「本地皮肤」只放用户自己导入/保存的（.mc\skins 里的文件，含按玩家名获取后保存的）。
  // 内置 Steve / Alex 只作为「官方皮肤」tab 的 9 套之一出现，不再混进这个列表。
  const allSkins = useMemo(() => {
    const seen = new Set<string>();
    return (skins ?? []).filter((item) => {
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
  }, [skins]);
  // 官方皮肤 tab 只放「按正版玩家名拉取」的皮肤，与「本地皮肤」分工明确
  const officialSkins = useMemo(() => (skins ?? []).filter((item) => item.source === "mojang"), [skins]);
  const [officialId, setOfficialId] = useState<string | null>(null);
  const previewDataUrl = preview?.dataUrl ?? currentSkin?.dataUrl ?? builtin?.classic.dataUrl ?? null;
  // 披风只属于正版账号：supported=false 时 items 恒为空，界面换成提示块而不是空网格
  const capeSupported = capes?.supported !== false;
  const capeItems = capes?.items ?? [];
  const activeCapeItem = capeItems.find((cape) => cape.id === capes?.active) ?? null;

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

  // 内置 Steve/Alex 只是预览用的常量，不算「刚拖入还没保存」
  const isStaged = preview !== null && !preview.builtin && !(skins ?? []).some((item) => item.path === preview.path);

  const saveAndApply = async () => {
    if (!preview) {
      notify(t("skin.needFile"));
      return;
    }
    // 正版账号要走 Mojang 上传，必须先二次确认；离线/第三方直接本地应用
    if (isMicrosoft) {
      setUploadError("");
      setUploadOpen(true);
      return;
    }
    setBusy(true);
    try {
      const saved = isStaged ? await api.skinUpload(stripDataUrlPrefix(preview.dataUrl)) : preview;
      await api.skinApply(saved.path);
      await load();
      notify(t("skin.appliedLocal"));
    } catch (error) {
      notify(String(error));
    } finally {
      setBusy(false);
    }
  };

  /**
   * 确认上传：先把贴图落到本地，再调 ms_skin_upload。
   * 官方 9 套 / 内置 Steve、Alex 是打包资源，运行时没有磁盘路径 → 只能传 pngBase64；
   * 本地已保存的皮肤直接给 pngPath。失败把后端原文留在弹窗红字里。
   */
  const confirmUpload = async () => {
    if (!preview) return;
    setUploading(true);
    setUploadError("");
    try {
      // 官方 / 内置 / 刚拖入的皮肤在磁盘上还没有文件：先落到本地存档，再拿 base64 去上传
      const needsLocalCopy = isStaged || !hasDiskPath(preview.path);
      let saved = preview;
      let pngBase64: string | undefined;
      if (needsLocalCopy) {
        pngBase64 = await imageUrlToBase64(preview.dataUrl);
        saved = await api.skinUpload(pngBase64);
      }
      await api.msSkinUpload({
        pngPath: needsLocalCopy ? undefined : saved.path,
        pngBase64,
        variant: model === "slim" ? "slim" : "classic",
      });
      await api.skinApply(saved.path);
      await load();
      setUploadOpen(false);
      notify(t("skin.uploadedToast", { name: accountName }));
    } catch (error) {
      setUploadError(String(error instanceof Error ? error.message : error));
    } finally {
      setUploading(false);
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
    setPreviewCape(activeCapeItem ? { name: activeCapeItem.name, dataUrl: activeCapeItem.url } : null);
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

  /**
   * 恢复正版皮肤：ms_profile 拿到账号在用的皮肤 → 先直接远程预览 → 落盘到本地皮肤 → 显示为当前皮肤。
   * 没有任何上传动作，只做本地恢复。
   */
  const restoreOfficialSkin = async () => {
    setRestoring(true);
    setRestoreError("");
    setRestoreNeedsAccounts(false);
    try {
      const profile = await api.msProfile();
      const nextModel: SkinModel = profile.variant === "slim" ? "slim" : "default";
      if (!profile.skinUrl) {
        // 账号用的是默认皮肤：按 variant 应用内置 Steve / Alex
        setPreview(profile.variant === "slim" ? BUILTIN.slim : BUILTIN.classic);
        setModel(nextModel);
        notify(t("skin.restoreDefaultSkin", { model: profile.variant === "slim" ? t("skin.model.slim") : t("skin.model.classic") }));
        return;
      }
      const displayName = (profile.name || accountName || "微软账号") + "-正版";
      // 预览层允许直接吃远程 URL，不必先落盘
      setPreview({ name: displayName, path: profile.skinUrl, dataUrl: profile.skinUrl, size: 0 });
      setModel(nextModel);
      const saved = await api.skinImportFromUrl({ url: profile.skinUrl, name: displayName });
      await load();
      setPreview(saved);
      notify(t("skin.restored", { name: profile.name || accountName }));
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (/过期|expired/i.test(message)) {
        setRestoreError(t("skin.restoreExpired"));
        setRestoreNeedsAccounts(true);
      } else if (/登录|login/i.test(message)) {
        setRestoreError(t("skin.restoreNeedLogin"));
        setRestoreNeedsAccounts(true);
      } else {
        setRestoreError(t("skin.restoreFailed", { message }));
      }
    } finally {
      setRestoring(false);
    }
  };

  /** 删除本地皮肤：确认后调 skins_delete，刷新列表；删掉当前在用的就切回内置 Steve。 */
  const removeSkin = async () => {
    if (!pendingDeleteSkin) return;
    const target = pendingDeleteSkin;
    const wasCurrent = currentPath === target.path || preview?.path === target.path;
    setPendingDeleteSkin(null);
    setDeletingName(target.name);
    setDeleteError("");
    try {
      await api.skinsDelete({ name: target.name });
      await load();
      if (wasCurrent) {
        setPreview(BUILTIN.classic);
        notify(t("skin.switchedToSteve"));
      } else {
        notify(t("skin.deleted", { name: target.name }));
      }
    } catch (error) {
      setDeleteError(t("skin.deleteFailed", { message: error instanceof Error ? error.message : String(error) }));
    } finally {
      setDeletingName(null);
    }
  };

  /** 打开披风设置：草稿默认是当前装备的那张。 */
  const openCapeSettings = () => {
    setCapeDraft(capes?.active ?? null);
    setCapeOpen(true);
  };

  /** 应用披风：cape_set_active → 重新拉列表 → 3D 预览立刻换成新披风。 */
  const saveCape = async () => {
    setCapeSaving(true);
    try {
      const result = await api.capeSetActive({ capeId: capeDraft });
      const next = await api.capesList();
      setCapes(next);
      const active = next.items.find((cape) => cape.id === result.active) ?? null;
      setPreviewCape(active ? { name: active.name, dataUrl: active.url } : null);
      setCapeOpen(false);
      notify(t("cape.updated"));
    } catch (error) {
      notify(String(error));
    } finally {
      setCapeSaving(false);
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
          cape={capeSupported ? (previewCape?.dataUrl ?? null) : null}
          capeNote={capeSupported ? null : t("cape.needOfficial")}
          resetToken={resetToken}
        />

        <div className="skin-panel__row" data-skin-account>
          <span className="skin-panel__label">{t("skin.account")}</span>
          <span className="skin-panel__value">
            {accountName || "—"}
            {isMicrosoft ? (
              <span className="chip chip--tiny" data-skin-account-badge>
                {t("skin.accountBadge")}
              </span>
            ) : null}
          </span>
        </div>

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
          <button type="button" className="pill-outline" data-action="cape-open-2" onClick={openCapeSettings}>
            {t("cape.title")}
          </button>
          {isMicrosoft ? (
            <button
              type="button"
              className="pill-outline"
              data-action="skin-restore-ms"
              disabled={restoring}
              title={t("skin.restore")}
              onClick={() => void restoreOfficialSkin()}
            >
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                <path d="M7 17.5h9.5a3.5 3.5 0 0 0 .4-6.98A5 5 0 0 0 7.6 9.2 3.7 3.7 0 0 0 7 17.5z" />
                <path d="M12 11.5v5.2M9.9 14.6 12 16.7l2.1-2.1" />
              </svg>
              {restoring ? t("skin.restoring") : t("skin.restore")}
            </button>
          ) : null}
          <button type="button" className="pill-outline" disabled={busy} onClick={() => void saveOnly()}>
            <img src={icons.download} alt="" />
            {t("skin.saveLocal")}
          </button>
        </div>

        {restoreError ? (
          <div className="field-error" data-skin-restore-error>
            {restoreError}
            {restoreNeedsAccounts ? (
              <button type="button" className="link-button" data-action="skin-restore-go-accounts" onClick={onOpenAccounts}>
                {t("cape.goAccounts")}
              </button>
            ) : null}
          </div>
        ) : null}

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
              {isMicrosoft ? t("skin.apply") : t("skin.applyLocal")}
            </button>
          </div>
        </div>
      </section>

      {uploadOpen ? (
        <Modal
          title={t("skin.uploadTitle")}
          onClose={() => setUploadOpen(false)}
          footer={
            <>
              <button type="button" className="pill-outline" onClick={() => setUploadOpen(false)}>
                {t("common.cancel")}
              </button>
              <button type="button" className="pill-button" data-action="skin-upload-confirm" disabled={uploading} onClick={() => void confirmUpload()}>
                {uploading ? t("skin.uploading") : t("skin.uploadConfirm")}
              </button>
            </>
          }
        >
          <div className="skin-upload" data-skin-upload-modal>
            <div className="skin-upload__row">
              {preview ? <img className="skin-upload__thumb" src={preview.dataUrl} alt="" width={48} height={48} /> : null}
              <div>
                <p data-skin-upload-lead>
                  {t("skin.uploadLead", { model: model === "slim" ? t("skin.model.slim") : t("skin.model.classic"), name: accountName || "—" })}
                </p>
                <span className="chip chip--tiny" data-skin-upload-model>
                  {model === "slim" ? t("skin.model.slim") : t("skin.model.classic")}
                </span>
              </div>
            </div>
            <div className="skin-upload__warn" data-skin-upload-warn>
              {t("skin.uploadWarn")}
            </div>
            {uploadError ? (
              <div className="field-error" data-skin-upload-error>
                {uploadError}
              </div>
            ) : null}
          </div>
        </Modal>
      ) : null}

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
            <span className="row__hint" data-skin-count>
              {t("skin.count", { count: tab === "saved" ? allSkins.length : OFFICIAL_SKINS.length })}
            </span>
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
        {deleteError ? (
          <div className="field-error" data-skin-delete-error>
            {deleteError}
          </div>
        ) : null}
        {tab === "saved" ? (
          allSkins.length === 0 && !isStaged ? (
            <div className="empty-hint skin-empty" data-skin-local-empty>
              {t("skin.localEmpty")}
              <button type="button" className="pill-outline" data-action="skin-upload-empty" onClick={() => fileRef.current?.click()}>
                {t("skin.upload")}
              </button>
            </div>
          ) : (
          <div className="skin-grid">
            <button type="button" className="skin-tile skin-tile--upload" onClick={() => fileRef.current?.click()}>
              <span className="skin-tile__plus" aria-hidden="true">
                +
              </span>
              <span className="skin-tile__name">{t("skin.upload")}</span>
              <span className="skin-tile__meta">{t("skin.uploadHint")}</span>
            </button>
            {allSkins.map((item) => (
              <div className={"skin-tile-wrap" + (deletingName === item.name ? " is-deleting" : "")} key={item.path}>
                <button
                  type="button"
                  className={
                    "skin-tile" +
                    (preview?.path === item.path ? " is-active" : "") +
                    (currentPath === item.path ? " is-current" : "")
                  }
                  data-skin-item={item.name}
                  data-skin-name={item.name}
                  data-skin-builtin="0"
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
                  <span className="skin-tile__name">{item.name}</span>
                  <span className="skin-tile__meta">{formatBytes(item.size)}</span>
                </button>
                <button
                  type="button"
                  className="skin-tile__delete"
                  data-action="skin-delete"
                  data-skin-delete={item.name}
                  title={t("common.delete")}
                  aria-label={t("skin.deleteConfirmTitle")}
                  onClick={(event) => {
                    event.stopPropagation();
                    setDeleteError("");
                    setPendingDeleteSkin(item);
                  }}
                >
                  <img src={icons.trash} alt={t("common.delete")} />
                </button>
                {deletingName === item.name ? <span className="skin-tile__deleting">{t("skin.deleting")}</span> : null}
              </div>
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
          )
        ) : (
          <div className="official-skins" data-official-skins>
            <div className="detail-card__title">{t("skin.officialGroup")}</div>
            <div className="official-skins__grid">
              {OFFICIAL_SKINS.map((entry) => (
                <button
                  type="button"
                  key={entry.id}
                  className={"official-skin" + (officialId === entry.id ? " is-active" : "")}
                  data-official-skin={entry.id}
                  data-official-model={entry.model === "classic" ? "classic" : "slim"}
                  data-official-name={entry.name}
                  data-official-url={entry.url}
                  onClick={() => {
                    setOfficialId(entry.id);
                    setPreview({
                      name: entry.name,
                      path: "official://" + entry.id,
                      dataUrl: entry.url,
                      size: 0,
                      builtin: false,
                      source: "mojang",
                    });
                    setModel(entry.model === "classic" ? "default" : "slim");
                  }}
                >
                  {/* 头像要和账号页一致：先裁头部（8,8）再叠帽子层（40,8），别把整张 64×64 贴图塞进来 */}
                  <HeadAvatar className="official-skin__img" skin={entry.url} size={56} />
                  <span className="official-skin__name">{entry.name}</span>
                  <span className="official-skin__model">
                    {entry.model === "classic" ? t("skin.model.classic") : t("skin.model.slim")}
                  </span>
                </button>
              ))}
            </div>
            {officialSkins.length > 0 ? (
              <div className="row__hint" data-official-saved>
                {t("skin.officialSavedHint")}
              </div>
            ) : null}
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

      {capeOpen ? (
        <Modal
          title={t("cape.settings")}
          className="modal--wide"
          onClose={() => setCapeOpen(false)}
          footer={
            <>
              <button type="button" className="pill-outline" onClick={() => setCapeOpen(false)}>
                {t("common.cancel")}
              </button>
              {capeSupported ? (
                <button
                  type="button"
                  className="pill-button"
                  data-action="cape-save"
                  disabled={capeSaving}
                  onClick={() => void saveCape()}
                >
                  {capeSaving ? t("cape.saving") : t("cape.apply")}
                </button>
              ) : null}
            </>
          }
        >
          <div className="cape-modal" data-cape-modal>
            <p className="cape-modal__lead">{t("cape.modalLead")}</p>
            {capes === null ? (
              <div className="row__hint">{t("common.loading")}</div>
            ) : capeSupported ? (
              <div className="cape-picker" data-cape-picker>
                <button
                  type="button"
                  className={"cape-pick" + (capeDraft === null ? " is-active" : "")}
                  data-cape-option="none"
                  onClick={() => setCapeDraft(null)}
                >
                  <span className="cape-pick__img cape-pick__img--none" aria-hidden="true" />
                  <span className="cape-pick__name" title={t("cape.noneTitle")}>
                    {t("cape.noneTitle")}
                  </span>
                  <span className="cape-pick__meta">{t("cape.noneMeta")}</span>
                  {capeDraft === null ? (
                    <span className="cape-pick__check" aria-hidden="true">
                      ✓
                    </span>
                  ) : null}
                </button>
                {capeItems.map((cape) => (
                  <button
                    type="button"
                    key={cape.id}
                    className={
                      "cape-pick" +
                      (capeDraft === cape.id ? " is-active" : "") +
                      (cape.id === capes.active ? " is-current" : "")
                    }
                    data-cape-option={cape.id}
                    data-cape-state={cape.state}
                    onClick={() => setCapeDraft(cape.id)}
                  >
                    {cape.previewDataUrl ? (
                      <img className="cape-pick__img" src={cape.previewDataUrl} alt="" width={40} height={64} />
                    ) : (
                      <TextureCrop
                        className="cape-pick__img"
                        src={cape.url}
                        sx={1}
                        sy={1}
                        sw={10}
                        sh={16}
                        width={40}
                        height={64}
                      />
                    )}
                    <span className="cape-pick__name" title={cape.alias ? cape.name + " · " + cape.alias : cape.name}>
                      {cape.name}
                    </span>
                    <span className="cape-pick__meta">{t("cape.kind")}</span>
                    {capeDraft === cape.id ? (
                      <span className="cape-pick__check" aria-hidden="true">
                        ✓
                      </span>
                    ) : null}
                  </button>
                ))}
              </div>
            ) : (
              <div className="notice cape-modal__notice" data-cape-unsupported>
                <strong>{t("cape.unsupportedTitle")}</strong>
                <span data-cape-reason>{capes.reason ?? ""}</span>
                <button
                  type="button"
                  className="pill-button"
                  data-action="cape-go-accounts"
                  onClick={() => {
                    setCapeOpen(false);
                    onOpenAccounts();
                  }}
                >
                  {t("cape.goAccounts")}
                </button>
              </div>
            )}
          </div>
        </Modal>
      ) : null}

      {pendingDeleteSkin ? (
        <Modal
          title={t("skin.deleteConfirmTitle")}
          onClose={() => setPendingDeleteSkin(null)}
          footer={
            <>
              <button type="button" className="pill-outline" onClick={() => setPendingDeleteSkin(null)}>
                {t("common.cancel")}
              </button>
              <button type="button" className="pill-button" data-action="skin-delete-confirm" onClick={() => void removeSkin()}>
                {t("common.delete")}
              </button>
            </>
          }
        >
          <p data-skin-delete-confirm-text>{t("skin.deleteConfirm", { name: pendingDeleteSkin.name })}</p>
        </Modal>
      ) : null}

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
