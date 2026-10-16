# Screenguard 桌面端交付状态与发布流程

状态快照：2026-10-17，源码版本 0.4.42。本文区分代码存在、自动化构建、安装、真实设备验收和公开发布；其中一项通过不能代替其他项。

## 发布目标

| 平台 | 当前自动化安装包 | 已验证范围 | 未完成的发布证据 |
|---|---|---|---|
| Windows x64 | NSIS 安装包 | 0.4.41 CI run `38926691248` 的 Windows 检查、Rust 测试和 NSIS 构建成功；安装包 artifact 已下载回读并识别为 NSIS | Windows 实机完整回归、安装升级/卸载验收、安装包签名状态确认 |
| macOS Apple Silicon | DMG | 0.4.42 本机 22 项 Rust 单测、Release/DMG 构建、安装版本与运行进程均已核验；DMG 校验有效；0.4.41 CI run `38926691248` 的三平台检查和 Mac/Windows 打包成功 | 0.4.42 CI；双屏电源联动和窗口跨屏实测；Developer ID 签名、公证和另一台干净 Mac 安装验证 |
| Linux | 暂不发布 | CI 编译和单测 | 没有面向用户的打包、安装与验收流程 |

本项目当前 Release 目标为 Windows x64 和 Apple Silicon Mac；不将 CI 产物称作通过设备验收的正式发行版。

## 功能验收矩阵

| 功能 | 源码状态 | 自动化证据 | 正式发布前还需的真实验收 |
|---|---|---|---|
| 显示器枚举、模式与排列 | Windows/macOS 已实现 | 双平台编译和 Rust 单测 | 不同 GPU、扩展坞和混合 DPI 屏幕核验 |
| 亮度/音量/色彩控制 | Windows/macOS 按 DDC、系统接口或 ICC 路径实现 | 编译/语法检查 | 按具体显示器确认写入与回读；不支持时 UI 必须明确报错 |
| 系统音量 | Windows 可枚举并选择 CoreAudio 端点；macOS 读取/设置系统默认输出音量与静音，不提供应用内输出设备切换 | macOS 解析单测覆盖有效、缺失、越界数据；当前 Mac 实际接口返回 `missing value`，UI 停用控制且不发送设置 | 在提供系统输出音量的 Mac 上确认音量/静音生效及系统托盘同步；确认显示器音频等固定音量输出被如实禁用 |
| 电源联动 | 默认关闭的用户开关、持久化状态和原生后台监听；macOS 结合 VCP `0xD6` 与 CoreGraphics 单屏睡眠状态检测，失败时使用系统级待机/唤醒兜底；面板可逐屏查看 Quartz 状态 | macOS DDC/Quartz 转换策略及诊断序列化单测；三平台 CI | 真实关闭主屏、确认副屏进入待机；再反向唤醒。物理断电但 macOS 仍报告显示器在线且唤醒时，系统没有可检测信号 |
| 窗口列表、双屏铺满与还原 | Windows/macOS 已实现；macOS 需辅助功能授权；macOS 屏幕联合包围盒有横竖屏、负坐标及禁用屏测试 | macOS 窗口边界 3 项测试；跨平台编译 | 分别选择普通应用和视频窗口，跨不同分辨率/方向屏幕铺满，再确认原窗口位置和大小恢复 |
| BetterDisplay 控件 | macOS 命令集接入，部分能力取决于 BetterDisplay 版本、Pro 和显示器 | macOS 编译 | 对每条菜单命令核对执行结果与实际屏幕状态；当前 DDC 读取曾超时，不能宣称电源控制已实测通过 |
| 电池、ADB、自启 | 按平台接入；ADB 下载需用户接受 SDK 条款 | CI 编译 | 有设备时验证电量/网络 ADB；检查登录启动、升级后设置保留 |
| 备份与一键还原 | 已有启动基线与恢复入口 | Rust/平台编译 | 对显示模式、窗口布局、色彩、电源逐项验证还原；测试中断与部分失败路径 |

## 当前发布阻塞项

