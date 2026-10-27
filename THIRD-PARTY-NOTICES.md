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


## 3. 图标素材

- src/assets/icons/ 里的 png 来自本工程 图片素材/UI素材（自有素材），已缩放到 128px 以控制包体。
- src-tauri/icons/ 里的应用图标是 M1 占位图（脚本生成），后续替换为正式美术素材。
