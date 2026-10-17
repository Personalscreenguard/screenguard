# Screenguard 桌面端交付状态与发布流程

状态快照：2026-10-17，源码版本 0.4.45。本文区分代码存在、自动化构建、安装、真实设备验收和公开发布；其中一项通过不能代替其他项。

## 发布目标

| 平台 | 当前自动化安装包 | 已验证范围 | 未完成的发布证据 |
|---|---|---|---|
| Windows x64 | NSIS 安装包 | CI run `39033430993` 的 Windows 静默安装/卸载 smoke 和 NSIS 构建均成功；artifact 已下载回读并识别为 NSIS | Windows 实机完整回归、升级保留设置/卸载行为验收、安装包签名状态确认 |
| macOS Apple Silicon | DMG | 本机 0.4.45 DMG 已重建，`hdiutil verify` 为 VALID；单独生成的 `.app` 已 ad-hoc 签名并安装运行。CI run `39033430993` 的上一源码 DMG 也已下载回读。本轮为 tag 发布工作流加入 Developer ID 证书导入、签名身份、公证票据和 Gatekeeper 检查；尚无 Developer ID 签名/公证包 | 双屏电源联动和窗口跨屏实测；配置 Apple 发布 secrets 后运行 tag 发布并验证干净 Mac 安装 |
| Linux | 暂不发布 | CI 编译和单测 | 没有面向用户的打包、安装与验收流程 |

本项目当前 Release 目标为 Windows x64 和 Apple Silicon Mac；不将 CI 产物称作通过设备验收的正式发行版。

## v0.4.45 macOS 跨屏窗口还原路径修复（2026-10-17）

- 修复 `restore_foreground` 的平台分工：macOS 原生还原器已依据窗口备份中的 `alignment_applied` 决定是否恢复显示器模式；上层 Tauri 命令不再在 macOS 额外无条件调用 `align_restore_modes`，避免把另一项操作遗留的布局备份误当成本次窗口铺满产生的变更。Windows 仍由上层独立还原窗口与显示模式。
- 修改前可恢复分支：`backup/pre-macos-window-restore-wrapper-20261017`。版本升至 0.4.45；Mac `cargo check --offline`、指定 MacOSX26.5 SDK 的 `cargo test --locked --lib`（22/22）、仓库自检、`node --check ui/main.js` 与 `git diff --check` 已通过。默认 MacOSX27.0 SDK 的 linker 无法解析其 `.tbd` 架构标签，因此本机检查与构建显式使用 26.5 SDK。
- 本机 Apple Silicon Release App/DMG 重建成功。Tauri App 的 linker 签名起初未通过严格 bundle 校验；本地 ad-hoc 重签后 `codesign --verify --deep --strict` 通过。该签名没有 Team ID，也未公证。桌面 DMG 从这份已验证的 App、Applications 快捷入口和许可证重新组装，`hdiutil verify` 为 VALID，SHA-256 `91b86a5b5cc50321b6d0bb701d33cbc482ea42338036ce7c48ec8635a703104f`，sidecar 回读一致。`/Applications/Screenguard.app` 正运行 0.4.45，安装二进制 SHA-256 `58f073a79b5cbae286cd368db85e3bae3830e36b5775c6f8cb924651fb8ebfcd`。安装前 0.4.44 已备份至 `~/Library/Application Support/Screenguard/backups/pre-0.4.45-app-install-20261017/`；电源联动偏好仍为关闭。
- 完整跨平台 CI 与 Windows 安装包仍待本提交推送后验证；这项源码回滚修复尚无真实多屏窗口 UI 实测。DMG 镜像校验只证明镜像完整性，不代替对其内 App 的签名与运行验收；本地组装包中的 App 已单独验证并与安装副本匹配，但仍不是正式签名公证包。

## 功能验收矩阵

