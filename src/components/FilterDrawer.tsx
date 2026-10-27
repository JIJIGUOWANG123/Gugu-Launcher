import { useEffect, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { icons } from "../assets/icons";
import { useT } from "../lib/i18n";

interface Props {
  title: string;
  onClose: () => void;
  /** 底部固定的一行按钮（重置 / 完成） */
  footer: ReactNode;
  children: ReactNode;
}

/**
 * 右侧筛选抽屉：半透明遮罩 + 贴右边缘的整高面板（不是居中小弹窗）。
 * 点遮罩或 ✕ 关闭，Esc 也能关；抽屉本身点不穿。
 */
export function FilterDrawer({ title, onClose, footer, children }: Props) {
  const t = useT();

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // 挂到 body：祖先上的 filter/backdrop-filter 会让 position: fixed 失效，抽屉必须贴着视口右边缘
  return createPortal(
    <div className="drawer-overlay" role="presentation" onClick={onClose}>
      <aside
        className="filter-drawer"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        data-filter-drawer
        onClick={(event) => event.stopPropagation()}
      >
        <header className="filter-drawer__head">
          <h3 className="filter-drawer__title">{title}</h3>
          <button type="button" className="icon-button" data-action="drawer-close" title={t("common.close")} onClick={onClose}>
            <img src={icons.close} alt="" />
          </button>
        </header>
        <div className="filter-drawer__body">{children}</div>
        <footer className="filter-drawer__foot">{footer}</footer>
      </aside>
    </div>,
    document.body,
  );
}
