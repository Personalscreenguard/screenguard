$ErrorActionPreference = "Stop"

$required = @("WINDOWS_CERTIFICATE", "WINDOWS_CERTIFICATE_PASSWORD", "WINDOWS_SIGNING_CERT_THUMBPRINT")
foreach ($name in $required) {
  if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
    throw "缺少 GitHub Actions secret: $name"
  }
}

$installers = @(Get-ChildItem "src-tauri/target/release/bundle/nsis/*-setup.exe")
if ($installers.Count -ne 1) { throw "预期一个 NSIS 安装包，实际数量: $($installers.Count)" }

$pfxPath = Join-Path $env:RUNNER_TEMP "screenguard-signing.pfx"
try {
  $pfxBytes = [Convert]::FromBase64String(($env:WINDOWS_CERTIFICATE -replace "\s", ""))
  [IO.File]::WriteAllBytes($pfxPath, $pfxBytes)
  $certificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new(
    $pfxBytes,
    $env:WINDOWS_CERTIFICATE_PASSWORD,
    [Security.Cryptography.X509Certificates.X509KeyStorageFlags]::EphemeralKeySet
  )
  $expectedThumbprint = ($env:WINDOWS_SIGNING_CERT_THUMBPRINT -replace "\s", "").ToUpperInvariant()
  if ($certificate.Thumbprint.ToUpperInvariant() -ne $expectedThumbprint) {
    throw "Windows 签名证书 Thumbprint 与配置不匹配"
  }
  if ($certificate.NotAfter.ToUniversalTime() -le [DateTime]::UtcNow) {
    throw "Windows 签名证书已过期"
  }

  $sdkRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
  $signtool = Get-ChildItem $sdkRoot -Directory | Sort-Object { [version]$_.Name } -Descending |
    ForEach-Object { Join-Path $_.FullName "x64\signtool.exe" } |
    Where-Object { Test-Path $_ } | Select-Object -First 1
  if (-not $signtool) { throw "Windows SDK signtool.exe 未找到" }

  & $signtool sign /fd SHA256 /td SHA256 /tr "http://timestamp.digicert.com" /f $pfxPath /p $env:WINDOWS_CERTIFICATE_PASSWORD $installers[0].FullName
  if ($LASTEXITCODE -ne 0) { throw "signtool sign 失败，退出码: $LASTEXITCODE" }

  $signature = Get-AuthenticodeSignature $installers[0].FullName
  if ($signature.Status -ne "Valid") { throw "安装包签名验证失败: $($signature.Status) $($signature.StatusMessage)" }
  if ($signature.SignerCertificate.Thumbprint.ToUpperInvariant() -ne $expectedThumbprint) {
    throw "安装包签名者 Thumbprint 与配置不匹配"
  }
  if (-not $signature.TimeStamperCertificate) { throw "安装包缺少可信时间戳" }
  Write-Host "Windows Authenticode 签名、证书身份与时间戳验证通过。"
} finally {
  Remove-Item $pfxPath -Force -ErrorAction SilentlyContinue
}
