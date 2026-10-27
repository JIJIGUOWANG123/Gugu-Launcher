import { useState } from "react";
import { Modal } from "./Modal";
import { formatBytes } from "../lib/format";
import { useT } from "../lib/i18n";
import { api } from "../lib/tauri-api";
import type { ExternalCandidate, ExternalScanResult } from "../lib/types";

interface Props {
  onClose: () => void;
  onImported: () => Promise<void> | void;
  onGoInstances: () => void;
  notify: (message: string) => void;
}

type Step = "pick" | "list" | "done";

/** 从 HMCL / PCL2 导入实例：选目录 → 勾选版本 → 引用/复制导入。 */
export function ImportExternalModal({ onClose, onImported, onGoInstances, notify }: Props) {
  const t = useT();
  const [step, setStep] = useState<Step>("pick");
  const [root, setRoot] = useState("");
  const [scan, setScan] = useState<ExternalScanResult | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [mode, setMode] = useState<"link" | "copy">("link");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ ok: number; failed: Array<{ id: string; reason: string }> } | null>(null);

  const pickDirectory = async () => {
    setError("");
    try {
      const dir = await api.pickDirectory();
      if (!dir) {
        notify(t("import.pickCanceled"));
        return;
      }
      setRoot(dir);
      await runScan(dir);
    } catch (err) {
      setError(String(err instanceof Error ? err.message : err));
    }
  };

  const runScan = async (dir: string) => {
    setBusy(true);
    try {
      const scanned = await api.externalScan({ root: dir });
      setScan(scanned);
      setSelected(new Set(scanned.candidates.filter((item) => !item.broken).map((item) => item.id)));
      setStep("list");
    } catch (err) {
      setScan(null);
      setError(String(err instanceof Error ? err.message : err));
    } finally {
      setBusy(false);
    }
  };

  const toggle = (candidate: ExternalCandidate) => {
    if (candidate.broken) return;
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(candidate.id)) next.delete(candidate.id);
      else next.add(candidate.id);
      return next;
    });
  };

  const doImport = async () => {
    if (selected.size === 0) {
      setError(t("import.noneSelected"));
      return;
    }
    setBusy(true);
    setError("");
    try {
      const imported = await api.externalImport({ root, ids: Array.from(selected), mode });
      setResult({ ok: imported.instances.length, failed: imported.failed });
      setStep("done");
      await onImported();
    } catch (err) {
      setError(String(err instanceof Error ? err.message : err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={t("import.title")}
      onClose={onClose}
      footer={
        step === "list" ? (
          <>
            <button type="button" className="pill-outline" onClick={() => { setStep("pick"); setScan(null); }}>
              {t("import.reselect")}
            </button>
            <button type="button" className="pill-button" data-action="import-start" disabled={busy || selected.size === 0} onClick={() => void doImport()}>
              {busy ? t("import.importing") : t("import.start", { count: selected.size })}
            </button>
          </>
        ) : step === "done" ? (
          <>
            <button type="button" className="pill-outline" onClick={onClose}>
              {t("common.close")}
            </button>
            <button type="button" className="pill-button" data-action="import-go-instances" onClick={onGoInstances}>
              {t("import.goInstances")}
            </button>
          </>
        ) : (
          <button type="button" className="pill-outline" onClick={onClose}>
            {t("common.cancel")}
          </button>
        )
      }
    >
      {step === "pick" ? (
        <div className="import-pick" data-import-step="pick">
          <button type="button" className="pill-button" data-action="import-pick" disabled={busy} onClick={() => void pickDirectory()}>
            {busy ? t("import.scanning") : t("import.pickDir")}
          </button>
          <div className="row__hint">{t("import.pickHint")}</div>
          {root ? <div className="mono">{root}</div> : null}
          {error ? (
            <div className="field-error" data-import-error>
              {error}
            </div>
          ) : null}
        </div>
      ) : null}

      {step === "list" && scan ? (
        <div className="import-list" data-import-step="list">
          <div className="row__hint" data-import-summary>
            {t("import.detected", { launcher: scan.launcher === "pcl2" ? "PCL2" : scan.launcher === "hmcl" ? "HMCL" : t("import.unknown"), count: scan.candidates.length })}
          </div>
          <div className="import-rows">
            {scan.candidates.map((candidate) => {
              const disabled = Boolean(candidate.broken);
              return (
                <label
                  key={candidate.id}
                  className={"import-row" + (disabled ? " is-broken" : "") + (selected.has(candidate.id) ? " is-active" : "")}
                  data-import-row={candidate.id}
                  data-import-broken={disabled ? "1" : "0"}
                >
                  <input type="checkbox" checked={selected.has(candidate.id)} disabled={disabled} onChange={() => toggle(candidate)} />
                  <span className="import-row__meta">
                    <span className="import-row__name">{candidate.name}</span>
                    <span className="chip chip--tiny">{candidate.mcVersion}</span>
                    <span className="chip chip--tiny">{candidate.loader || "Vanilla"}</span>
                    <span className="chip chip--tiny">{candidate.isolated ? t("import.isolated") : t("import.shared")}</span>
                    {candidate.hasMods ? <span className="chip chip--tiny">{t("import.mods", { count: candidate.modCount ?? 0 })}</span> : null}
                    <span className="row__hint">{formatBytes(candidate.sizeBytes)}</span>
                  </span>
                  {disabled ? <span className="import-row__broken">{candidate.broken}</span> : null}
                </label>
              );
            })}
          </div>
          <div className="import-mode">
            <span className="row__hint">{t("import.mode")}</span>
            <label>
              <input type="radio" name="import-mode" checked={mode === "link"} data-import-mode="link" onChange={() => setMode("link")} />
              {t("import.modeLink")}
            </label>
            <label>
              <input type="radio" name="import-mode" checked={mode === "copy"} data-import-mode="copy" onChange={() => setMode("copy")} />
              {t("import.modeCopy")}
            </label>
          </div>
          {error ? (
            <div className="field-error" data-import-error>
              {error}
            </div>
          ) : null}
        </div>
      ) : null}

      {step === "done" && result ? (
        <div className="import-done" data-import-step="done">
          <div className="row__hint" data-import-result>
            {t("import.result", { ok: result.ok, fail: result.failed.length })}
          </div>
          {result.failed.length > 0 ? (
            <ul className="import-failed">
              {result.failed.map((item) => (
                <li key={item.id}>
                  {item.id}：{item.reason}
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      ) : null}
    </Modal>
  );
}