1. 用户提到的新增 Windows 代码还没有出现在远端仓库分支或提交中，无法基于真实差异完成 Mac 移植。
2. Windows 和 macOS 物理显示器行为仍需设备验收。macOS 屏幕即使物理关闭仍留在系统在线列表、且 DDC 不可读时，系统无法判断该屏已关；未知状态必须保留为未知。
3. 当前签名身份列表没有 Developer ID Application。现有 release workflow 不执行签名或公证，只会创建带明确警示的 GitHub Draft Release；不能把未签名/未公证 DMG 当正式 macOS 发布。
4. 当前 GitHub 最新公开 Release 仍是 v0.2.9；Windows x64 旧安装包可以回滚，但它不包含 0.4.38 的新代码。

## v0.4.38 当前交付核验（2026-10-16）

- 本轮发现并修复 macOS 跨屏窗口还原的状态耦合：此前还原窗口会无条件尝试恢复磁盘上的显示布局备份，即使这次铺满没有改屏幕模式。新版窗口备份记录本次是否应用了屏幕对齐；仅在确有对齐时恢复布局。无该字段的旧备份继续兼容旧行为。
- 本机 Rust 单测 15/15 通过，包含是否恢复模式的决策及旧版 JSON 备份兼容覆盖；仓库自检、JS 语法、`git diff --check` 通过。
- 本机 Release/DMG 构建已成功；`/tmp/screenguard-v0438-target/release/bundle/dmg/Screenguard_0.4.38_aarch64.dmg` 的 SHA-256 为 `9c930593026b1ed798456c10a5cbedf8ccb5da6682cd688f3b3ef9a7990b46ad`，`hdiutil verify` 为 `VALID`；副本 `/Users/nanyu/Desktop/Screenguard_0.4.38_aarch64.dmg` 的 sidecar 校验通过。
- `/Applications/Screenguard.app` 已安装并运行 0.4.38；进程路径为 `/Applications/Screenguard.app/Contents/MacOS/screenguard`，可执行文件 SHA-256 `715a2e0951f2a4cee4e86ab65cce0141ff829c6f1f61cc5a0c45e0c24684469d`，与构建产物相同。安装前 0.4.37 App 备份在 `~/Library/Application Support/Screenguard/backups/pre-0.4.38-app-install-20261016/`。
- 代码提交 CI run `38919104377` 全部成功：仓库自检、macOS/Windows/Ubuntu Rust 检查，以及 macOS DMG 和 Windows NSIS 构建。
- CI 产物回读：macOS artifact `12053109203`（3,983,512 bytes），`hdiutil verify` 为 `VALID`，SHA-256 `309c733c2681b7b3df09de568fbb4f395c9eb3badc6a46eaefe7d83da07ee941`；Windows artifact `12053254261`（2,299,316 bytes），识别为 NSIS 自解压 GUI 安装包，SHA-256 `11b8c5c1345958ada3bd99b03c166aa0bf9c817a86ce841342d702e5cae107a5`。两者已复制到桌面并通过 SHA-256 sidecar 校验；Windows 安装尚未实机验收。
- PR 保持 open/draft；电源联动、窗口铺满/还原仍需实体双屏及辅助功能权限验收。

## v0.4.39 当前交付核验（2026-10-16）

- macOS 新增系统默认输出音量/静音读取与设置；不伪装设备列表或输出路由切换。`osascript get volume settings` 的 `missing value` 被视为不可调，UI 禁用音量/静音，并且设置命令在确认当前端点可调前不会执行。
- 本地 `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib` 18/18 通过；`cargo check --locked`、仓库自检、前端脚本语法检查、版本一致性和 `git diff --check` 通过。
- Apple Silicon Release/DMG 构建成功；DMG SHA-256 `4daf97f4ce83bc783d81934f66e221e9abfdafd4b40e3e77e999e86b40542c4a`，`hdiutil verify` 为 `VALID`。安装前 0.4.38 App 已完整备份至 `~/Library/Application Support/Screenguard/backups/pre-0.4.39-app-install-20261016/`；当前 `/Applications/Screenguard.app` 为 0.4.39，运行二进制哈希 `489b79c0882db5af29284c06efdc123933760c450666813598fcc81a82c60c49` 与构建产物一致。
- 当前 Mac 返回 `output volume:missing value`、`output muted:missing value`，因此仅证明安全停用路径；实际音量变化和硬件/交互验收仍未完成。
- 代码提交 `570bb50d5c111694a99c3da5cc879bfceffb051d` 的 CI run `38922598089` 全部成功。Mac artifact `12054578968`（3,985,907 bytes）DMG SHA-256 `86bd15787b81a3589802612c9b175b4b285ddd487c71cc5c6a4a8e587b6b47ae`，`hdiutil verify` 为 `VALID`；Windows artifact `12054549822`（2,297,257 bytes）NSIS 安装包 SHA-256 `4569e6c160486318ea2ecc77c75f6fbea850bedf84f67a33f9d8e1d74a263050`。两包均复制到桌面并通过 SHA-256 sidecar 校验；Windows 包尚未实机安装。

