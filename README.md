# 🖥️ 屏幕守护 Screenguard

跨平台显示器控制与多屏助手（Tauri 2 + Rust）。

面向**双屏日常用户**：主屏 27" 4K + 便携副屏（横/竖切换）的亮度/音量/色彩/布局一体化控制，支持跨屏看电影（VLC 无边框铺满）。

## 功能

| 模块 | 说明 | macOS | Windows | Linux |
|---|---|---|---|---|
| 显示器列表 | 分辨率/刷新率/主副屏/DDC 能力 | ✅ | ✅ | 🟡（部分） |
| 亮度/音量 DDC | 滑块实时控制（需硬件支持 DDC/CI） | ✅ | ✅ | ✅ |
| 色彩同步 | 一键把全部屏对齐 sRGB / Display P3 / Adobe RGB | ✅ | ✅（P3/Adobe 需自备 .icc） | ⏳ |
| 对齐 Mac | 外接屏对齐 Mac 内建屏（Display P3） | ✅ | ✅（同上） | ⏳ |
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
