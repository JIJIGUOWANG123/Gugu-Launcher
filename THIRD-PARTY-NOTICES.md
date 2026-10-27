# 第三方组件与许可（THIRD-PARTY-NOTICES）

咕咕启动器本身以 MIT 许可发布。以下组件随本工程源码或安装包分发，各自遵循其原始许可。

## 1. skinview3d 3.4.2

- 项目：<https://github.com/bs-community/skinview3d>
- 许可：MIT，Copyright (c) 2014-2018 Kent Rasmussen
- 并入位置：`src/renderer/skinview/`（animation.ts / model.ts / nametag.ts / skinview3d.ts / viewer.ts，
  以及 assets/minecraft.woff2）
- 原始许可文件同时保留在 `src/renderer/skinview/LICENSE`
- 改动说明：仅作为源码保存在工程内，未修改逻辑；M1 不参与前端构建（tsconfig 已 exclude）。

## 2. three.js

- 项目：<https://threejs.org/>
- 许可：MIT
- 状态：skinview3d 的 peer 依赖。接入皮肤预览页时再通过 npm 安装（不并入源码）。


## 3. Terracotta（陶瓦联机）0.4.2

- 项目：<https://github.com/burningtnt/Terracotta>
- 许可：**AGPL-3.0**（Copyright (c) burningtnt 及 Terracotta 贡献者）
- 分发位置：`src-tauri/resources/terracotta/`
  - `terracotta.exe`（v0.4.2，Windows x86_64，9,402,880 字节，
    sha256 `74c10568a7fea9c1d38cf8d2d4ca90baf1517f8e5a26c63d3349db70bc449796`）
  - `LICENSE-AGPL-3.0.txt`（AGPL-3.0 许可证全文，与上游 LICENSE 完全一致）
  - `SOURCE.txt`（项目名、版本、上游地址、源码获取方式）
  - `VCRUNTIME140.DLL`（97,656 字节，sha256
    `475ab98b7722e965bd38c8fa6ed23502309582ccf294ff1061cb290c7988f0d1`，
    Microsoft Visual C++ 运行库，随上游发布包一起提供，原样分发未做修改）
- 用途：**仅作为独立进程调用**（`terracotta.exe --hmcl <port 文件>`），
  通过它的本地 HTTP 接口读取状态。本工程**不链接、不修改**该二进制，
  也不把它并入 Rust 代码；运行时首次启动会把内嵌副本复制到
  `<数据根>/terracotta/` 再执行。
- 按 AGPL-3.0 要求，安装包内随附许可证全文与源码获取方式；
  上游源码亦可直接从上面的 GitHub 地址获取（v0.4.2 标签）。
- 备注：该二进制由上游以 tar.gz 发布，包内的 `VCRUNTIME140.DLL` 属于
  Microsoft Visual C++ 可再发行组件，同样原样分发，未做修改；缺少 VC 运行库的
  机器上 terracotta.exe 无法启动，所以本启动器把它一起内嵌并在首次运行时
  复制到 `<数据根>/terracotta/`（与 exe 一起校验 sha256）。

## 4. cubiomes（种子地图世界生成计算）

- 项目：<https://github.com/Cubitect/cubiomes>
- 许可：**MIT**（Copyright (c) Cubitect）
- 分发位置：`src-tauri/third_party/cubiomes/`（C 源码、查表头文件、LICENSE、PROVENANCE.md；
  另含本工程新增的 `seedmap_glue.c` / `seedmap_glue.h` 胶水层，未修改上游任何 C 源）
- 来源与版本：见同目录 `PROVENANCE.md`（上游 master 快照，支持到 Minecraft 26.3）
- 用途：在本地计算指定种子的生物群系与结构位置（`src-tauri/src/seedmap.rs` 通过 FFI 调用），
  计算结果只用于界面绘制，不联网、不上传。
- 编译方式：`build.rs` 用 `cc` 把 8 个 .c 编成静态库 `cubiomes_seedmap` 后链接进主程序。

## 5. 图标与皮肤素材

- `src/assets/icons/`、`src/assets/skins/` 里的 png 来自本工程 `图片素材/`（自有素材）；
  其中 `src/assets/skins/` 的 9 套默认皮肤贴图取自 Minecraft 1.20.1 客户端 jar 中的官方资源，
  仅用于皮肤预览与默认外观，未再分发游戏本体。
- `src-tauri/icons/` 里的应用图标由 `图片素材/logo/LOGO.png` 生成（自有素材）。

- src/assets/icons/ 里的 png 来自本工程 图片素材/UI素材（自有素材），已缩放到 128px 以控制包体。
- src-tauri/icons/ 里的应用图标是 M1 占位图（脚本生成），后续替换为正式美术素材。