| 功能 | 源码状态 | 自动化证据 | 正式发布前还需的真实验收 |
|---|---|---|---|
| 显示器枚举、模式与排列 | Windows/macOS 已实现 | 双平台编译和 Rust 单测 | 不同 GPU、扩展坞和混合 DPI 屏幕核验 |
| 亮度/音量/色彩控制 | Windows/macOS 按 DDC、系统接口或 ICC 路径实现 | 编译/语法检查 | 按具体显示器确认写入与回读；不支持时 UI 必须明确报错 |
| 系统音量 | Windows 可枚举并选择 CoreAudio 端点；macOS 读取/设置系统默认输出音量与静音，不提供应用内输出设备切换 | macOS 解析单测覆盖有效、缺失、越界数据；当前 Mac 实际接口返回 `missing value`，UI 停用控制且不发送设置 | 在提供系统输出音量的 Mac 上确认音量/静音生效及系统托盘同步；确认显示器音频等固定音量输出被如实禁用 |
| 电源联动 | 默认关闭的用户开关、持久化状态和原生后台监听；macOS 结合 VCP `0xD6` 与 CoreGraphics 单屏睡眠状态检测，失败时使用系统级待机/唤醒兜底；面板可逐屏查看 Quartz 状态。Quartz 当前状态不等于物理电源状态 | macOS DDC/Quartz 转换策略及诊断序列化单测；三平台 CI | 真实关闭主屏、确认副屏进入待机；再反向唤醒。物理断电但 macOS 仍报告显示器在线且唤醒时，系统没有可检测信号 |
| 窗口列表、双屏铺满与还原 | Windows/macOS 已实现；macOS 需辅助功能授权；macOS 屏幕联合包围盒有横竖屏、负坐标及禁用屏测试 | macOS 窗口边界 3 项测试；跨平台编译 | 分别选择普通应用和视频窗口，跨不同分辨率/方向屏幕铺满，再确认原窗口位置和大小恢复 |
| BetterDisplay 控件 | macOS 命令集接入，部分能力取决于 BetterDisplay 版本、Pro 和显示器 | macOS 编译 | 对每条菜单命令核对执行结果与实际屏幕状态；当前 DDC 读取曾超时，不能宣称电源控制已实测通过 |
| 电池、ADB、自启 | 按平台接入；ADB 下载需用户接受 SDK 条款 | CI 编译 | 有设备时验证电量/网络 ADB；检查登录启动、升级后设置保留 |
| 备份与一键还原 | 已有启动基线与恢复入口 | Rust/平台编译 | 对显示模式、窗口布局、色彩、电源逐项验证还原；测试中断与部分失败路径 |

## 当前发布阻塞项

1. 用户提到的新增 Windows 代码还没有出现在远端仓库分支或提交中，无法基于真实差异完成 Mac 移植；本次只适配了 PR 中已有的 Windows 健康面板能力。
2. Windows 和 macOS 物理显示器行为仍需设备验收。macOS 屏幕即使物理关闭仍留在系统在线列表、且 DDC 不可读时，系统无法判断该屏已关；未知状态必须保留为未知。
3. 当前签名身份列表没有 Developer ID Application，GitHub 仓库也未配置 Apple 发布 secrets。tag 工作流现要求证书、签名身份、Apple ID 公证凭据和临时 keychain 密码；缺少任一项就会停止，且签名、公证、Gatekeeper 检查全部成功前不会上传 macOS 包。当前仍没有已签名/公证 DMG。
4. 当前 GitHub 最新公开 Release 仍是 v0.2.9；Windows x64 旧安装包可以回滚，但它不包含 0.4.43 的新代码。

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
- PR 代码 commit `fe9e98ca10def91a5a5316a7796c72eae0974c60` 对应 CI run `38930181257` 全部成功：仓库检查、macOS/Windows/Ubuntu Rust 检查与测试、macOS DMG 和 Windows NSIS 构建。macOS artifact `12057954740`（3,990,367 bytes），DMG SHA-256 `f9dcfcfbd01378327db95ff5107ffc093f1b95258324748f420b0ce4d058a080`，`hdiutil verify` 为 `VALID`；Windows artifact `12058572940`（2,303,948 bytes），识别为 Nullsoft NSIS 安装包，SHA-256 `eb03ee70a2fffbada6aeb30832f92cee5cfe9cda5fb88abe5f672955b3765859`。两包及 SHA-256 sidecar 已复制到桌面并校验通过；Windows 包尚未实机安装。
- 修改前工作分支备份为 `backup/pre-v0442-power-sleep-diagnostic-20261017`。真实双屏电源联动行为仍待用户在面板开启开关并读取 Quartz 状态后实测。

## v0.4.43 macOS 显示器状态面板适配（2026-10-17）