## v0.4.41 当前交付核验（2026-10-16）

- 修正 macOS 电源联动开启后的面板提示：DDC 不可读时明确说明后台仍监测 Quartz 单屏睡眠及显示器断开/重连，并准确说明物理断电但系统仍报告唤醒时的限制。
- 基于 v0.4.40 已验证代码，版本升至 0.4.41；本机 Release/DMG 构建成功，DMG SHA-256 `4406e55cb78e1e6b3486007565c34c8f6ec97e795c328ec3c23638084afaec18` 且 `hdiutil verify` 为 `VALID`。安装前 0.4.40 App 已备份至 `~/Library/Application Support/Screenguard/backups/pre-0.4.41-app-install-20261016/`；当前 `/Applications/Screenguard.app` 为 0.4.41，运行二进制 SHA-256 `2b002ab54ab9bc6f2f466279ebb83fbf9bdae02b7597adb16d7890a7969cc07b` 与构建产物一致。
- PR 代码提交 `b32c9a7dfc9c1e41f84c4623267cf0caee0ddc90` 的 CI run `38926691248` 全部成功，覆盖 macOS/Windows/Ubuntu 检查和测试、macOS DMG、Windows NSIS 构建。Mac artifact `12057451798`（3,987,631 bytes）DMG SHA-256 `21ca0a5ae53f57f5c2b3212fce219f1edd53a95e8b72bf7ba6dda676108bc579`，`hdiutil verify` 为 `VALID`；Windows artifact `12057806131`（2,297,228 bytes）NSIS SHA-256 `1533eac8536429a8e80802c4245a61cb80d82234babd39cccae5f176b4c4caea`。CI 包均复制至桌面并通过 SHA-256 sidecar 校验；Windows 包尚未实机安装。

## v0.4.42 当前交付核验（2026-10-17）

- 为 macOS 电源联动增加“读取 macOS 屏幕状态”诊断入口，逐屏显示 Quartz 报告的睡眠/唤醒状态，并提示该值不等同于物理断电；修正电源联动 tooltip，说明其会综合 DDC、Quartz 睡眠状态和显示器连接事件。
- 诊断入口用于查看 macOS 是否观测到用户操作对应的睡眠转换。若外接屏物理断电后仍显示在线且系统报告唤醒，界面会如实显示该状态，不把物理电源猜成关闭。
- 本机 Rust 单测 22/22、`cargo check`、仓库自检和前端语法检查通过；Apple Silicon Release/DMG 构建成功。DMG SHA-256 `52b1800f07ca8545c66385401be8c4e1261c8cb48eb7dc751d55f7997a9525bf`，`hdiutil verify` 为 `VALID`，桌面副本的 sidecar 校验通过。
- 安装前 0.4.41 App 已备份至 `~/Library/Application Support/Screenguard/backups/pre-0.4.42-app-install-20261017/`；当前 `/Applications/Screenguard.app` 为 0.4.42，运行二进制 SHA-256 `040d700442fd12b01f73e2484ecbb03ccc0c10d9d9f17fe2c406c5741b2be84e` 与构建产物一致。
- 修改前工作分支备份为 `backup/pre-v0442-power-sleep-diagnostic-20261017`；0.4.42 三平台 CI 和 Windows/macOS CI 安装包待验证。真实双屏行为仍待用户在面板开启电源联动后实测。

