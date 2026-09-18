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

| 显示器健康 | 链路异常检测（读事件日志）/ 一键修复（强制重协商）/ 自动修复 + 掉线提醒 | ⏳ | ✅ | ⏳ |

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

- **v0.3.2**（2026-09-19）：**总亮度 · 全局快捷键 · 电池健康度与耗电曲线 · 色彩/HDR 同步 · 任意窗口铺满 · 面板重排**——按用户逐条反馈落实：① **软件层总亮度**（新「系统控制」分区，与系统音量同区）：走 GDI `SetDeviceGammaRamp`，**不依赖 DDC/CI**，因此对**读取不到 DDC 的小米主屏同样生效**；三通道写同一条曲线（只改明暗、不动色相），限幅 40~160 防误操作，配「亮度复位」；gamma ramp 是会话级、重启自动还原，不落盘、不改 EDID/ICC。② **全局快捷键 `Ctrl+Alt+L`**：Rust 原生 `RegisterHotKey` + 独立线程消息循环（零额外 Rust 依赖），切换「全部屏幕待机 / 唤醒」；用 `GetLastInputInfo` 判断是否被其它输入唤醒过，避免状态错乱；键位直接显示在界面上。③ **小米主屏亮度/音量读不到的真因**（用户反馈「Mac 上能识别」）：两块屏走不同 GPU 分支（小米 `4&1f78adb9`=NVIDIA 分支，副屏 `5&1623317c`=Intel iGPU 分支），**小米所在的 DP→KVM 链路不向 Windows 转发 DDC/CI**（同一台屏在 Mac 上直连可读），且小米 MiTV Assistant（6095/9095）**没有任何亮度接口**（实测 `getbrightness` 无响应、其余 404）→ 结论：不是软件 bug，改由「总亮度」gamma 统一兜底。④ **电池设备增强**：新增**健康度**（满充容量 ÷ 设计容量）与**耗电柱状图**（每次刷新采样，本地留 40 个样本，画迷你柱状图并标注时段与增减量）；UI 明确提示「保养模式限充 80% 会让健康度显示约 80%，属正常不是损耗」，避免误报焦虑。⑤ **色彩同步扩充**：8 种目标色彩空间（sRGB / Display P3 / Adobe RGB / DCI-P3 / Rec.709 / Rec.2020 / ProPhoto / 灰度），ICC 查找改为**别名模糊匹配**（系统色彩目录 + `%LOCALAPPDATA%\screenguard_icc\`），放入对应 `.icc` 即可用，不再硬编码三种；新增 **亮度同步**（各屏 DDC 亮度取齐）与 **HDR 同步**（读 `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO` / 写 `SET_ADVANCED_COLOR_STATE`，实测本机两块屏都支持且原先都开着）。⑥ **「窗口 / 多屏布局」真正落地**：旧版只做诊断（Windows 改每屏缩放必须注销、没有运行时 API），现实现**写 HKCU `PerMonitorSettings\DpiValue`** 把副屏缩放档对齐主屏，并**自动备份原值 + 提供「还原缩放」**。⑦ **双屏铺满支持任意窗口**：新增 `SpanForeground` / `RestoreForeground` —— 以**当前前台窗口**为目标（不再限定播放器），浏览器里的抖音/视频全屏、PotPlayer、VLC 都能一键横跨两块屏；原位置与窗口样式一并记住，「还原」精确回退；已加防呆（连点两次不会覆盖备份）。实测窗口矩形 `795×737 → 4160×2560`，还原后精确回到 `883,331,1678,1068`。⑧ **面板体验**：「检查记录」改**可折叠**（默认收起、一按展开、列表限高可滚）；**快捷控制「颜色三合一」**（点一下在 `sRGB → P3 → AdobeRGB` 间循环，腾出的格子补上 亮度同步 / HDR 同步 / 任意窗口铺满）；**彻底禁止横向滚动**（`overflow-x:hidden` + 全局长词断行，实测 `scrollWidth == clientWidth == 380`）；托盘面板**贴合屏幕顶端**（`y = 0`）。踩坑：① 往 `CORE_CS` 加方法前必须确认类边界——本次误删过一次函数签名，靠 `cargo check` 立即抓回；② **C# 里绝对不能出现单引号**（会截断 PowerShell 的单引号字符串），新写的 JSON 迷你解析器一律用双引号 + 字符码（`32 / 9 / 45`）比较；③ 「铺满」的备份若被重复点击覆盖，「还原」就永久失效——必须判断「已铺满且是同一窗口」时保留旧备份；④ **PowerShell 非 DPI 感知进程读到的窗口坐标是被虚拟化的**，跨屏验证不能直接拿它和 `GetSystemMetrics` 的虚拟屏尺寸比大小。
- **v0.3.1**（2026-09-18）：**显示器故障检测 · 一键修复 · 自动修复与掉线提醒**——新增面板「显示器健康」分区：① **检查记录**：读 Windows 事件日志，列出近 24 小时显示链路异常（`Kernel-PnP/Device Management` 的**事件 ID 1010 = 设备被突然移除**、`Driver Watchdog` 长时间阻塞、`Display` 驱动重置 4101）；② **一键修复**：对每台在用屏强制重新协商显示配置（先把刷新率降到 60Hz，再切回原模式，两次 `ChangeDisplaySettingsEx` 触发完整 EDID／色彩格式重协商）——专治 KVM 切换、显示器开关、电脑重启后出现的**画面发白发亮（色彩格式被降级成 YCbCr）、清晰度变差、抖动花屏**，实测两台屏均恢复（2400×3840@60 / 3840×2160@120）；③ **深度修复**：轻修无效时追加系统级关屏 + 唤醒，强制整条链路重新握手；④ **自动修复**（可开关）：后台线程每 45 秒查一次日志，发现显示器掉线且 6 秒后仍没自行恢复即自动重新协商（150 秒冷却）；⑤ **掉线提醒**（可开关）：右下角自建气泡窗口（不引入 notification 插件），12 秒自动消失、每次显示重新计时。两个开关持久化在 `%APPDATA%\Screenguard\health.json`。

  **上线第一件事就查出了真凶**：近 24 小时 **40 次**显示器突然离线（自述「一周几次」，实际平均每 36 分钟一次，多数瞬间恢复未被察觉）。每次掉线的设备组合完全一致——**两台显示器 + KVM 内部的 USB Hub（`USB\VID_05E3&PID_061x`，Genesys）+ 音频设备同时消失**，而 TDR 事件 0 次、nvlddmkm 错误 0 条 → 证明**不是驱动崩溃、也不是某块屏故障，而是 KVM 整条链路断开**（USB Hub 一起掉是供电不足的典型特征）。事件里还抓到 `DISPLAY\@@@@0000` 这类 **EDID 厂商字节全零的损坏身份**，与 v0.2.8 那次「KVM 破坏 EDID」的结论吻合。**治本建议**：检查 KVM 的 USB 上游线与供电（建议用自带电源适配器），或换支持 EDID 模拟的 KVM、把高刷屏直连雷电口。

  踩坑：① **PowerShell `-split ([char]10)` 不按换行分割**——`-split` 的参数走正则，传 `[char]` 对象会退化成空模式按**每个字符**切（实测 31 字符的行被切成 31 段、解析结果永远为空，导致一键修复报「未枚举到可用显示器」）；按分隔符切字符串必须用 .NET 的 `String.Split([char]N)`；② **C# 桥接方法要放进正确的类**——`CORE_CS` 里有多达 4 个类，往文件尾部追加方法会落进 `SGBatt`，编译报「当前上下文中不存在 AllocDevMode」，加方法前先 `grep -n "^public class"` 确认目标类的起止行；③ **`EventLogQuery` 没有 `Tq` 属性**（XPath 必须作为第三个构造参数传入）、**`EventLogReader` 没有 `Close()`**（用 `Dispose()`）——任一处写错都会让 `Add-Type` 整体编译失败、依赖该类的全部命令静默失效（改完 C# 立即跑 `scripts/check_cs.py`）；④ **气泡通知的时序**——窗口首次创建时页面还没加载完，Rust 的 `emit` 会丢；同一窗口复用第二次显示时也没有重新执行的入口，改为「窗口可见时主动拉取 + 兜底轮询」并在每次显示时重置自动关闭计时器；⑤ 面板内容溢出被裁（`overflow:hidden`）导致新分区落在屏幕外看不见，改为整页滚动 + 标题栏 `sticky`

- **v0.3.0**（2026-09-17）：**屏幕待机 / 黑屏（两套方案）**——新增「屏幕电源」卡（主窗口 + 托盘面板）。**方案A（纯本地，不联网也能用）**：① 系统级「全部待机」一次黑掉所有屏（含不走 DDC/CI 的显示器，这是唯一能影响到它们的软件手段）；② 单台 DDC 电源模式控制（VCP `0xD6`：开 / 待机 / 软关，可回读）；③ **联动开关**——轮询在线显示器快照，某台离线时其余屏自动跟着待机、回来时跟着点亮（2 次采样防抖 + 15 秒冷却，避免切输入源瞬间抖动误触发）。**方案B（联网增强，可选）**：接上显示器的网络 ADB 后，电脑可直接控制显示器本体（电源 / 音量 / 输入源），为「按遥控器联动」铺路。踩坑：① **PowerShell 的退出码不可信**——脚本 stdout 已完整输出，但只要过程中产生过被抑制的 ErrorRecord，退出码就是 1，`run_cmd` 只看退出码会把成功判成失败（v0.2.9 的显示器列表实际一直被整个丢弃），改为「有 stdout 就以 stdout 为准」；② **C# 桥接代码不能内联进 `Add-Type -TypeDefinition`**——累积到 30KB+ 就撞 Windows 命令行长度上限（`os error 206`），改为落盘 + `Add-Type -Path`（必须带 UTF-8 BOM，否则 PowerShell 5.1 按 ANSI 读中文注释导致编译失败）；③ 系统级关屏必须用 `SendMessageTimeout` 带超时，同步 `SendMessage` 广播会被无响应窗口卡死（实测卡 >90 秒）；④ 显示器 DDC 待机后**不会从系统消失**，所以「某台屏是否已关」不能靠「它从设备列表消失」判断；⑤ 名字兜底逻辑漏算了 DisplayConfig 这条来源，导致两台屏显示成同一个名字；⑥ 联动防抖若在检测到变化时就更新基线，第二次采样就看不到变化、永远凑不满确认次数，联动会彻底不触发。**方案B 的 adb 不必下载**：ROG 机型预装的 ASUS GlideX 自带 adb（1.0.41），程序会自动探测
- **v0.2.9**（2026-09-12）：**带电池的连接设备电量监控**——新增「电池设备」卡（主窗口 + 托盘面板），枚举所有带电池的连接设备（笔记本内电池 / USB·2.4G 无线键鼠 / 蓝牙耳机等），显示**电量百分比 + 充电状态**（充电中 / 已接通电源 / 放电中）。实现走 Windows 统一标准通道：SetupAPI 按 `GUID_DEVCLASS_BATTERY` 枚举所有电池设备接口，再 `CreateFile` + `IOCTL_BATTERY_QUERY_TAG / QUERY_INFORMATION / QUERY_STATUS` 取满充容量、剩余容量、电源状态位。踩坑：① `FILE_DEVICE_BATTERY` 是 **0x29 不是 0x22**，控制码 `IOCTL_BATTERY_QUERY_TAG=0x294040 / QUERY_INFORMATION=0x294044 / QUERY_STATUS=0x29404C`（写错会 `ERROR_INVALID_FUNCTION`）；② `BATTERY_INFORMATION.FullChargedCapacity` 在**偏移 16**（不是 28，前面有 DesignedCapacity=12）；③ `PowerState` 位序是 `POWER_ON_LINE=0x1 / DISCHARGING=0x2 / CHARGING=0x4`；④ 内置电池通常无友好名，直接显示「内置电池」而非序列号，外设回退读电池栈 `BatteryDeviceName`。绝对模式（mWh）与相对模式（HID 外设百分比）都兼容

- **v0.2.8**（2026-09-12）：**DDC/CI 驱动栈加固**——实机排查发现外接显示器出现清晰度/横竖屏/复制扩展错乱，根因是 **NVIDIA 驱动崩溃循环（nvlddmkm Event 14/153，TDR 复位失败）**，而非软件改动系统底层：① 显卡驱动 TDR 期间，`GetPhysicalMonitorsFromHMONITOR`/`SetVCPFeature` 等 dxva2 调用会长时间阻塞或抛异常，且此前 `TryVCP` 无 try/catch、异常会沿 `EnumDisplayMonitors` 回调冒泡；② DDC 读写失败时盲目 120ms×N 重试，在驱动复位循环里高频 DDC 超时反而加剧 TDR。修复：`TryVCP` 整体 try/catch（驱动异常静默快速失败，回退 WMI/提示兜底，不再冒泡崩进程）、`DestroyPhysicalMonitors` 移入 finally（任何路径释放物理显示器句柄防泄漏）、单次失败退避（sleep 从 120ms 降到 80ms 且只在还有重试机会时 sleep）。**恢复手段不变：`Win+Ctrl+Shift+B` 重启图形驱动；建议更新 NVIDIA 驱动根治崩溃循环**

- **v0.2.7**（2026-09-12）：**亮度/音量控制补全「最后一公里」**——实机诊断发现两个结构性盲区：① 笔记本内屏（eDP 面板）**根本没有 DDC/CI 通路**，旧版只有 DDC 一条路，内屏亮度必然不可控 → 新增 **WMI 亮度回退**（`root\wmi` 的 `WmiMonitorBrightness` 读 / `WmiMonitorBrightnessMethods.WmiSetBrightness` 写，与系统亮度滑块同源，实测 100→80→100 生效）；② 音量此前只做 DDC 0x62（显示器喇叭），笔记本喇叭/耳机/HDMI 音频完全没覆盖 → 新增**系统音量控制**（CoreAudio `IAudioEndpointVolume`，控制默认输出端点，与托盘音量同源；主窗口与托盘面板均有滑块 + 静音开关）。CoreAudio 互操作实测踩坑三连：**PROPVARIANT 内联值（VT_UI4）不能当指针解引用**（访问违例直接崩进程）；**GetMute/SetMute 的 BOOL 必须用 int 编组**（COM 里 `bool` 默认按 VARIANT_BOOL 2 字节编组，API 写 4 字节 → 栈帧损坏 → 莫名 NRE）；属性读取分步防御化。另有：驱动栈异常启发式提示（仅一块屏且运行在 1024x768@60 时提示 `Win+Ctrl+Shift+B` 重启图形驱动——本机 NVIDIA 驱动崩溃循环实测亮度/音量/枚举会整体失效）；亮度设置失败时按 DDC/WMI 分别给出可操作指引

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
