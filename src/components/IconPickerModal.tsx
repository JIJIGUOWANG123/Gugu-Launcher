import { useRef, useState } from "react";
import { icons } from "../assets/icons";
import { BUILTIN_INSTANCE_ICONS } from "../assets/instance-icons";
import { Modal } from "./Modal";
import { DEFAULT_ICON_GRADIENT, InstanceIcon } from "./InstanceIcon";
import { useT } from "../lib/i18n";
import type { InstanceIconSpec } from "../lib/types";

interface Props {
  spec: InstanceIconSpec | undefined;
  name: string;
  onChange: (spec: InstanceIconSpec) => void;
  onClose: () => void;
  notify: (message: string) => void;
}

/** 12 个预设渐变（直接用 CSS linear-gradient，不依赖后端）。 */
const GRADIENTS: Array<{ id: string; label: string; css: string }> = [
  { id: "dawn", label: "晨曦", css: "linear-gradient(135deg, #ffd89b, #ff8a5c)" },
  { id: "mint", label: "薄荷", css: "linear-gradient(135deg, #b8f2e6, #5fb8a8)" },
  { id: "sky", label: "天空", css: "linear-gradient(135deg, #cfe3ff, #7aa7f0)" },
  { id: "grape", label: "葡萄", css: "linear-gradient(135deg, #e0c3fc, #8e6cf0)" },
  { id: "lemon", label: "柠檬", css: "linear-gradient(135deg, #fff6b7, #f6c945)" },
  { id: "forest", label: "森林", css: "linear-gradient(135deg, #c7f0c0, #4f9d5a)" },
  { id: "coral", label: "珊瑚", css: "linear-gradient(135deg, #ffd1dc, #ff6b8b)" },
  { id: "slate", label: "石墨", css: "linear-gradient(135deg, #dfe3ea, #7c8798)" },
  { id: "ember", label: "余烬", css: "linear-gradient(135deg, #ffb199, #c62f2f)" },
  { id: "ocean", label: "深海", css: "linear-gradient(135deg, #a8e6ff, #2b6cb0)" },
  { id: "candy", label: "糖果", css: "linear-gradient(135deg, #ffd6f5, #b06ae0)" },
  { id: "sand", label: "沙丘", css: "linear-gradient(135deg, #f4e4c1, #c9a227)" },
];

const MAX_UPLOAD_BYTES = 256 * 1024;
const TARGET_SIZE = 256;

/** 上传的图统一缩放到 256×256（等比裁切），再交给后端落盘。 */
async function toSquareDataUrl(file: File): Promise<string> {
  const raw = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.onerror = () => reject(new Error("读取图片失败"));
    reader.readAsDataURL(file);
  });
  const image = await new Promise<HTMLImageElement>((resolve, reject) => {
    const element = new Image();
    element.onload = () => resolve(element);
    element.onerror = () => reject(new Error("这张图片打不开"));
    element.src = raw;
  });
  const canvas = document.createElement("canvas");
  canvas.width = TARGET_SIZE;
  canvas.height = TARGET_SIZE;
  const ctx = canvas.getContext("2d");
  if (!ctx) return raw;
  const side = Math.min(image.width, image.height);
  ctx.drawImage(image, (image.width - side) / 2, (image.height - side) / 2, side, side, 0, 0, TARGET_SIZE, TARGET_SIZE);
  return canvas.toDataURL("image/png");
}

/**
 * 「选择图标」弹窗：A 底板渐变（12 预设 + 自定义两色）/ B 21 张内置图标 / C 上传本地图。
 * 内置图标只能换不能删；上传的自定义图标可以删掉（回到名称首字）。
 */