## v0.4.40 交付核验（2026-10-16）

- macOS 原生后台联动新增 CoreGraphics `CGDisplayIsAsleep` 单屏睡眠传感器，在 DDC VCP `0xD6` 无法读取时作为独立来源；单屏状态变化可触发同步，多屏并发或与 DDC 冲突时不猜测来源。DDC 目标控制失败仍走 macOS 系统级待机/唤醒兜底。
- 本地 Rust 单测 21/21、`cargo check`、仓库自检、前端脚本语法检查和版本一致性通过；Apple Silicon Release/DMG 构建成功。DMG SHA-256 `e6b241b1cbec338f0175de385b6e1c9ddaeb651bf1d9475f7d9a8b55bd49a9f5`，`hdiutil verify` 为 `VALID`；桌面副本及 sidecar 校验通过。
- 已将原 0.4.39 安装备份至 `~/Library/Application Support/Screenguard/backups/pre-0.4.40-app-install-20261016/`；当前 `/Applications/Screenguard.app` 为 0.4.40，运行二进制 SHA-256 `ec21022c1208e68afd4bd5fe92c15c4e1fb6868fe3dfb9230280c4122e290993` 与构建产物一致。
- PR 代码提交 `08f7b37c8de02b71707896188e1bc8539a697379` 的 CI run `38924784040` 全部成功，包含 macOS/Windows/Ubuntu 检查与测试、macOS DMG 和 Windows NSIS 构建。Mac artifact `12056068292`（3,987,300 bytes）DMG SHA-256 `10cd0b24bd7266480e77d0d4f1482f721c461bd41ea428b043102c644103310c`，`hdiutil verify` 为 `VALID`；Windows artifact `12055699379`（2,299,497 bytes）NSIS 安装包 SHA-256 `c2ea11991d15c46e802cb9b8c71b852a293a401acb7a1725244af197d135acb1`。两份 CI 包均复制至桌面并通过 sidecar SHA-256 校验；Windows 包尚未在 Windows 实机安装。
- 该 API 反映 macOS 认定的显示器睡眠状态；若外接屏物理断电后仍在线且系统仍报告唤醒，仍无法得知硬件电源变化。真实关屏/唤醒跟随仍需双屏实测。

## v0.4.37 历史交付核验（2026-10-16）

- 源码候选：PR #1 `codex/screenguard-v0.4.31-mac-adaptation`，修复 commit `b59e4741765e1401016afe504a286e6152969c54`；PR 保持 open/draft，未合入 `main`。
- 修复内容：macOS VCP `0xD6` 状态解析此前遗漏值 `3`，导致 DPMS Suspend 没有进入关屏状态转换；现解析器识别 1–5，新增五种模式及无效输出测试。版本号已统一升至 0.4.37。
- 本地源码核验：`cargo test --locked --manifest-path src-tauri/Cargo.toml --lib` 13/13 通过；`scripts/check_repo.py`、`node --check ui/main.js`、版本一致性检查及 `git diff --check` 通过。
- CI：代码 commit 对应 run `38914770919` 的仓库检查、macOS/Windows/Ubuntu Rust 检查与测试、Windows C# 桥接、macOS DMG 和 Windows NSIS 构建全部成功；Release workflow 配置检查 run `38914771070` 成功。
- CI 安装包：macOS artifact `12052530998`，3,981,400 bytes，SHA-256 `15d9494be04b3c00ba6859543319f3e07a4d63d0ccd62ca431dd6c3dcf8f07a5`，`hdiutil verify` 为 `VALID`。Windows artifact `12052417052`，2,298,660 bytes，SHA-256 `7ed2846bec69d54bcba303915bf458770a9ebd2e40c351e0e64ac309dd79d56d`，识别为 NSIS 自解压 GUI 安装包；尚未在 Windows 实机安装运行。两份 CI 安装包已复制到桌面并通过 SHA-256 sidecar 校验。
- 本机安装：`/Applications/Screenguard.app` 版本为 0.4.37，运行进程路径为该 App 内可执行文件，二进制 SHA-256 `07af4fb517ca902da2dfc67d148b30d9b589d3a3b584dd889e116d9f410fea69`。安装前版本备份在 `~/Library/Application Support/Screenguard/backups/pre-0.4.37-app-install-20261016/`。
- 本机 DMG：独立 Release 构建产物 `/tmp/screenguard-v0437-target/aarch64-apple-darwin/release/bundle/dmg/Screenguard_0.4.37_aarch64.dmg` 已通过 `hdiutil verify`；桌面另有 CI 版和本地版 DMG 候选包。
- 硬件边界：电源联动开关仍为关闭。只读 BetterDisplay VCP `0xD6` 查询中，显示器 ID 3 返回 `Failed.`，ID 2 在 8 秒后超时；本轮没有发送 DDC 电源写入命令。真实双屏关/唤醒联动仍需用户在开关开启后验收。窗口跨屏铺满也尚无真实 UI 验收；Computer Use runtime 当前缺失。
- 发布边界：PR 未合并，公开 Release 仍为 v0.2.9；没有 Developer ID Application 证书，因此尚未签名/公证或正式发布。

