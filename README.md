# 🖥️ 屏幕守护 Screenguard

跨平台显示器控制与多屏助手（Tauri 2 + Rust）。

面向**双屏日常用户**：主屏 27" 4K + 便携副屏（横/竖切换）的亮度/音量/色彩/布局一体化控制，支持跨屏看电影（VLC 无边框铺满）。

## 功能

| 模块 | 说明 | macOS | Windows | Linux |
|---|---|---|---|---|
| 显示器列表 | 分辨率/刷新率/主副屏/DDC 能力 | ✅ | ✅ | 🟡（部分） |
| 亮度/音量 DDC | 滑块实时控制（需硬件支持 DDC/CI） | ✅ | ✅ | ✅ |
| 色彩同步 | 一键把全部屏对齐 sRGB / Display P3 / Adobe RGB | ✅ | ✅（P3/Adobe 需自备 .icc） | ⏳ |
| 对齐色彩 | macOS：外接屏对齐内建屏（Display P3）；Windows：其余屏对齐主屏当前 ICC | ✅ | ✅ | ⏳ |
| 窗口跨屏等大 | 副屏对齐 2x 逻辑档，窗口拖动不变小（**不会**降 4K 物理分辨率） | ✅ | ✅（诊断+指引） | ⏳ |
| 副屏横竖切换 | 一键旋转/恢复（按真实布局动态计算，无硬编码） | ✅ | ✅ | ⏳ |
| 双屏铺满 | 播放器窗口铺满所有屏的真实包围盒，跨屏看电影 | ✅ | ✅（PotPlayer/VLC） | ⏳ |
| 开机自启 | 面板开关直接管理（macOS LaunchAgent / Windows 注册表 / Linux autostart） | ✅ | ✅ | ✅ |

✅ = 完整实现；🟡 = 部分实现；⏳ = 暂未实现（点击会明确报错，不假成功）

## 依赖