- Windows 的事件编号、显示驱动重置日志和链路重协商操作不适用于 macOS。本版本在 Mac 上将该区域呈现为 Quartz 逐屏睡眠状态读取器，隐藏 Windows 专用修复按钮、自动修复和事件提醒设置，避免显示空的 Windows 事件日志或提供无效操作。
- 状态直接调用既有 `get_display_sleep_states`，每分钟刷新一次；无状态时显示“状态未知”。明确说明 Quartz 只表示系统认定的睡眠/唤醒状态，不代表显示器面板物理断电。Windows 行为保持不变。
- 本机 `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib` 22/22 通过；`scripts/check_repo.py`、`node --check ui/main.js`、内嵌脚本语法和 `git diff --check` 通过。当前 Command Line Tools 默认 MacOSX27.0 SDK 与本机 linker 不兼容；显式使用 `/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk` 后测试及构建成功。
- Apple Silicon Release/DMG 构建成功：`/tmp/screenguard-v0443-target/release/bundle/dmg/Screenguard_0.4.43_aarch64.dmg`，SHA-256 `947169c4d3548f1efef0ccf812426e10e0be4fd47c2a7a9a688ad69e949f53b0`，`hdiutil verify` 为 `VALID`。桌面副本 `/Users/nanyu/Desktop/Screenguard_0.4.43_aarch64.dmg` 和 sidecar 校验通过。
- 修改前 `/Applications/Screenguard.app` 0.4.42 备份至 `~/Library/Application Support/Screenguard/backups/pre-0.4.43-app-install-20261017/`。当前已安装并运行 0.4.43；运行进程 `/Applications/Screenguard.app/Contents/MacOS/screenguard`，二进制 SHA-256 `b55c0d64ae2c124a96c33450e9d3c56809590cc62228cd1d8aa94f4d2a32534b` 与本机 Release 构建一致。
- PR 代码 commit `7d6be10a10f477a2e741b83cb9c409d8197df695` 对应 CI run `38954826601` 全部成功：仓库自检、macOS/Windows/Ubuntu Rust 检查与测试、macOS DMG 和 Windows NSIS 构建。Draft desktop release workflow run `38954826540` 成功。
- CI artifact `12070547413`（ZIP 3,989,253 bytes，GitHub digest `cc1e764a18dc6392f5071c5e40f020ee7f0e9108799084a43f18cf04d3f1bcf2`）解出 DMG 4,020,193 bytes，SHA-256 `adacae454b280b4ba22df5e4594ca2cff4255314fb569285ee7856116b5e7599`，`hdiutil verify` 为 `VALID`；桌面副本 `Screenguard_0.4.43_aarch64-ci-38954826601.dmg` sidecar 校验通过。
- CI artifact `12070522871`（ZIP 2,305,136 bytes，GitHub digest `b19408d6af51b8a300fa03df9ce286d02cd5cf58918ed01f0da8c69dbf0b4168`）解出 Windows 安装包 2,325,340 bytes，SHA-256 `c10b36320806611d48ffdf27438c9069e952a9720f8e33cb3d99f4e566529cd5`，识别为 NSIS 自解压 GUI 安装包；桌面副本 `Screenguard_0.4.43_x64-setup-ci-38954826601.exe` sidecar 校验通过。Windows 安装和运行仍未在 Windows 实机验收。
- 实体双屏电源跟随、跨屏窗口实测、Windows 真机验收、Developer ID 签名和公证仍待完成。

## v0.4.43 Windows NSIS 安装卸载 smoke gate（2026-10-17）

- CI 修复提交 `7eeabefba93f47b726be12814de02e2471b06450` 对应 run `38999872386` 全部成功；仓库自检、macOS/Windows/Ubuntu Rust 检查与测试、macOS DMG 构建均通过。Windows job 的临时目录静默安装、主程序存在检查、同步静默卸载和主程序删除断言全部通过。Draft desktop release workflow run `38999872276` 配置检查成功，tag-only 打包和发布步骤按预期跳过。
- Windows artifact `12089115392`（ZIP 2,302,942 bytes，GitHub digest `ef22ae7201a34a050075981577ebfd5d218ed8a42c7dbcfe0914a80410d35617`）已下载回读；解出 NSIS 安装包 2,323,144 bytes，SHA-256 `f32a7f31c898bcc1d2656746fb08dc7551b2f9518751150b4ad87b8bff2e70b6`。macOS artifact `12088955856`（ZIP 3,989,250 bytes，GitHub digest `2f3af29d56bba584578fd5b29879abe647cc24f17206fb190920904a83a53310`）解出 Apple Silicon DMG 4,020,194 bytes，SHA-256 `cb8aba5bae2565fe5ede74bb79cea403302260885098b1f86e5b188e97003a80`，`hdiutil verify` 为 `VALID`。
- 这项 Windows 安装卸载检查运行在 GitHub Actions runner，不代表用户 Windows 设备的安装、启动、设置迁移或卸载回归已验收；真机验证仍待完成。

