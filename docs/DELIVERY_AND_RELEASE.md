# Screenguard 桌面端交付状态与发布流程

状态快照：2026-10-16，源码版本 0.4.35。本文区分代码存在、自动化构建、安装、真实设备验收和公开发布；其中一项通过不能代替其他项。

## 发布目标

| 平台 | 当前自动化安装包 | 已验证范围 | 未完成的发布证据 |
|---|---|---|---|
| Windows x64 | NSIS 安装包 | CI run `38894176613` 中 Windows 检查、Rust 单测、C# 桥接检查和 NSIS 构建全部成功 | Windows 实机完整回归、安装升级/卸载验收、安装包签名状态确认 |
| macOS Apple Silicon | DMG | CI run `38894176613` 中 macOS 检查、Rust 单测、DMG 构建成功；本机 0.4.35 App 已安装并核验版本/二进制哈希，DMG 校验有效 | 双屏电源联动和窗口跨屏实测；Developer ID 签名、公证和另一台干净 Mac 安装验证 |
| Linux | 暂不发布 | CI 编译和单测 | 没有面向用户的打包、安装与验收流程 |

本项目当前 Release 目标为 Windows x64 和 Apple Silicon Mac；不将 CI 产物称作通过设备验收的正式发行版。

## 功能验收矩阵

| 功能 | 源码状态 | 自动化证据 | 正式发布前还需的真实验收 |
|---|---|---|---|
| 显示器枚举、模式与排列 | Windows/macOS 已实现 | 双平台编译和 Rust 单测 | 不同 GPU、扩展坞和混合 DPI 屏幕核验 |
| 亮度/音量/色彩控制 | Windows/macOS 按 DDC、系统接口或 ICC 路径实现 | 编译/语法检查 | 按具体显示器确认写入与回读；不支持时 UI 必须明确报错 |
| 电源联动 | 默认关闭的用户开关、持久化状态和原生后台监听；macOS 对 DDC suspend 状态 2/3/4/5 有单测 | macOS 7 个 Rust 单测；三平台 CI | 真实关闭主屏、确认副屏进入待机；再反向唤醒；覆盖 DDC 不可读、应用退出和权限场景 |
| 窗口列表、双屏铺满与还原 | Windows/macOS 已实现；macOS 需辅助功能授权 | 跨平台编译 | 分别选择普通应用和视频窗口，跨不同分辨率/方向屏幕铺满，再确认原窗口位置和大小恢复 |
| BetterDisplay 控件 | macOS 命令集接入，部分能力取决于 BetterDisplay 版本、Pro 和显示器 | macOS 编译 | 对每条菜单命令核对执行结果与实际屏幕状态；当前 DDC 读取曾超时，不能宣称电源控制已实测通过 |
| 电池、ADB、自启 | 按平台接入；ADB 下载需用户接受 SDK 条款 | CI 编译 | 有设备时验证电量/网络 ADB；检查登录启动、升级后设置保留 |
| 备份与一键还原 | 已有启动基线与恢复入口 | Rust/平台编译 | 对显示模式、窗口布局、色彩、电源逐项验证还原；测试中断与部分失败路径 |

## 当前发布阻塞项

1. 用户提到的新增 Windows 代码还没有出现在远端仓库分支或提交中，无法基于真实差异完成 Mac 移植。
2. Windows 和 macOS 物理显示器行为仍需设备验收。macOS 屏幕即使物理关闭仍留在系统在线列表、且 DDC 不可读时，系统无法判断该屏已关；未知状态必须保留为未知。
3. 当前签名身份列表没有 Developer ID Application。现有 release workflow 不执行签名或公证，只会创建带明确警示的 GitHub Draft Release；不能把未签名/未公证 DMG 当正式 macOS 发布。
4. 当前 GitHub 最新公开 Release 仍是 v0.2.9；Windows x64 旧安装包可以回滚，但它不包含 0.4.35 的新代码。

## 最新交付核验（2026-10-16）

- 源码候选：PR #1 `codex/screenguard-v0.4.31-mac-adaptation`，commit `a89a6e8b84d4b8bde95592dfac513c40c12f842f`；PR 保持 open/draft，未合入 `main`。
- CI：run `38894176613` 的仓库检查、macOS/Windows/Ubuntu Rust 检查与测试、macOS DMG 和 Windows NSIS 构建全部成功。
- 本机安装：`/Applications/Screenguard.app` 的 `CFBundleShortVersionString` 为 `0.4.35`；可执行文件 SHA-256 为 `0b872ea219f88a8e59de67c6112ffb60550d8aa1b7e9ed4f99f5da98c2f65d81`，与本机构建产物一致。
- 本机安装包：`Screenguard_0.4.35_aarch64.dmg` 的 SHA-256 为 `f009a01e7c34346bdb8c64f4362aecf98fad3830f08585d081175d23eff1f251`；`hdiutil verify` 返回 `VALID`。
- 本次范围没有更改用户原始主工作区的既有改动；所有尚未完成的实机、签名、公证和公开发布门槛继续保持未完成状态。

## 自动化发布流程

`.github/workflows/release.yml` 只处理 `v*` tag：

1. tag 对应的版本必须与 `package.json`、`src-tauri/tauri.conf.json` 和 `Cargo.toml` 一致。
2. macOS 运行仓库检查和 Rust 单测，构建 Apple Silicon DMG；Windows 运行仓库检查、C# 桥接检查和 Rust 单测，构建 x64 NSIS 包。
3. 两个安装包均生成 SHA-256 文件。
4. 仅当 tag 的提交已合入 `main` 且双平台构建成功时，自动创建或更新 GitHub Draft Release。
5. Draft 不会自动公开。只有实机验收、签名/公证和升级/回滚检查均有证据后，维护者才可手动发布。

正式打 tag 前要先完成 PR 审核并合入 `main`，确认仓库版本字段一致，再创建 `vX.Y.Z` tag 并推送。不要在未合入 main 的 PR commit 上创建正式版本 tag；工作流会拒绝这种 tag。

## 变更保护与回滚

- 代码修改先在独立 PR worktree 完成；原始主工作区的未提交修改保持原样。
- 发布工作流新增前保留分支 `backup/pre-release-workflow-20261016`，可用它恢复工作流文件。
- 现有发布资产不覆盖；新版本使用新的 tag、Release 和安装包文件名。
- 发布后回滚通过重新发布上一稳定安装包或修复后发布新补丁版本完成，不移动已公开的历史 tag。
