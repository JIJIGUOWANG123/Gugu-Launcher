import { FunGroups } from "../components/FunSettings";
import { useT } from "../lib/i18n";
import type { Settings } from "../lib/types";

interface Props {
  settings: Settings;
  dataRoot: string;
  onChange: (patch: Partial<Settings>) => void;
  notify: (message: string) => void;
}

/** 娱乐页：今日人品 / 每日一言 / GitHub 加速下载器（从设置里独立出来）。 */
export function FunPage({ settings, dataRoot, onChange, notify }: Props) {
  const t = useT();
  return (
    <div className="page" data-page-root="fun">
      <h1 className="page-title">{t("nav.fun")}</h1>
      <FunGroups settings={settings} dataRoot={dataRoot} onChange={onChange} onNotify={notify} visible={() => true} />
    </div>
  );
}
