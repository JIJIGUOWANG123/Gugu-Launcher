# 咕咕启动器 / Gugu Launcher

**仓库：** <https://github.com/JIJIGUOWANG123/Gugu-Launcher>

Minecraft 开源无广告启动器。技术栈：**Rust + Tauri 2 + React 18 / TypeScript / Vite**。

> 当前进度：**M1 工程骨架**（可构建、可编译、模块接口已留好）。启动内核、下载器、皮肤预览等能力在后续里程碑接入。

---

## 目录结构

```text
D:\minecraft\
├─ package.json              # 前端工程（npm 兼容，pnpm 也可）
├─ vite.config.ts            # dev 端口固定 1420，与 tauri.conf.json 的 devUrl 对齐
├─ tsconfig.json
├─ index.html
├─ src\                      # 前端（React + TS）
│  ├─ App.tsx                # 壳：标题栏 + 左侧导航 + 内容区
│  ├─ main.tsx
│  ├─ components\            # TitleBar / SideNav / Modal / InstanceCard
│  ├─ pages\                 # 主页、实例页、设置页、占位页
│  ├─ lib\                   # i18n.ts / tauri-api.ts / theme.ts / types.ts
│  ├─ styles\                # theme.css（浅/深两套变量）+ app.css
│  ├─ assets\icons\          # 从 图片素材\UI素材 拷来的 png（已缩到 128px）
│  └─ renderer\skinview\     # 并入的 skinview3d 源码（M1 只存源，不参与构建）
├─ src-tauri\                # Rust / Tauri 2
│  ├─ Cargo.toml
│  ├─ build.rs
│  ├─ tauri.conf.json
│  ├─ capabilities\default.json
│  ├─ icons\                 # 应用图标（占位，后续换美术素材）
│  └─ src\                   # paths / settings / db / meta / download / instance /
│                            # launch / java / net / update / commands / state
├─ 源代码\                    # 导出给 GitHub 的源码快照（留空，发版时导出）
├─ 上传GitHub版本\            # 上传用发布包（文件名不带版本号）
├─ 备份安装包\<版本号>\        # 安装包归档
├─ 所有版本备份\<版本号>\      # 每个版本的全量备份
└─ 测试代码\                  # 自检脚本
```

## 数据目录 `<安装目录>\.mc`

首次启动自动创建，全部运行期数据都在这里（不进仓库、不进安装包）：

| 目录 | 用途 |
| --- | --- |
| `assets\` | 游戏资源对象（按 sha1 分桶） |
| `libraries\` | 依赖库 |
| `versions\` | 版本 JSON 与客户端 jar |
| `instances\` | 每个实例一个目录（含 `.minecraft\`） |
| `runtimes\` | 自动下载的 Java 运行时 |
| `skins\` | 皮肤缓存 |
| `game-icons\` | 实例图标 |
| `cache\` | 临时缓存 |
| `logs\` | 启动器与游戏日志 |
| `diagnostics\` | 诊断报告 |
| `settings.json` | 设置文件 |
| `gugu.db` | SQLite（instances / accounts / tasks） |
| `version_manifest.json` | 版本清单缓存 |

开发期（`cfg!(debug_assertions)`）数据根固定为仓库根的 `D:\minecraft\.mc`；
发布版是 exe 所在目录下的 `.mc`；也可用环境变量 `GUGU_DATA_ROOT` 整体覆盖。

## 开发命令

```powershell
# 主用 pnpm（本机没有 npm；tauri.conf.json 的 beforeDevCommand/beforeBuildCommand 已写成 pnpm）
pnpm install                     # 安装前端依赖
pnpm run dev                     # 前端 dev server（127.0.0.1:1420），浏览器里自动用 mock 数据
pnpm run build                   # tsc --noEmit + vite build → dist\
pnpm tauri:dev                   # 起 Tauri 窗口（需要 MSVC 构建工具 + Windows SDK）
pnpm tauri:build                 # 打包安装包（NSIS，M5 阶段完善）

