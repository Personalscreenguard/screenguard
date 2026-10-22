# Screenguard 米家挂灯 1S 局域网探测器

此工具只用于验证 Go/No-Go：它只读取挂灯 `light.on` 状态，不控制灯或显示器。最多运行 180 秒；成功识别 6 次变化后提前结束，连续 3 次读取失败也会提前结束。

令牌由终端隐藏输入，仅短暂保存在进程内存并在退出时清零；不会写入配置、命令行参数、日志或项目文件。请勿把令牌复制到聊天中。

在项目根目录运行：

```sh
cd "/Users/nanyu/Documents/AI项目/Screenguard屏幕守护项目/源码/tools/miot-probe"
SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk cargo run
```

输入挂灯的局域网 IP 和本地令牌后，等待“初始状态”提示，再用实体遥控器完成三次关灯/开灯。每次变化都应在 5 秒内显示。输出达到 `6/6` 才通过；连接失败或少于六次时保留终端错误信息即可，不要发送令牌。
