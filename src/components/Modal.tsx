import { useEffect, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { icons } from "../assets/icons";
import { useT } from "../lib/i18n";

interface Props {
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  /** 需要更多横向空间的弹窗（例如披风网格）加个修饰类 */
  className?: string;
}

/**
 * 悬浮窗：用 portal 挂到 document.body。
 *
 * 为什么必须 portal：模糊是加在外壳（.app-body / .topbar）上的 filter: blur()，
 * 而 filter 会作用于整棵子树。如果弹窗渲染在页面组件里（.app-body 内），
 * 弹窗自己也会被糊掉、还会继承 pointer-events: none 点不动。
 * 挂到 body 后：底层外壳模糊、弹窗清晰可交互。
 *
 * 同时给 body 加 modal-open，由 CSS 负责模糊底层内容。
 */
export function Modal({ title, onClose, children, footer, className }: Props) {
  const t = useT();

  useEffect(() => {
    document.body.classList.add("modal-open");
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      document.body.classList.remove("modal-open");
      window.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  return createPortal(
    <div className="modal-overlay" role="dialog" aria-modal="true" aria-label={title} onClick={onClose}>
      <div className={"modal" + (className ? " " + className : "")} onClick={(event) => event.stopPropagation()}>
        <div className="modal__head">
          <h3 className="modal__title">{title}</h3>
          <button className="icon-button" title={t("common.cancel")} onClick={onClose}>
            <img src={icons.close} alt="" />
          </button>
        </div>
        <div className="modal__body">{children}</div>
        {footer ? <div className="modal__foot">{footer}</div> : null}
      </div>
    </div>,
    document.body,
  );
}
