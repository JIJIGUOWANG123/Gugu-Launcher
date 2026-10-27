import { useEffect, useRef, useState } from "react";
import { IdleAnimation, PlayerAnimation, RunningAnimation, WalkingAnimation } from "../renderer/skinview/animation";
import { SkinViewer } from "../renderer/skinview/viewer";
import { useT } from "../lib/i18n";
import type { SkinModel } from "../lib/types";
import { HeadAvatar } from "./HeadAvatar";

export type SkinAction = "idle" | "walk" | "run" | "none";

interface Props {
  /** 皮肤 PNG 的 data URL（或 http 地址）；null 表示还没选 */
  skin: string | null;
  model: SkinModel;
  action: SkinAction;
  autoRotate: boolean;
  /** 深色预览背景 */
  dark: boolean;
  cape: string | null;
  /** 预览角落的小字提示（离线/第三方账号不支持披风时用） */
  capeNote?: string | null;
  /** 每加一就重置到默认机位（「重置视角」按钮用） */
  resetToken?: number;
}

function animationFor(action: SkinAction): PlayerAnimation | null {
  if (action === "idle") return new IdleAnimation();
  if (action === "walk") return new WalkingAnimation();
  if (action === "run") return new RunningAnimation();
  return null;
}

/**
 * 3D 皮肤预览：把并入的 skinview3d 包一层。
 * - 拖动旋转 / 滚轮缩放由 SkinViewer 内部的 OrbitControls 负责，不再自己写；
 * - 尺寸跟着容器走（ResizeObserver），三档宽度都不用另写断点；
 * - 环境没有 WebGL 时回落到 2D 头像，界面不至于空一块；
 * - preserveDrawingBuffer 打开，自检脚本可以直接读 canvas 像素。
 */
export function SkinViewer3D({ skin, model, action, autoRotate, dark, cape, capeNote, resetToken = 0 }: Props) {
  const t = useT();
  const hostRef = useRef<HTMLDivElement | null>(null);
  const viewerRef = useRef<SkinViewer | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let viewer: SkinViewer;
    try {
      viewer = new SkinViewer({
        width: Math.max(host.clientWidth, 200),
        height: Math.max(host.clientHeight, 260),
        preserveDrawingBuffer: true,
      });
    } catch {
      setFailed(true);
      return;
    }
    viewer.canvas.style.width = "100%";
    viewer.canvas.style.height = "100%";
    viewer.canvas.style.display = "block";
    host.appendChild(viewer.canvas);
    viewerRef.current = viewer;

    const observer = new ResizeObserver(() => {
      viewer.width = Math.max(host.clientWidth, 1);
      viewer.height = Math.max(host.clientHeight, 1);
    });
    observer.observe(host);

    return () => {
      observer.disconnect();
      viewerRef.current = null;
      viewer.dispose();
      if (viewer.canvas.parentElement === host) host.removeChild(viewer.canvas);
    };
  }, []);

  useEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer || !skin) return;
    // ModelType 是 skinview-utils 的联合类型（"default" | "slim"），default 即经典模型
    const modelType = model === "slim" ? "slim" : "default";
    void Promise.resolve(viewer.loadSkin(skin, { model: modelType, ears: "load-only" })).catch(() => undefined);
  }, [skin, model]);

  useEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer) return;
    if (!cape) {
      viewer.resetCape();
      return;
    }
    void Promise.resolve(viewer.loadCape(cape)).catch(() => undefined);
  }, [cape]);

  useEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer) return;
    viewer.animation = animationFor(action);
  }, [action]);

  useEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer) return;
    viewer.autoRotate = autoRotate;
  }, [autoRotate]);

  useEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer) return;
    viewer.background = dark ? "#1f232c" : "#e9edf3";
  }, [dark]);

  useEffect(() => {
    if (resetToken === 0) return;
    viewerRef.current?.resetCameraPose();
  }, [resetToken]);

  return (
    <div className={"skin-stage" + (dark ? " skin-stage--dark" : "")}>
      <div className="skin-stage__canvas" ref={hostRef} data-skin-canvas="3d">
        {failed ? (
          <div className="skin-stage__fallback">
            <HeadAvatar skin={skin} size={96} />
            <span>{t("skin.previewFailed")}</span>
          </div>
        ) : null}
      </div>
      <span className="skin-stage__hint">{t("skin.hintDrag")}</span>
      {capeNote ? (
        <span className="skin-stage__note" data-cape-note>
          {capeNote}
        </span>
      ) : null}
    </div>
  );
}
