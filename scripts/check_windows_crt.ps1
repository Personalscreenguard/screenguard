param(
  [string]$ExecutablePath = 'src-tauri/target/release/screenguard.exe'
)

$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path -LiteralPath $ExecutablePath).Path
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path -LiteralPath $vswhere)) {
  throw '找不到 Visual Studio Installer 的 vswhere.exe，无法审计 Windows CRT 依赖。'
}

$vsRoot = & $vswhere -latest -products '*' -property installationPath
if (-not $vsRoot) { throw '找不到 Windows MSVC 工具链安装路径。' }
$dumpbin = Get-ChildItem -Path (Join-Path $vsRoot 'VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe') -File |
  Sort-Object FullName |
  Select-Object -Last 1
if (-not $dumpbin) { throw '找不到 MSVC dumpbin.exe。' }

$output = & $dumpbin.FullName /DEPENDENTS $exe 2>&1
if ($LASTEXITCODE -ne 0) { throw "dumpbin 无法读取应用程序导入表：$exe" }
$output | ForEach-Object { Write-Host $_ }

$crtImports = @($output | Select-String -Pattern '(?i)\b(?:vcruntime|msvcp|concrt)[a-z0-9_]*\.dll\b')
if ($crtImports.Count -gt 0) {
  throw "Windows 应用仍依赖独立 Visual C++ Runtime，用户可能需要另装 VC++ Redistributable：$($crtImports -join '; ')"
}
Write-Host 'Windows 主程序未发现 VCRUNTIME、MSVCP 或 CONCRT 导入；无需因这些 DLL 单独安装 VC++ Redistributable。'
