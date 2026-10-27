fn main() {
    // 构建时间：更新检查里「版本号解析不出来就用发布时间兜底」要用它比较
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=GUGU_BUILD_TIME={now}");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=installer-hooks.nsh");

    build_cubiomes();

    // 不再注入任何凭据：微软登录的 client id 由用户在「设置 → 内容服务」里自填，
    // 存在 .mc/settings.json，编译期与源码里都没有它。
    tauri_build::build();
}

/// 种子地图要用 cubiomes（C，MIT）：用 cc crate 直接编进 exe，
/// 运行时不需要任何外部 exe 或 DLL。源码见 third_party/cubiomes/。
fn build_cubiomes() {
    const DIR: &str = "third_party/cubiomes";
    // 种子地图需要的这一套（loot/quadbase/tests/xradv 是上游的可选工具与战利品模块，
    // finders/util 没有引用它们的实现，见 PROVENANCE.md）
    const SOURCES: [&str; 15] = [
        "noise.c",
        "biomes.c",
        "layers.c",
        "biomenoise.c",
        "terrainnoise.c",
        "generator.c",
        "finders.c",
        "util.c",
        "features/abandoned_camp.c",
        "features/end_city.c",
        "features/fortress.c",
        "features/mineshaft.c",
        "features/stronghold.c",
        "seedmap_compat.c",
        "seedmap_glue.c",
    ];

    let mut build = cc::Build::new();
    build
        .include(DIR)
        .files(SOURCES.iter().map(|name| format!("{DIR}/{name}")))
        // cubiomes 依赖有符号整数回绕（哈希与随机数移植），不是 UB 假设
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-sign-compare")
        .warnings(false)
        .compile("cubiomes_seedmap");

    println!("cargo:rerun-if-changed={DIR}");
}
