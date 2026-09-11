"""从 windows.rs 提取 CORE_CS (r#"..."#) 并做 C# 语法编译检查。
用法: python check_cs.py  → 输出 COMPILE_OK / COMPILE_FAILED（失败时退出码 1）
"""
import os
import re
import subprocess
import sys

RS = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                   "..", "src-tauri", "src", "platform", "windows.rs"))

src = open(RS, encoding="utf-8").read()
m = re.search(r'const CORE_CS: &str = r#"(.*?)"#;', src, re.S)
if not m:
    print("CORE_CS 未找到")
    sys.exit(1)
cs = m.group(1)
# CORE_CS 是常量、作为 format! 的实参（不是格式串），内部 {} 原样保留；
# 但 PS 用单引号包裹传给 Add-Type，C# 内若含单引号会截断 —— 直接拒绝
if "'" in cs:
    print("C# 内含单引号，会被 Add-Type 单引号包裹截断！")
    sys.exit(1)

tmp = os.path.join(os.environ.get("TEMP", os.environ.get("TMP", "/tmp")), "sg_core_check.cs")
os.makedirs(os.path.dirname(tmp), exist_ok=True)
open(tmp, "w", encoding="utf-8").write(cs)

ps = f'''Add-Type -TypeDefinition (Get-Content -Raw -Encoding UTF8 '{tmp.replace(chr(92), "/")}') -Language CSharp -ErrorAction Stop; Write-Host COMPILE_OK'''
r = subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", ps],
                   capture_output=True, text=True, encoding="utf-8", errors="replace")
out = (r.stdout or "") + (r.stderr or "")
if "COMPILE_OK" in out:
    print("COMPILE_OK  (C# 桥接代码语法正确)")
    sys.exit(0)
print("COMPILE_FAILED:")
print(out[:2000])
sys.exit(1)