- **macOS**：`displayplacer`（`brew install displayplacer`）、[BetterDisplay](https://betterdisplay.pro)（DDC 读写，需保持运行）、可选 VLC（双屏铺满）
- **Windows**：DDC 走系统 `dxva2.dll`（PowerShell 注入，无需额外软件）；VLC 可选
- **Linux**：`ddcutil`（`apt install ddcutil`）；VLC 可选

> 国内构建 Rust 依赖若直连 crates.io 失败，配置镜像：
> ```toml
> # ~/.cargo/config.toml
> [source.crates-io]
> replace-with = "rsproxy"
> [source.rsproxy]
> registry = "sparse+https://rsproxy.cn/index/"
> [net]
> git-fetch-with-cli = true
> ```

## 开发与构建

```bash
npm install            # 安装 Tauri CLI
npm run dev            # 开发模式
npm run build          # macOS 打包（app + dmg）
npm run build:win      # Windows 打包（需对应 target）
npm run build:linux    # Linux 打包
```

产物：`src-tauri/target/release/bundle/`

## 版本历史

- **v0.2.6**（2026-09-12）：**显示器友好名接入 DisplayConfig（CCD）权威来源**——Win11 24H2 上 PnP/WMI 监视器枚举可能整体失效（本机实测：驱动栈卡死期间 `WmiMonitorID` 返回 0 条），新增 `DisplayNameMap()`：`QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS)` + `GET_SOURCE_NAME`/`GET_TARGET_NAME` 把 `\\.\DISPLAYn` 直接映射到显示器友好名，作为名字匹配第一优先级（WMI 段匹配与 1↔1 兜底保持不变）。实现要点（本机逐一实测踩坑）：① 结构布局对照 wingdi.h 修正——`DISPLAYCONFIG_PATH_SOURCE_INFO` **没有** `reserved` 字段共 20 字节；`DISPLAYCONFIG_TARGET_DEVICE_NAME` 友好名是 `WCHAR[64]` 且中间有 outputTechnology/EDID/connectorInstance 字段共 420 字节；② `QDC_ONLY_ACTIVE_PATHS=2`（`QDC_ALL_PATHS` 才是 1，拿全部路径会得到 80 条非活动路径、源名是 WinDisc 占位）；③ 路径/模式数组必须用原始指针 + Marshal 直读——实测 .NET 结构体数组经 marshaller 进出后数据全零（与 dxva2 `[Out]` 教训同源）；④ 端到端实测 `\\.\DISPLAY2 → Mi Monitor` 命中

- **v0.2.5**（2026-09-11）：**消除全部已知隐患**——① Windows 显示器枚举的 `DISPLAY_DEVICEW` 改为文档字段顺序（`StateFlags` 紧跟 `DeviceString`），实测（ctypes 与 .NET 双路验证）确认旧实现的「`_pad` 隐藏字段」是误诊：**Win11 24H2 起 `EnumDisplayDevicesW` 对「适配器上挂的监视器」的枚举直接失败**（与 cb 取 840/844 无关），uid 为空时回退用设备名作会话内 id，友好名字由 WMI 侧补齐（含剩余 1↔1 配对兜底）；② DEVMODE 由「结构体整块读改写」改为**按固定偏移就地改**，旋转时未声明区间（`dmDeviceName`/`dmFormName` 等）不再被清零；③ **Windows 色彩同步重写**：实测 `WcsAssociateColorProfileWithDevice` 无论设备名对错都返回 `TRUE`、关联却从不落地，已弃用，改走 GDI 设备上下文（`CreateDC` + `SetICMProfileW` + `GetICMProfileW`），并先用 `InstallColorProfileW` 装进系统色彩目录，写入后**另开 DC 回读校验**（同 DC 会读到刚写的缓存值，得出「总是成功」的假阳性）；失败时按是否提权给出可操作指引；④ Windows「对齐 Mac」语义修正为「对齐主屏色彩」（Windows 无内建 P3 屏）；⑤ Windows「窗口跨屏等大」改为读取 `PerMonitorSettings` 的真实 DPI 档做诊断（Windows 改缩放需注销，不再假装能改）；⑥ macOS `match_ppi`/`rotate_secondary` 去掉 `1920x1080` 硬编码，改按 `system_profiler` 原生像素 ÷ 2 **动态推导**目标档；⑦ Linux 未实现功能统一返回明确错误而非静默；⑧ 托盘面板新增独立「关闭」按钮（`hide_panel`）与 Esc 键，不再只有一点就杀进程的「退出」；⑨ 打包配置补齐（`targets: all` + NSIS currentUser + 中英文 + license）；⑩ 补 `LICENSE`（MIT）、仓库自检脚本（`check_repo.py`：版本一致性 / bundle 引用 / JS 语法 / C# 单引号防线）与三平台 CI

- **v0.2.4**（2026-09-11）：**修复 3 个功能阻断 bug**——① 托盘快捷面板整段脚本因顶层 `await` 语法错误而完全不执行（刷新/自启/快捷操作/滑块全部失效）；② Windows 色彩同步的 WCS 设备路径拼接错误（缺 `\`→`#` 转换、且多出 `DISPLAY#` 前缀），「色彩同步 / 对齐 Mac」必然报错；③ Linux 端 xrandr 解析用 `starts_with("connected")` 判断，而该行行首实为接口名 → 检测不到任何显示器。另修：滑块拖动进程风暴（改为串行化 + 合并最新值，同时最多 1 个后台进程）、播放器启动失败不回退下一个、Windows 单实例自检缺失、开机自启路径未加引号、写死 macOS 依赖的跨平台提示文案

- **v0.2.3**（2026-09-10）：**修复 DDC/CI 真正生效的关键 bug**——`GetPhysicalMonitorsFromHMONITOR` 的数组参数缺 `[Out]` 标注，marshaler 不回写导致句柄恒为 0（此前误判为"显示器不支持 DDC/CI"）。参考 emoacht/Monitorian 实现。亮度/音量现已实测可读写；列表加载时顺带读回真实亮度/音量值

- **v0.2.2**（2026-09-09）：修复 DDC/CI 的 P/Invoke 签名错误（输出参数错位导致读写必失败）+ 加读写重试；PPI 诊断改按厂商型号匹配注册表；代码整理（零编译 warning）

- **v0.2.1**（2026-09-09）：Windows 端完整实现（PowerShell + C# Win32 桥接：显示器枚举/DDC/旋转/ICC/双屏铺满，零额外 Rust 依赖）；修复新版 Windows 下显示器 UID 错位、名字匹配；主窗口关闭改驻留托盘
- **v0.2.0**（2026-09-09）：开机自启开关（面板可管）；主/面板显示版本号；全平台自启实现（LaunchAgent/注册表/autostart）
- **v0.1.0**（2026-09-05）：初版——DDC 亮度音量、色彩同步、副屏旋转、双屏铺满（VLC）

## 说明

- 显示器识别全动态（读取 displayplacer / WMI / xrandr），**不依赖硬编码 ID 或分辨率**，换屏即插即用
- macOS 下 4K 副屏以 HiDPI（2x 逻辑档）运行才清晰，"窗口跨屏等大"只对齐逻辑档，绝不降物理分辨率
- 本仓库为个人项目，UI 仅中文

## License

MIT