export function IconPickerModal({ spec, name, onChange, onClose, notify }: Props) {
  const t = useT();
  const fileRef = useRef<HTMLInputElement | null>(null);
  const [from, setFrom] = useState("#cfe3ff");
  const [to, setTo] = useState("#7aa7f0");
  const [uploading, setUploading] = useState(false);
  const current = spec ?? { kind: "none" as const };
  const gradient = current.gradient || DEFAULT_ICON_GRADIENT;

  const apply = (next: Partial<InstanceIconSpec>) => onChange({ ...current, gradient, ...next });

  const upload = async (file: File | null | undefined) => {
    if (!file) return;
    if (!/^image\/(png|jpeg|jpg)$/i.test(file.type)) {
      notify(t("icon.uploadType"));
      return;
    }
    if (file.size > MAX_UPLOAD_BYTES) {
      notify(t("icon.uploadTooBig", { size: Math.round(file.size / 1024) }));
      return;
    }
    setUploading(true);
    try {
      const dataUrl = await toSquareDataUrl(file);
      onChange({ kind: "custom", dataUrl, gradient, id: undefined, path: undefined });
      notify(t("icon.uploaded"));
    } catch (error) {
      notify(error instanceof Error ? error.message : String(error));
    } finally {
      setUploading(false);
    }
  };

  return (
    <Modal
      title={t("icon.title")}
      onClose={onClose}
      className="icon-picker-modal"
      footer={
        <button type="button" className="pill-button" data-action="icon-done" onClick={onClose}>
          {t("common.finish")}
        </button>
      }
    >
      <div className="icon-picker" data-icon-picker>
        <div className="icon-picker__preview">
          <InstanceIcon spec={{ ...current, gradient }} name={name} className="detail-icon__preview" alt={t("icon.preview")} />
          <div className="row__hint">
            {current.kind === "builtin"
              ? t("icon.currentBuiltin", { id: BUILTIN_INSTANCE_ICONS.find((item) => item.id === current.id)?.label ?? current.id ?? "" })
              : current.kind === "custom"
                ? t("icon.currentCustom")
                : t("icon.currentNone", { initial: (name || "?").trim().slice(0, 1) })}
          </div>
        </div>

        {/* A. 背景渐变色 */}
        <section className="icon-picker__section" data-icon-gradients>
          <div className="icon-picker__title">{t("icon.gradientTitle")}</div>
          <div className="icon-gradient-grid">
            {GRADIENTS.map((item) => (
              <button
                key={item.id}
                type="button"
                className={"icon-gradient" + (gradient === item.css ? " is-active" : "")}
                data-icon-gradient={item.id}
                title={item.label}
                style={{ background: item.css }}
                onClick={() => apply({ gradient: item.css })}
              >
                <span>{item.label}</span>
              </button>
            ))}
          </div>
          <div className="icon-color-row">
            <label className="icon-color">
              <span>{t("icon.colorFrom")}</span>
              <input
                type="color"
                value={from}
                data-icon-color="from"
                onChange={(event) => {
                  setFrom(event.target.value);
                  apply({ gradient: "linear-gradient(135deg, " + event.target.value + ", " + to + ")" });
                }}
              />
            </label>
            <label className="icon-color">
              <span>{t("icon.colorTo")}</span>
              <input
                type="color"
                value={to}
                data-icon-color="to"
                onChange={(event) => {
                  setTo(event.target.value);
                  apply({ gradient: "linear-gradient(135deg, " + from + ", " + event.target.value + ")" });
                }}
              />
            </label>
          </div>
        </section>

        {/* B. 内置图标（21 张，不可删除） */}
        <section className="icon-picker__section" data-icon-builtins>
          <div className="icon-picker__title">{t("icon.builtinTitle", { count: BUILTIN_INSTANCE_ICONS.length })}</div>
          <div className="icon-builtin-grid">
            {BUILTIN_INSTANCE_ICONS.map((item) => (
              <button
                key={item.id}
                type="button"
                className={"icon-builtin" + (current.kind === "builtin" && current.id === item.id ? " is-active" : "")}
                data-icon-builtin={item.id}
                title={item.label}
                onClick={() => onChange({ kind: "builtin", id: item.id, gradient, dataUrl: undefined, path: undefined })}
              >
                <img src={item.url} alt={item.label} />
                <span>{item.label}</span>
              </button>
            ))}
          </div>
        </section>

        {/* C. 上传本地图 */}
        <section className="icon-picker__section" data-icon-upload>
          <div className="icon-picker__title">{t("icon.uploadTitle")}</div>
          <div className="icon-upload">
            <input
              ref={fileRef}
              type="file"
              accept="image/png,image/jpeg"
              data-icon-upload-input
              className="icon-upload__input"
              onChange={(event) => {
                void upload(event.target.files?.[0]);
                event.target.value = "";
              }}
            />
            <button type="button" className="pill-outline" data-action="icon-upload-pick" disabled={uploading} onClick={() => fileRef.current?.click()}>
              <img src={icons.download} alt="" />
              {uploading ? t("icon.uploading") : t("icon.uploadPick")}
            </button>
            <span className="row__hint">{t("icon.uploadHint")}</span>
            {current.kind === "custom" ? (
              <button
                type="button"
                className="pill-outline"
                data-action="icon-delete"
                onClick={() => {
                  onChange({ kind: "none", gradient, id: undefined, path: undefined, dataUrl: undefined });
                  notify(t("icon.deleted"));
                }}
              >
                <img src={icons.trash} alt="" />
                {t("icon.delete")}
              </button>
            ) : null}
          </div>
          <div className="row__hint">{t("icon.builtinLocked")}</div>
        </section>
      </div>
    </Modal>
  );
}
