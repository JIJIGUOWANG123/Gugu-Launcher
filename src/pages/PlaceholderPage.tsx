import { icons } from "../assets/icons";
import { useT } from "../lib/i18n";

interface Props {
  titleKey: string;
}

/** 其余 8 个页面在 M1 只有占位骨架，明确标注未实现，避免看起来"点坏了"。 */
export function PlaceholderPage({ titleKey }: Props) {
  const t = useT();
  return (
    <div>
      <h1 className="page-title">{t(titleKey)}</h1>
      <p className="page-subtitle">{t("common.notImplemented")}</p>
      <section className="card card-row">
        <img src={icons.alert} alt="" width={20} height={20} />
        <span className="mono">{titleKey}</span>
      </section>
    </div>
  );
}
