"""screenguard Windows 桥接端到端自检（只读为主，绝不旋转/移动窗口）

做法：把 windows.rs 里的 CORE_CS 抽出来交给 PowerShell 的 Add-Type 编译，
      再直接调用 [SGCore] 的公开方法，验证：
        ① ListDisplays —— DISPLAY_DEVICEW 结构体布局修正后 uid / 分辨率 / 主副屏是否正确
        ② IccGet / IccInstall / IccSet —— ICC 读取、安装、关联+回读校验（幂等，不改视觉）
        ③ whoami 提权判定

用法:
    python scripts/e2e_bridge.py            # 只读检查
    python scripts/e2e_bridge.py --write    # 额外做 ICC 写入校验（幂等，失败也不改变状态）
"""
import os
import re
import subprocess
import sys
import tempfile

RS = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                  "..", "src-tauri", "src", "platform", "windows.rs")

SRGB = r"C:\Windows\System32\spool\drivers\color\sRGB Color Space Profile.icm"


def extract_core_cs():
    src = open(RS, encoding="utf-8").read()
    m = re.search(r'const CORE_CS: &str = r#"(.*?)"#;', src, re.S)
    if not m:
        print("CORE_CS 未找到")
        sys.exit(1)
    cs = m.group(1)
    if "'" in cs:
        print("C# 内含单引号，会被 Add-Type 单引号包裹截断！")
        sys.exit(1)
    return cs


def run_ps(body, cs):
    """把 CORE_CS 落盘后，在同一次 PowerShell 会话里执行 body"""
    tmpdir = tempfile.mkdtemp(prefix="sg_e2e_")
    cs_path = os.path.join(tmpdir, "sg_core.cs")
    open(cs_path, "w", encoding="utf-8").write(cs)
    script = (
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8;\n"
        f"Add-Type -TypeDefinition (Get-Content -Raw -Encoding UTF8 '{cs_path.replace(os.sep, '/')}') -Language CSharp -ErrorAction Stop;\n"
        + body
    )
    r = subprocess.run(
        ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script],
        capture_output=True, text=True, encoding="utf-8", errors="replace",
    )
    return (r.stdout or "") + (r.stderr or "")


def main():
    write_mode = "--write" in sys.argv
    cs = extract_core_cs()
    print("=" * 70)
    print("screenguard Windows 桥接端到端自检")
    print("=" * 70)

    body = r"""
$lines = [SGCore]::ListDisplays() -split ([char]10);
$devs = @();
foreach ($l in $lines) {
  if ($l.Trim()) { Write-Output ("RAW|" + $l) }
  $p = $l -split '\|';
  if ($p.Count -ge 8 -and $p[0] -eq 'D') { $devs += $p[1] }
}
Write-Output ("DEVCOUNT|" + $devs.Count)
Write-Output "NAMEMAP-START";
[SGCore]::DisplayNameMap() -split ([char]10) | ForEach-Object { if ($_.Trim()) { Write-Output ("MAP|" + $_) } }
foreach ($d in $devs) { Write-Output ("ICC|" + $d + "|" + [SGCore]::IccGet($d)) }
Write-Output ("INSTALL|" + [SGCore]::IccInstall('PROFILE_PLACEHOLDER'))
"""
    body = body.replace("PROFILE_PLACEHOLDER", SRGB)

    if write_mode:
        # 正向：把「当前已关联的同一配置」再关联一次 —— 幂等，且能验证回读校验的通过分支
        body += r"""
foreach ($d in $devs) {
  Write-Output ("SET_SAME|" + $d + "|" + [SGCore]::IccSet($d, 'PROFILE_PLACEHOLDER'))
}
""".replace("PROFILE_PLACEHOLDER", SRGB)

        # 反例：关联一个「内容相同但路径不同」的配置副本。
        # 预期 ERR —— 说明回读校验确实在起作用（Windows 权限不足时会返回成功但不生效）。
        # 视觉零风险：副本内容与 sRGB 完全一致；无论成败随后都重新关联回系统 sRGB。
        import shutil
        copy = os.path.join(tempfile.gettempdir(), "sg_srgb_copy.icm")
        try:
            shutil.copyfile(SRGB, copy)
        except OSError as e:
            print(f"（副本创建失败，跳过反例：{e}）")
            copy = None
        if copy:
            body += r"""
foreach ($d in $devs) {
  Write-Output ("SET_OTHER|" + $d + "|" + [SGCore]::IccSet($d, 'COPY_PLACEHOLDER'))
}
foreach ($d in $devs) {
  Write-Output ("RESTORE|" + $d + "|" + [SGCore]::IccSet($d, 'PROFILE_PLACEHOLDER'))
}
""".replace("COPY_PLACEHOLDER", copy).replace("PROFILE_PLACEHOLDER", SRGB)

    out = run_ps(body, cs)
    print(out.rstrip())

    print("-" * 70)
    print("提权判定（whoami /groups 完整性级别）")
    g = subprocess.run(["whoami", "/groups"], capture_output=True, text=True,
                       encoding="utf-8", errors="replace").stdout
    high = "S-1-16-12288" in g
    med = "S-1-16-8192" in g
    print(f"  is_elevated() 应为 {high}（高完整性 S-1-16-12288 存在={high}，中完整性存在={med}）")

    print("-" * 70)
    if "RAW|" not in out:
        print("❌ ListDisplays 无输出 —— 桥接可能整体失败")
        sys.exit(1)
    print("✅ 桥接调用成功。判读要点：")
    print("   * RAW| 行的 uid 形如 MONITOR\\XMI27B3\\{4d36e96e-...}\\0007 → 结构体布局正确")
    print("   * MAP| 行应形如 M|\\\\.\\DISPLAY1|<友好名> → DisplayConfig 映射正常（24H2 上名字的权威来源）")
    print("   * ICC| 行应为 OK:C:\\WINDOWS\\...\\sRGB Color Space Profile.icm")
    print("   * SET_SAME| 行应为 OK（关联同一配置，幂等；ERR 说明回读校验判定未生效）")
    if not write_mode:
        print("   （未启用 --write，跳过 ICC 写入校验）")


if __name__ == "__main__":
    main()