## v0.4.36 历史交付核验（2026-10-16）

- 源码候选：PR #1 `codex/screenguard-v0.4.31-mac-adaptation`，电源修复 commit `963391b98b298531e4c9d6753c44d8bc64bdfa6b`；证据文档 commit `749f319581e83d5894e03e39f03989f0194e2372`；PR 保持 open/draft，未合入 `main`。
- CI：测试提交 `73958021ea2e6917bbc10c6d10cd615dd15e08cf` 的 run `38899909127` 成功；Node 24 Actions 更新提交 `1aea5b2250755f0e0a2f5a8ab776b33b69a636eb` 的 run `38900099781` 六项仓库检查、Mac/Windows/Ubuntu Rust 检查/测试以及双平台安装包构建全部成功；Draft desktop release 配置检查 run `38900099445` 成功（PR 触发时不执行发行包和 Draft Release job）。
- CI 产物回读：macOS artifact `12044228631`（3,980,567 bytes）下载后 `hdiutil verify` 为 `VALID`，SHA-256 `e43928571b772339dab541ba4a107d170af816456bd9e1c888c00eb66a4fc9f5`。Windows artifact `12045165124`（2,298,498 bytes）为 `Screenguard_0.4.36_x64-setup.exe`，识别为 NSIS 自解压安装包，SHA-256 `3fabac4317798cb7aae148eed7f0d2759c9451ce4fd5f782fd14b3ac0139a2bb`；已复制到用户桌面供 Windows 机器试装，但尚未在 Windows 实机安装/运行。
- 当前 PR head 的 Windows installer 与 Mac DMG artifact 构建步骤也已通过；上述下载校验值对应 run `38896724184` 的独立 artifact，不能混同为 run `38897994905` 的文件哈希。
- 最新 run `38900099781` artifacts 已下载回读：Mac DMG `12046210139`（3,980,571 bytes），`hdiutil verify` 为 `VALID`，SHA-256 `340a43d20c48fe4e1ba965e9f342d5991248e6ae57c03e961f53a516f0837e85`；Windows installer `12044959989`（2,298,827 bytes），识别为 NSIS 自解压 PE32 安装包，SHA-256 `4623ad822fddbf2911e9fde48d409b2a7b1f484a0f097065829a566e2c4bf277`，并已复制至桌面供 Windows 实机试装。
- 本机安装：`/Applications/Screenguard.app` 的 `CFBundleShortVersionString` 为 `0.4.36`；可执行文件 SHA-256 为 `14125b10f7b967e0cf6b2306b10b7b0d4f09ad566e1443ddce2e822a15aa4294`，与本机构建产物一致。原 0.4.35 App 备份位于 `~/Library/Application Support/Screenguard/backups/pre-0.4.36-app-install-20261016/`。
- 本机安装包：`Screenguard_0.4.36_aarch64.dmg` 的 SHA-256 为 `0650de92e73493b9d6e57debd1462bb445dffcf4e9c6dec846af1c9bfa5b4085`；`hdiutil verify` 返回 `VALID`。
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