## v0.4.43 macOS Developer ID 发布门禁（2026-10-17）

- tag 发布工作流现在先导入并核验 Developer ID Application `.p12` 身份，再让 Tauri 对 Apple Silicon App 签名并执行 Apple 公证；产物挂载后还会检查 App 的深层代码签名、签名者与 Team ID、公证票据和 Gatekeeper 接受状态。
- 发布所需 GitHub Actions secrets：`APPLE_CERTIFICATE`（base64 编码的 `.p12`）、`APPLE_CERTIFICATE_PASSWORD`、`APPLE_SIGNING_IDENTITY`（必须以 `Developer ID Application:` 开头）、`KEYCHAIN_PASSWORD`、`APPLE_ID`、`APPLE_PASSWORD`（Apple app-specific password）和 `APPLE_TEAM_ID`。只应通过仓库 Actions secrets 配置，不要提交到源码或聊天。
- PR 构建仍不读取这些 secrets；正式 tag 构建在 secrets 缺少、证书无法导入、签名不匹配、公证/装订失败或 Gatekeeper 拒绝时会失败，因而不会进入双平台 Draft Release 汇总。
- 本轮只完成工作流源码与文档变更。当前没有 Developer ID Application 证书，也没有配置仓库 secrets；未运行 tag 构建，未生成已签名/公证的候选包，不能据此声称发布验收通过。
- 提交 `e4162aff12478b80e79ce7c2706533ea543fb6db` 的 CI run `39001798036` 全部成功：仓库自检、macOS/Windows/Ubuntu Rust 检查与测试、macOS DMG、Windows NSIS 构建及 Windows 静默安装/卸载 smoke。Draft workflow 检查 run `39001798041` 成功；tag-only 打包与 Draft Release job 按 PR 事件跳过。
- CI artifact：Mac `12089995376`（归档 3,989,255 bytes，GitHub digest `77c4f8f3e566463425123b22e454a1acbbe7a2090f378ad493e65e40ba2fc651`），DMG SHA-256 `54be839d2abf51b3bc6dc2a2fcb05bcb317f4d3dc45530b863e77055f29023cb`，`hdiutil verify` 为 `VALID`；Windows `12089980600`（归档 2,302,751 bytes，GitHub digest `d447daaa3452afdaa1e40199109b3d84eb98f8dafbd5e887afa42110f9eb99e1`），解出 NSIS 安装包 SHA-256 `4a6274e2f5c01137fab07b5cd29e52551abdb341eef30f0e260ea5db244a4938`。下载的 Mac app 显示 `adhoc,linker-signed`；这次 PR artifact 未签名/公证，不可当作正式 Mac 发布包。

## v0.4.44 macOS 电源联动只读诊断（2026-10-17）

