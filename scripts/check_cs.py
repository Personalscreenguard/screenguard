"""从 windows.rs 提取 CORE_CS (r#"..."#) 并做 C# 语法编译检查。
用法: python check_cs.py  → 输出 COMPILE_OK / 编译错误
"""
import re, subprocess, sys, os

RS = os.path.expanduser("~/projects/screenguard/src-tauri/src/platform/windows.rs")
src = open(RS, encoding="utf-8").read()
m = re.search(r'const CORE_CS: &str = r#"(.*?)"#;', src, re.S)
if not m:
    print("CORE_CS 未找到")
    sys.exit(1)
cs = m.group(1)
# Rust 源码里的 {} 不是转义(在 r#""# 里原样)，但 PS 脚本用了 {{ }}——CORE_CS 内部无 {} 转义需求
# 检查单引号（PS 单引号包裹会截断）
if "'" in cs:
    print("C# 内含单引号，会被 Add-Type 单引号包裹截断！")
    sys.exit(1)

# 注意：windows.rs 用 format! 生成 PS 脚本，CORE_CS 本身作为 {cs} 参数——内部花括号会怎样?
# CORE_CS 是常量，不经过 format!（它是 format! 的实参值，不是格式串），所以原样保留。
tmp = os.path.join(os.environ.get("LOCALAPPDATA", "C:/Windows/Temp"), "Temp", "sg_core_check.cs")
os.makedirs(os.path.dirname(tmp), exist_ok=True)
open(tmp, "w", encoding="utf-8").write(cs)

ps = f'''Add-Type -TypeDefinition (Get-Content -Raw -Encoding UTF8 '{tmp.replace(chr(92), "/")}') -Language CSharp -ErrorAction Stop; Write-Host COMPILE_OK'''
r = subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", ps],
                   capture_output=True, text=True, encoding="utf-8", errors="replace")
out = (r.stdout or "") + (r.stderr or "")
if "COMPILE_OK" in out:
    print("COMPILE_OK  (C# 桥接代码语法正确)")
else:
    print("COMPILE_FAILED:")
    print(out[:2000])
