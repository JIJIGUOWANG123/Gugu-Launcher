# 贡献指南

感谢你愿意改进咕咕启动器。下面是最短的可行路径。

## 环境要求

| 组件 | 版本 / 说明 |
| --- | --- |
| Node.js | 20 或更高（仅用于前端构建） |
| pnpm | 9 或更高（`packageManager` 字段已锁定） |
| Rust | 1.80 或更高（`rust-toolchain` 不锁定，使用 stable 即可） |
| MSVC | Visual Studio Build Tools，勾选「使用 C++ 的桌面开发」（Tauri 与 cubiomes 的 C 编译都需要） |
| WebView2 | Windows 10/11 自带；更早系统需安装运行时 |

## 本地开发

```bash
pnpm install
pnpm dev            # 前端热更新（浏览器预览时使用内置 mock 数据）
pnpm tauri dev      # 带 Rust 后端的完整开发模式
```

## 构建与打包

```bash
pnpm tauri build    # 产出 src-tauri/target/release/bundle/nsis/*.exe
```

> 首次打包需要 NSIS。Tauri CLI 会自动下载；若网络不通，可从 NSIS 官网下载 3.x 后解压到
> `%LOCALAPPDATA%\tauri\NSIS`，并补上 `nsis_tauri_utils.dll`。

## 测试

```bash
cd src-tauri && cargo test          # 后端单元测试（无需网络）
pnpm exec tsc --noEmit              # 前端类型检查
pnpm build                          # 前端构建
```

UI 冒烟测试脚本与截图归档不在本仓库内（属于维护者的本地工具链），提交 PR 时只需保证
`cargo test`、`tsc --noEmit` 与本机构建通过。

## 代码约定

- **Rust**：运行期禁止 `unwrap()` / `panic!`；错误统一走 `error.rs` 的错误类型；所有子进程必须用
  `util::hidden` / `util::hidden_async`，否则会在 Windows 上弹出控制台黑窗。
- **TypeScript**：函数组件 + hooks；所有面向用户的文案走 `src/lib/i18n.ts`（当前仅简体中文）；
  不允许在组件里直接 `invoke`，统一用 `src/lib/tauri-api.ts` 封装（那里同时维护 mock 数据）。
- **样式**：颜色、圆角、阴影一律使用 `src/styles/theme.css` 里的 CSS 变量，禁止硬编码色值。
- **依赖**：不引入新的前端运行时依赖；Rust 侧新增 crate 需在 PR 描述里说明理由。

## 提交与 PR

1. 从 `main` 切出分支，命名 `feat/xxx`、`fix/xxx`、`docs/xxx`。
2. 提交信息用中文或英文均可，首行概括改动，正文写「为什么」与「验证方式」。
3. PR 描述里请附：
   - 改动点列表；
   - 验证证据（命令输出、截图或日志片段）；
   - 是否影响既有数据目录结构（`.mc` 里的文件布局）。
4. 一个 PR 只做一件事；纯格式化或重命名请单独提交。

## 数据目录与凭据

- 启动器数据根默认是安装目录下的 `.mc`，**不要在代码里写死路径**，走 `paths.rs`。
- 微软登录的 Client ID 由用户自己在「设置 → 内容服务」填写，**仓库里不允许出现任何真实 Client ID、
  访问令牌或密钥**；提交前请自查。

## 第三方组件

| 组件 | 许可 | 用途 |
| --- | --- | --- |
| skinview3d / three.js | MIT | 皮肤 3D 预览 |
| cubiomes | MIT | 种子地图的世界生成计算 |
| Terracotta | AGPL-3.0 | 陶瓦联机（以独立进程调用，未修改二进制） |

完整清单见 `THIRD-PARTY-NOTICES.md`。修改涉及 AGPL 组件的分发方式前请先开 issue 讨论。
