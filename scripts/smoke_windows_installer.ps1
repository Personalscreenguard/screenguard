param(
  [Parameter(Mandatory = $true)]
  [string]$InstallerPath
)

$ErrorActionPreference = 'Stop'
$installer = (Resolve-Path -LiteralPath $InstallerPath).Path
$installDir = Join-Path $env:RUNNER_TEMP ("screenguard-smoke-" + [guid]::NewGuid().ToString('N'))
$appExe = Join-Path $installDir 'screenguard.exe'
$uninstaller = $null
$uninstalled = $false

function Invoke-WaitedProcess {
  param(
    [Parameter(Mandatory = $true)][string]$FilePath,
    [Parameter(Mandatory = $true)][string[]]$ArgumentList,
    [Parameter(Mandatory = $true)][string]$Label
  )

  $process = Start-Process -FilePath $FilePath -ArgumentList $ArgumentList -PassThru
  if (-not $process.WaitForExit(180000)) {
    try { $process.Kill() } catch { }
    throw "$Label 超时（180 秒）"
  }
  $process.Refresh()
  if ($process.ExitCode -ne 0) {
    throw "$Label 退出码为 $($process.ExitCode)"
  }
}

try {
  Write-Host "静默安装 NSIS 包到临时目录：$installDir"
  # Tauri NSIS supports /S for silent mode, /NS to suppress shortcuts, and /D=... as the final argument.
  Invoke-WaitedProcess -FilePath $installer -ArgumentList @('/S', '/NS', "/D=$installDir") -Label 'NSIS 安装'

  if (-not (Test-Path -LiteralPath $appExe -PathType Leaf)) {
    throw "安装完成后找不到主程序：$appExe"
  }
  if ((Get-Item -LiteralPath $appExe).Length -le 0) {
    throw "安装后的主程序文件为空：$appExe"
  }

  $uninstaller = Get-ChildItem -LiteralPath $installDir -Filter '*.exe' -File |
    Where-Object { $_.Name -match '(?i)^uninstall.*\.exe$' } |
    Select-Object -First 1
  if (-not $uninstaller) {
    throw "安装目录中找不到 NSIS 卸载程序：$installDir"
  }

  Write-Host "开始静默卸载：$($uninstaller.FullName)"
  Invoke-WaitedProcess -FilePath $uninstaller.FullName -ArgumentList @('/S') -Label 'NSIS 卸载'
  $uninstalled = $true
  if (Test-Path -LiteralPath $appExe) {
    throw "卸载后主程序仍然存在：$appExe"
  }
  Write-Host 'NSIS 静默安装/卸载烟雾检查通过。'
}
finally {
  if (-not $uninstalled -and $uninstaller -and (Test-Path -LiteralPath $uninstaller.FullName)) {
    try {
      Invoke-WaitedProcess -FilePath $uninstaller.FullName -ArgumentList @('/S') -Label '失败清理时的 NSIS 卸载'
    } catch {
      Write-Warning "临时安装卸载清理失败：$_"
    }
  }
  if (Test-Path -LiteralPath $installDir) {
    Remove-Item -LiteralPath $installDir -Recurse -Force
  }
}
