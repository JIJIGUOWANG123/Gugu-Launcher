# skinview3d 源码并入说明

- 来源：skinview3d 3.4.2（<https://github.com/bs-community/skinview3d>）
- 许可：MIT，Copyright (c) 2014-2018 Kent Rasmussen（见同目录 `LICENSE`）
- 并入文件：`animation.ts` / `model.ts` / `nametag.ts` / `skinview3d.ts` / `viewer.ts` /
  `assets/minecraft.woff2`
- 改动：**无**。原样拷贝，未修改逻辑；原库本身不在文件里写 MIT 头注释，许可信息靠 `LICENSE` +
  仓库根的 `THIRD-PARTY-NOTICES.md` 体现。
- 状态：M1 只把源码并入工程，**不参与前端构建**（`tsconfig.json` 已 exclude 该目录）。
- 依赖：`three`（^0.156）与 `skinview-utils` 在接入皮肤预览页时再通过 npm 安装。
