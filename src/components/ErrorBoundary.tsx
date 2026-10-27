import { Component, type ErrorInfo, type ReactNode } from "react";
import { translate } from "../lib/i18n";

interface Props {
  children: ReactNode;
  /** 出错时「返回主页」要跳哪 */
  onHome: () => void;
  /** 页面名（中文），只用于错误摘要里的定位信息 */
  pageName?: string;
}

interface State {
  message: string;
  stack: string;
}

/**
 * 页面级错误边界：任何单个页面渲染炸了都在这里兜住，
 * 顶栏（导航 / 主题 / 窗口按钮）和 toast 不受影响，用户还能换页或关窗口。
 *
 * 必须是 class 组件：React 只有 getDerivedStateFromError / componentDidCatch 这条兜底路径。
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { message: "", stack: "" };

  static getDerivedStateFromError(error: unknown): State {
    const err = error instanceof Error ? error : new Error(String(error));
    return { message: err.message || String(error), stack: err.stack ?? "" };
  }

  componentDidCatch(error: unknown, info: ErrorInfo): void {
    // 真机上没有 devtools，把它打到控制台，自检脚本用 Runtime.exceptionThrown / console 抓
    console.error("[页面崩溃]", this.props.pageName ?? "", error, info.componentStack);
  }

  render() {
    if (!this.state.message && !this.state.stack) return this.props.children;
    return (
      <section className="card page-error" data-page-error data-page-error-name={this.props.pageName ?? ""}>
        <div className="detail-card__title">{translate("zh-CN", "error.pageTitle")}</div>
        <div className="row__hint">{translate("zh-CN", "error.pageHint")}</div>
        <pre className="page-error__detail mono" data-page-error-message>
          {this.props.pageName ? this.props.pageName + "：" : ""}
          {this.state.message}
        </pre>
        {this.state.stack ? (
          <details className="page-error__stack">
            <summary>{translate("zh-CN", "error.pageStack")}</summary>
            <pre className="mono" data-page-error-stack>
              {this.state.stack.split("\n").slice(0, 8).join("\n")}
            </pre>
          </details>
        ) : null}
        <div className="page-error__actions">
          <button type="button" className="pill-button" data-action="page-error-home" onClick={this.props.onHome}>
            {translate("zh-CN", "error.pageHome")}
          </button>
        </div>
      </section>
    );
  }
}