- 现有“读取 macOS 屏幕状态”入口扩展为同时读取联动开关、Quartz 逐屏睡眠状态和每屏 DDC VCP 0xD6 电源值；DDC 失败逐屏展示错误，所有请求均为只读，不调用关屏/唤醒命令。
- 当前 Mac 在线设备为 Mi Monitor 主屏（CoreGraphics ID 3，1920x1080@60）与 HDMI TO USB 副屏（ID 2，1080x1920@60，竖屏）。只读 Quartz 查询显示两屏 online 且 awake；当前持久化联动开关为关闭。BetterDisplay VCP 0xD6 读取主屏返回 Failed.，副屏超时。以上是当前传感器状态，不是开启联动后的行为验收。
- 本机 Apple Silicon Release/DMG 构建成功：`src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/Screenguard_0.4.44_aarch64.dmg`，SHA-256 `b984858ff77fffb97493e68d7e8be72416509a8a9ed767250c9f20a3e480c8a5`，`hdiutil verify` 为 `VALID`；App 仍为 ad-hoc 签名，不能作为正式 Mac 发布包。
- 修改前工作区备份分支：`backup/pre-ddc-diagnostic-20261017`。仓库自检、22 项 Rust 单测与 `git diff --check` 已通过。代码 commit `e5c3f7600c27ec3a6c13157330f12ddc86521ffb` 对应 CI run `39033430993` 全部成功：macOS/Windows/Ubuntu Rust 检查与测试、Windows C# bridge、Windows NSIS 静默安装/卸载 smoke、macOS DMG、Windows NSIS 构建及仓库自检；Draft release workflow run `39033431007` 成功。
- CI artifact 回读：Mac artifact `12101616375`（归档 3,991,419 bytes，GitHub digest `254230a479547ff90af027c470480b12c10edc2c87f7c116c7614ab2206ed611`），DMG SHA-256 `f2e3bb47308b5f91ec2d657ed4ff34e29f3f19dc92aaf9852b90d267950eda43`，`hdiutil verify` 为 `VALID`；Windows artifact `12101721184`（归档 2,305,893 bytes，GitHub digest `c62125ddb6f34c917fab547249e4dd5556820f2e8403799647608f8f6db1d657`），解出 `Screenguard_0.4.44_x64-setup.exe`，识别为 Nullsoft NSIS 安装包，SHA-256 `05aaff8687456bc0185a31296b39cf00fa7b251f87466cef81abcb89fa7aac71`。CI Windows smoke 通过不等于 Windows 实机完整验收。
- 两份 CI 安装包和独立 SHA-256 sidecar 已复制到桌面：`Screenguard_0.4.44_aarch64-ci-39033430993.dmg` 与 `Screenguard_0.4.44_x64-setup-ci-39033430993.exe`；两个 sidecar 回读校验均通过。CI DMG 在无人值守 `hdiutil attach` 时显示内嵌 MIT license 并取消挂载，因此没有把 CI 包误记为已安装。安装验证使用相同 commit 的本机 Apple Silicon Release App。
- 当前 `/Applications/Screenguard.app` 已运行 0.4.44；可执行文件 SHA-256 `0f9779ebceba5f26361000f302ac6bcb751b2ef6931aa8b4c3e0c0474d5e087f`，`codesign` 显示 ad-hoc 且无 Team ID。安装前 0.4.43 App 已备份至 `~/Library/Application Support/Screenguard/backups/pre-0.4.44-app-install-20261017/`；未更改用户的屏幕电源联动开关。运行和版本已核对，不代表 UI 诊断或双屏物理关/唤醒通过。

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
2. macOS 运行仓库检查和 Rust 单测，导入 Developer ID Application 身份，构建 DMG 并对 App 签名，向 Apple 公证并装订票据，然后验证签名、Team ID、公证和 Gatekeeper；Windows 运行仓库检查、C# 桥接检查和 Rust 单测，构建 x64 NSIS 包。macOS 发布需预先配置本节列出的七项 GitHub Actions secrets。
3. 两个安装包均生成 SHA-256 文件。
4. 仅当 tag 的提交已合入 `main` 且双平台构建成功时，自动创建或更新 GitHub Draft Release。
5. Draft 不会自动公开。只有实机验收、升级/回滚检查均有证据后，维护者才可手动发布；macOS 的 Developer ID 签名、公证和 Gatekeeper 检查是 tag 构建的硬门槛。

正式打 tag 前要先完成 PR 审核并合入 `main`，确认仓库版本字段一致，再创建 `vX.Y.Z` tag 并推送。不要在未合入 main 的 PR commit 上创建正式版本 tag；工作流会拒绝这种 tag。

## 变更保护与回滚

- 代码修改先在独立 PR worktree 完成；原始主工作区的未提交修改保持原样。
- 发布工作流新增前保留分支 `backup/pre-release-workflow-20261016`，可用它恢复工作流文件。
- 现有发布资产不覆盖；新版本使用新的 tag、Release 和安装包文件名。
- 发布后回滚通过重新发布上一稳定安装包或修复后发布新补丁版本完成，不移动已公开的历史 tag。
