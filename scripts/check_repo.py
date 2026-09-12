"""screenguard 仓库自检（跨平台，CI 友好）：
  ① 三处版本号一致（Cargo.toml / tauri.conf.json / package.json）
  ② JSON 文件可解析且能被读到关键字段
  ③ tauri.conf.json 引用的 LICENSE / 图标存在
  ④ ui/*.js 与 ui/*.html 内联 <script> 语法检查（node --check）
     —— 经典脚本里出现顶层 await 会直接报语法错误，
        正是 v0.2.4 修掉的「面板整段脚本失效」那类 bug 的回归防线

用法: python scripts/check_repo.py   （需要 PATH 里有 node；缺失时 JS 检查跳过并提示）
"""
import json
import os
import re
import subprocess
import sys
import tempfile

# GitHub windows runner 的 stdout 默认 cp1252，直接 print 中文/✅ 会 UnicodeEncodeError
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
UI = os.path.join(ROOT, "ui")
INLINE_RE = re.compile(r"<script(?P<attrs>[^>]*)>(?P<code>.*?)</script>", re.S)


def fail(msg):
    print(f"  ❌ {msg}")
    return False


def ok(msg):
    print(f"  ✅ {msg}")
    return True


def check_versions():
    print("[1/4] 版本一致性")
    good = True
    cargo = open(os.path.join(ROOT, "src-tauri", "Cargo.toml"), encoding="utf-8").read()
    m = re.search(r'^version\s*=\s*"([^"]+)"', cargo, re.M)
    vs = {
        "Cargo.toml": m.group(1) if m else None,
        "tauri.conf.json": json.load(open(os.path.join(ROOT, "src-tauri", "tauri.conf.json"),
                                          encoding="utf-8")).get("version"),
        "package.json": json.load(open(os.path.join(ROOT, "package.json"),
                                       encoding="utf-8")).get("version"),
    }
    print("  ", vs)
    if None in vs.values():
        good = fail("有文件读不到 version 字段")
    elif len(set(vs.values())) != 1:
        good = fail("三处版本号不一致")
    else:
        ok(f"全部为 {vs['Cargo.toml']}")
    return good


def check_bundle_refs():
    print("[2/4] tauri.conf.json 引用完整性")
    conf = json.load(open(os.path.join(ROOT, "src-tauri", "tauri.conf.json"), encoding="utf-8"))
    good = True
    lic = conf.get("bundle", {}).get("licenseFile")
    if lic:
        p = os.path.normpath(os.path.join(ROOT, "src-tauri", lic))
        good = (ok(f"licenseFile 存在: {lic}") if os.path.exists(p) else fail(f"licenseFile 不存在: {lic}")) and good
    for icon in conf.get("bundle", {}).get("icon", []):
        p = os.path.join(ROOT, "src-tauri", icon)
        if not os.path.exists(p):
            good = fail(f"图标不存在: {icon}")
    if good:
        ok(f"全部 {len(conf.get('bundle', {}).get('icon', []))} 个图标存在")
    return good


def node_available():
    try:
        subprocess.run(["node", "--version"], capture_output=True, check=True)
        return True
    except (OSError, subprocess.CalledProcessError):
        return False


def node_check(label, code):
    fd, tmp = tempfile.mkstemp(suffix=".js")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(code)
        r = subprocess.run(["node", "--check", tmp], capture_output=True, text=True)
        if r.returncode == 0:
            return True
        print(f"  ❌ JS 语法错误: {label}")
        print("     " + (r.stderr or "").strip().splitlines()[0] if r.stderr else "")
        return False
    finally:
        os.unlink(tmp)


def check_js():
    print("[3/4] 前端 JS 语法（node --check；临时文件按 CommonJS 解析，顶层 await 会报错）")
    if not node_available():
        print("  ⚠️ node 不可用，跳过（CI 中会执行此检查）")
        return True
    good = True
    n = 0
    for name in sorted(os.listdir(UI)):
        p = os.path.join(UI, name)
        if name.endswith(".js"):
            n += 1
            good = node_check(name, open(p, encoding="utf-8").read()) and good
        elif name.endswith(".html"):
            src = open(p, encoding="utf-8").read()
            for i, m in enumerate(INLINE_RE.finditer(src)):
                attrs = m.group("attrs")
                if "src=" in attrs:  # 外链脚本不内联检查
                    continue
                n += 1
                good = node_check(f"{name} 第{i + 1}个内联<script>", m.group("code")) and good
    if good:
        ok(f"共检查 {n} 段脚本，语法全部通过")
    return good


def check_csharp_quote():
    """windows.rs 的 CORE_CS 内不允许出现单引号（PS 单引号包裹会截断）——与 check_cs.py 同规则的前置防线"""
    print("[4/4] windows.rs CORE_CS 单引号检查")
    src = open(os.path.join(ROOT, "src-tauri", "src", "platform", "windows.rs"), encoding="utf-8").read()
    m = re.search(r'const CORE_CS: &str = r#"(.*?)"#;', src, re.S)
    if not m:
        return fail("CORE_CS 未找到")
    if "'" in m.group(1):
        return fail("C# 内含单引号，会被 Add-Type 单引号包裹截断")
    return ok("无单引号")


def main():
    print("=" * 60)
    print("screenguard 仓库自检")
    print("=" * 60)
    good = all([check_versions(), check_bundle_refs(), check_js(), check_csharp_quote()])
    print("=" * 60)
    print("✅ 全部通过" if good else "❌ 存在问题")
    sys.exit(0 if good else 1)


if __name__ == "__main__":
    main()