cd src-tauri
cargo check                      # 只做类型检查
cargo test                       # 单元测试 41 项（源表 / 路径 / 设置 / 字体 / 版本号 / 资产选择等）
cargo build --release --features custom-protocol   # 直接产出 target\release\GuguLauncher.exe（内嵌前端）
```

> **npm 等价命令**（给装了 npm 的机器用）：`npm install` / `npm run dev` / `npm run build` /
> `npm run tauri:dev` / `npm run tauri:build`。本机 DSH 运行时只带 node + pnpm，所以配置里用的是 pnpm。
> 若某台机器 npm/pnpm 都没有，也可以用 `node node_modules/vite/bin/vite.js build` 直接构建。

> `src-tauri/.cargo/config.toml` 把 crates.io 换成了 USTC 镜像（本机访问 index.crates.io 单次 15s+）。
> 不需要的话直接删掉这个文件，Cargo 会回落到官方源。

> **本机环境注意**：这台机器上 `node` 与 `pnpm` 都不在 PATH 里（DSH 运行时以完整路径提供）。
> 直接跑 `pnpm run build` 会报 `'node' is not recognized`（脚本内部要调 node）。
> 两种做法：① 把 node 目录加进 PATH 再跑（本机实测通过）；② 用 `测试代码\run-frontend-check.ps1`，
> 它直接用完整路径调 node 与 vite。装了标准 Node 的机器不会有这个问题。

## 界面结构

照参照图做的是**顶部导航**（没有侧栏，内容区占满宽度）：

- 左：Logo + 「咕咕启动器 / GUGU LAUNCHER」双标题
- 中：主页 / 实例 / 多人 / 资源 / 账号 / 皮肤 / 关于（当前页高亮 + `aria-current="page"`）
- 右：主题切换（月亮/太阳）→ 下载中心（有下载时右上角角标）→ 设置齿轮 → 最小化/最大化/关闭
- 窗口按钮自绘，对应 `tauri.conf.json` 的 `decorations: false`；标题栏用 `data-tauri-drag-region` 拖动
- 响应式三档：≤1180px 收字号与间距、隐藏英文副标题；≤1024px 导航只留图标；≤900px 连中文标题也收起

## 设置（.mc\settings.json）

- **界面仅支持简体中文**（`language` 字段保留但只允许 `zh-CN`，其它值读盘时回落）。
- 字体可从系统中文体里选：设置 → 常规 → 显示字体。候选由 `fonts_list()` 枚举，
  用 cmap 真查「中 / 国 / 文」三个字形，不靠字体名猜；默认 **Microsoft YaHei（微软雅黑）**，
  回落链 `Microsoft YaHei → 微软雅黑 → Microsoft YaHei UI → 列表第一个 → system-ui`。
- 主题（浅/深）、主题色 `accent`、下载源、内存上限等字段同文件保存，写入前做取值校验。

## 微软登录 Client ID（用户自填）

微软登录用的 app client id **不再随安装包内置**，改由用户在「设置 → 内容服务」里自己填，
存在 `.mc\settings.json` 的 `msClientId` 字段里（`src-tauri/build.rs` 不再注入任何凭据）。

- 格式：标准 UUID（`8-4-4-4-12`）。填了合法值 → `ms_auth_status` 返回 `configured: true`；
  留空 → `false`（登录按钮置灰）；填了但格式不对 → 读盘回落为空并返回 `invalid: true`，
  提示「Client ID 格式不对：应为 8-4-4-4-12 的 UUID」。
- 未配置时 `ms_login_start` 直接返回，**不发任何网络请求**。
- 日志与错误信息里的 client id 一律打码（只显示前 8 位，如 `11111111…`）。
- 泄露防线：`测试代码\check-credentials.mjs` 检查「源码/快照里没有 `.ms-client-id` 文件、
  没有 client id 形态的 UUID、没有 `GUGU_MS_CLIENT_ID` 注入路径」，并检查
  `.mc\logs\*.log` 里不出现 `settings.json` 里的 client id。用法：`node 测试代码\check-credentials.mjs`。

## 第三方皮肤站（仅皮肤预览）

第三方账号**只用于皮肤预览**，不参与登录与启动（不做 authlib-injector 注入、不改 JVM 参数）：

- `skin_lookup_thirdparty({server, name})`：先按名字查
  `{server}/minecraft/profile/lookup/name/{name}`，失败再把入参当 UUID 查
  `{server}/sessionserver/session/minecraft/profile/{uuid}`；textures 属性可能是 base64 JSON，会解码，
  返回绝对化的皮肤/披风 URL 与 `default|slim` 模型。
- `skin_import_from_url({url, name})`：把皮肤下载进 `.mc\skins\`（校验 64×64 / 64×32，重名追加序号），
  前端「应用」时再调 `skin_apply`。
- 错误全部中文可读：`该玩家不存在` / `该玩家没有设置皮肤` / `皮肤站无响应` / 非法地址。

## 安装与更新

- **只发布安装包**：Release 里唯一资产是 `GuguLauncher-Setup.exe`（NSIS）。不做 portable / 单文件自替换。
- 安装包**未做代码签名**，首次运行 Windows 会提示「未知发布者」，选择「仍要运行」即可。
- **检测更新（1.0.0 已启用）**：启动后延迟 10 秒静默查一次，之后每 6 小时一次；全程不弹窗、不发系统通知，
  结果只显示在「关于」页的一行状态文字上。仓库还没有 Release 或网络失败时显示「已是最新（1.0.0）」，不报错。
- 更新方式（设置 → 常规）：`auto`（默认，发现新版本静默下载安装）/ `manual`（关于页提示，点「立即更新」才装）/ `off`（完全不请求 GitHub）。
  1.0.0 里实际落地的是「检查 + 状态显示 + manual 触发」；auto 的静默下载安装留到 1.0.1 之前的里程碑（代码里有 TODO 标注还差哪一步）。
- 版本识别：优先 `tag_name`，其次 Release 标题（纯数字也行，如 `101`），都解析不出来时用
  `published_at` 与安装包构建时间比较兜底；比较用数值元组补 0（`1.10.0 > 1.9.0`），绝不用字符串比较。
- 资产：只认 `GuguLauncher-Setup.exe`（大小写不敏感，带版本号也容错）；缺这个资产就视为该版本不可用并跳过。
- 节点：请求 Releases 前先按「能代理的节点优先」探测，直连 api.github.com 仍为首选；下载安装包前会**强制重测一轮**选最快节点。

## 发版与目录约定

| 目录 | 放什么 |
| --- | --- |
| `源代码\` | 导出给 GitHub 的源码快照（不含 node_modules / target / .mc） |
| `上传GitHub版本\` | 上传用的发布包，**文件名不带版本号**：`GuguLauncher-Setup.exe` |
| `备份安装包\<版本号>\` | 安装包归档，**带版本号**：如 `备份安装包\1.0.0\GuguLauncher-1.0.0-Setup.exe` |
| `所有版本备份\<版本号>\` | 只放**源码快照**（不放安装包）；当前留空，发布时导出 |
| `测试代码\` | 自检脚本与测试说明 |

打包只产出一种安装包，但落两份（内容完全一致）：
1. `上传GitHub版本\GuguLauncher-Setup.exe`（**不带版本号**，供 GitHub Releases）；
2. `备份安装包\<版本号>\GuguLauncher-<版本号>-Setup.exe`（**带版本号**，作为该版本的归档）。

### 打包（NSIS）

```powershell
pnpm tauri build        # 产物：src-tauri\target\release\bundle\nsis\咕咕启动器_1.0.0_x64-setup.exe
# 发布时复制并重命名为不带版本号的：上传GitHub版本\GuguLauncher-Setup.exe
```

- 打包需要 NSIS 3.13 工具链。tauri CLI 会把它放在 `%LOCALAPPDATA%\tauri\NSIS`，并校验这 12 个文件：
  `Bin/makensis.exe`、两个 `Stubs/lzma*`、`Plugins/x86-unicode/additional/nsis_tauri_utils.dll`（Tauri 专用插件）、
  以及 `Include/{MUI2,FileFunc,x64,nsDialogs,WinMessages}.nsh` 与 `Include/Win/{COM,Propkey,RestartManager}.nsh`。
- 本机实测：GitHub Releases 下载 NSIS 会 504，改为「手动放好 NSIS 3.13 + 通过加速节点取插件 DLL」即可离线打包；
  插件 DLL 来源 `https://github.com/tauri-apps/nsis-tauri-utils/releases/download/nsis_tauri_utils-v0.5.3/nsis_tauri_utils.dll`，sha1 `75197FEE3C6A814FE035788D1C34EAD39349B860`。
