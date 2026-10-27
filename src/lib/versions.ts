/**
 * 版本清单的字段兼容层。
 * 后端可能给 camelCase（versionType）、Mojang 原样（type）或 snake_case（version_type），
 * 所有页面一律走这里，避免「一个字段名不对整页版本列表变空」。
 */
import type { VersionManifest, VersionSummary } from "./types";

type Loose = VersionSummary & Record<string, unknown>;

function pickString(source: Record<string, unknown>, keys: string[]): string {
  for (const key of keys) {
    const value = source[key];
    if (typeof value === "string" && value.length > 0) return value;
  }
  return "";
}

/** 正式版 / 快照 / 旧测试版 / 旧 alpha：取到小写字符串。 */
export function versionTypeOf(version: VersionSummary): string {
  const raw = pickString(version as unknown as Record<string, unknown>, ["versionType", "type", "version_type"]);
  return raw.toLowerCase();
}

export function isRelease(version: VersionSummary): boolean {
  return versionTypeOf(version) === "release";
}

export function releaseTimeOf(version: VersionSummary): string {
  return pickString(version as unknown as Record<string, unknown>, ["releaseTime", "release_time", "time", "published"]);
}

/** 愚人节快照的显式 id 清单（Mojang 每年 4 月 1 日发的整活版本）。 */
export const APRIL_FOOLS_IDS = [
  "15w14a",
  "1.RV-Pre1",
  "3D Shareware v1.34",
  "20w14infinite",
  "22w13oneblockatatime",
  "23w13a_or_b",
  "23w13a_or_b_original_mode",
  "24w14potato",
  "25w14craftmine",
];

const APRIL_FOOLS_REGEX = /^d{2}wd{2}[a-z].*(potato|oneblock|craftmine|or_b|infinite|shareware|rv-pre)/i;
const APRIL_FOOLS_HINTS = ["shareware", "rv-pre", "3d"];

/** 是不是愚人节快照：先看显式清单，再用正则/关键字兜底。 */
export function isAprilFools(version: VersionSummary): boolean {
  const id = (version.id ?? "").trim();
  if (!id) return false;
  if (APRIL_FOOLS_IDS.some((item) => item.toLowerCase() === id.toLowerCase())) return true;
  if (APRIL_FOOLS_REGEX.test(id)) return true;
  const lower = id.toLowerCase();
  return APRIL_FOOLS_HINTS.some((hint) => lower.includes(hint));
}

/** 四类：正式版 / 测试版（不含愚人节）/ 愚人节版 / 远古版。 */
export type VersionKind = "release" | "snapshot" | "april" | "old";

export interface ClassifiedVersions {
  release: VersionSummary[];
  snapshot: VersionSummary[];
  april: VersionSummary[];
  old: VersionSummary[];
  counts: Record<VersionKind, number>;
  /** 四类按时间从新到旧合并（弹窗列表用这个顺序） */
  all: Array<{ version: VersionSummary; kind: VersionKind }>;
}

export function classifyVersions(list: VersionSummary[] | null | undefined): ClassifiedVersions {
  const release: VersionSummary[] = [];
  const snapshot: VersionSummary[] = [];
  const april: VersionSummary[] = [];
  const old: VersionSummary[] = [];
  const tagged: Array<{ version: VersionSummary; kind: VersionKind }> = [];
  for (const version of list ?? []) {
    const type = versionTypeOf(version);
    let kind: VersionKind;
    if (type === "release") kind = "release";
    else if (type === "old_beta" || type === "old_alpha") kind = "old";
    else if (type === "snapshot") kind = isAprilFools(version) ? "april" : "snapshot";
    else kind = "release";
    if (kind === "release") release.push(version);
    else if (kind === "snapshot") snapshot.push(version);
    else if (kind === "april") april.push(version);
    else old.push(version);
    tagged.push({ version, kind });
  }
  const byVersionTime = (a: VersionSummary, b: VersionSummary) =>
    releaseTimeOf(b).localeCompare(releaseTimeOf(a));
  const byTaggedTime = (a: { version: VersionSummary }, b: { version: VersionSummary }) =>
    releaseTimeOf(b.version).localeCompare(releaseTimeOf(a.version));
  release.sort(byVersionTime);
  snapshot.sort(byVersionTime);
  april.sort(byVersionTime);
  old.sort(byVersionTime);
  tagged.sort(byTaggedTime);
  return {
    release,
    snapshot,
    april,
    old,
    counts: { release: release.length, snapshot: snapshot.length, april: april.length, old: old.length },
    all: tagged,
  };
}

/** 只留正式版，按时间从新到旧。 */
export function releaseVersions(manifest: VersionManifest | null | undefined): VersionSummary[] {
  const list = manifest?.versions ?? [];
  return list
    .filter(isRelease)
    .slice()
    .sort((a, b) => releaseTimeOf(b).localeCompare(releaseTimeOf(a)));
}

/**
 * 把后端的清单折成页面能直接用的形状：
 * 补齐 versionType / releaseTime，缺 latestRelease 时用最新正式版兜底。
 */
export function normalizeManifest(raw: VersionManifest | null | undefined): VersionManifest | null {
  if (!raw) return null;
  const list = (raw.versions ?? []).map((item) => {
    const loose = item as Loose;
    const versionType = versionTypeOf(item) || "release";
    return {
      ...item,
      versionType,
      releaseTime: releaseTimeOf(item) || (typeof loose.time === "string" ? loose.time : ""),
      url: typeof item.url === "string" ? item.url : "",
    } as VersionSummary;
  });
  const releases = list.filter((item) => (item.versionType ?? "").toLowerCase() === "release");
  const sortedReleases = releases.slice().sort((a, b) => (b.releaseTime ?? "").localeCompare(a.releaseTime ?? ""));
  const latestRelease =
    raw.latestRelease ||
    raw.latest?.release ||
    sortedReleases[0]?.id ||
    "";
  const latestSnapshot = raw.latestSnapshot || raw.latest?.snapshot || "";
  return {
    ...raw,
    versions: list,
    latestRelease,
    latestSnapshot,
    fetchedAt: raw.fetchedAt ?? null,
    fromCache: Boolean(raw.fromCache),
  };
}
