fn main() {
    // 构建时间：更新检查里「版本号解析不出来就用发布时间兜底」要用它比较
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=GUGU_BUILD_TIME={now}");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=installer-hooks.nsh");

    // 不再注入任何凭据：微软登录的 client id 由用户在「设置 → 内容服务」里自填，
    // 存在 .mc/settings.json，编译期与源码里都没有它。
    tauri_build::build();
}