- 装不上 NSIS 时 `cargo build --release --features custom-protocol` 仍可产出能跑的 `GuguLauncher.exe`（只是没有安装包）。

### 图标

- 正式 logo 来源：`图片素材\logo\LOGO.png`（512×512）。不要手改 `src-tauri\icons\` 里的 png，全部由它生成。
- 重新生成（改了 logo 就跑这一条）：
  ```powershell
  pnpm tauri icon "D:\minecraft\图片素材\logo\LOGO.png"
  ```
- 产物（`src-tauri\icons\`）：`32x32.png` / `64x64.png` / `128x128.png` / `128x128@2x.png(256)` / `icon.png(512)` /
  `icon.ico`（16/24/32/48/64/256 六种尺寸）以及 Windows 磁贴用的 `Square*Logo.png` 与 `StoreLogo.png`。
- tauri icon 默认还会生成 `android/`、`ios/` 与 `icon.icns`；本工程只做 Windows 桌面，已删掉这些，需要时重新生成即可。
- `tauri.conf.json` 引用：`bundle.icon` 用上面这套；`bundle.windows.nsis.installerIcon / uninstallerIcon` 都用 `icons/icon.ico`，
  所以安装包 exe 与卸载程序 exe 都带 logo 图标（用 `[System.Drawing.Icon]::ExtractAssociatedIcon()` 验证过，非空）。
- **NSIS 向导里的品牌图**不是 installerIcon，而是另外两张 BMP（放在 `src-tauri/nsis/`，同样由 logo 生成）：
  - `headerImage.bmp`（150×57，向导内页左上的页眉）→ `bundle.windows.nsis.headerImage`
  - `sidebarImage.bmp`（164×314，欢迎页/结束页左侧的竖幅）→ `bundle.windows.nsis.sidebarImage`
  - 注意字段名是 `headerImage`，**没有** `installerHeaderImage` 这个键（写错会被配置校验拒掉）。
  重生成这两张：用 PIL 把 LOGO.png 贴到主色底上，尺寸严格按上面两个规格，存成 24 位 BMP。
- 向导语言固定简体中文：`bundle.windows.nsis.languages = ["SimpChinese"]`。
- 安装界面的截图归档在 `测试代码\界面截图\安装包\`（setup 文件图标 / 欢迎页 / 页眉页 / 安装后 exe 图标），
  该目录只做归档，导出源码快照时排除。

### 卸载行为

- **卸载 = 整个安装目录连同 `.mc` 用户数据一起删干净**，不做「是否保留用户数据」的询问。
  实现方式是 `src-tauri/installer-hooks.nsh`（由 `bundle.windows.nsis.installerHooks` 引入）：
  - `NSIS_HOOK_PREUNINSTALL`：先 `taskkill /F /IM GuguLauncher.exe`（进程不在时返回非 0，忽略），
    再 `RMDir /r "$INSTDIR\.mc"` —— 必须先杀进程，否则 db/log 被占用删不掉；
  - `NSIS_HOOK_POSTUNINSTALL`：NSIS 删完自己的文件后 `RMDir /r "$INSTDIR"` 兜底清空目录。
    此时卸载器已把自身复制到 %TEMP% 运行，删 `$INSTDIR` 不会卡住卸载流程（真机验证通过）。
- 为什么不写 `deleteAppDataOnUninstall: true`：Tauri 2.12 的 NSIS 配置 schema 里**没有**这个字段
  （只有 template / installMode / languages / installerHooks / compression 等），写了会直接配置校验失败。
  Tauri 模板本身只清理 `$APPDATA\<BUNDLEID>` 与 `$LOCALAPPDATA\<BUNDLEID>`，不管安装目录，所以必须靠钩子。
- 真机验证：`%TEMP%\gugu-uninstall-test` 静默装 → 跑 5 秒生成 `.mc` → `uninstall.exe /S` 之后
  `Test-Path` 为 `False`（目录整个消失），注册表卸载项 0、开始菜单/桌面快捷方式 0。

## TODO（等美术 / 后续里程碑）

- [ ] **导航图标语义**：`src/assets/icons/` 目前是「UI素材里语义最近」的临时映射
      （主页=固定到首页、实例=切换版本、资源=日志符号、账号=启用、皮肤=实例设置按钮图标、
      联机=分享实例包、Java=更新、存储=游戏目录和存档目录、关于=诊断报告、设置=设置），需要正式图标。
- [ ] **应用图标**：`src-tauri/icons/` 是脚本生成的占位小鸡头，等正式美术素材替换。
- [ ] **exe 版本资源**：`ProductName/FileVersion/ProductVersion` 已经是 1.0.0（NSIS 打包时会写入），
      但 `CompanyName` 目前是 `gugu`（由 identifier 推导），想改成 JIJIGUOWANG123 需要在
      `tauri.conf.json` 的 `bundle.windows` 里补 publisher 相关配置，等发版前统一处理。
- [ ] 新闻卡封面（主页 hero）现在是渐变占位，接 Minecraft 新闻接口后替换。
- [ ] 皮肤预览接线（`src/renderer/skinview/` 已并入源码，未参与构建，需装 three/skinview-utils）。
- [ ] **联机页（陶瓦 Terracotta，M4）**：只保留陶瓦一个入口。计划做法是「从官方源获取陶瓦客户端 →
      静默启动它 → 用联机码加入」，需要确定：陶瓦客户端版本与官方下载源、是否随包内置（体积约几十 MB）、
      启动参数与联机码的传递方式、游戏退出后是否保留隧道。
- [ ] 启动内核（`launch`/`java`）实现。

## 第三方组件与许可

| 组件 | 许可 | 说明 |
| --- | --- | --- |
| skinview3d 3.4.2 | MIT，Copyright (c) 2014-2018 Kent Rasmussen | 皮肤预览库，源码并入 `src\renderer\skinview\`（保留原 MIT 头注释与 LICENSE） |
| three.js | MIT | skinview3d 的 peer 依赖，接入皮肤页时再安装 |

完整信息见 [THIRD-PARTY-NOTICES.md](./THIRD-PARTY-NOTICES.md)。

## 其它文档

- `PLAN-GUGU-1.0.md` —— 里程碑计划（M1…M5）
- `测试代码\README.md` —— 自检脚本说明
