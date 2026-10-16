param([string]$DevelopmentCertificateThumbprint)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (git -C $root status --porcelain) { throw 'Commit source changes before building the deliverable.' }
$version = [regex]::Match([IO.File]::ReadAllText((Join-Path $root 'Cargo.toml')), 'version = "([^"]+)"').Groups[1].Value
$issVersion = [regex]::Match([IO.File]::ReadAllText((Join-Path $PSScriptRoot 'sortypix.iss')), 'AppVersion "([^"]+)"').Groups[1].Value
if ($version -ne $issVersion) { throw 'Installer version mismatch' }
$exe = Join-Path $root 'target/release/sortypix.exe'
$verified = Join-Path $root 'target/reports/verified-sha256.txt'
if (-not (Test-Path $verified) -or (Get-FileHash $exe).Hash -ne ([IO.File]::ReadAllText($verified)).Trim()) { throw 'Run check.ps1 on this release before packaging.' }
$shell = Join-Path $root 'target/release/sortypix_shell.dll'
$shellProof = Join-Path $root 'target/reports/verified-shell-sha256.txt'
if (-not (Test-Path $shellProof) -or (Get-FileHash $shell).Hash -ne ([IO.File]::ReadAllText($shellProof)).Trim()) { throw 'Run check.ps1 on the shell extension before packaging.' }
$identity = Join-Path $PSScriptRoot "out/Sortypix-Explorer-$version.msix"
$signature = Get-AuthenticodeSignature -LiteralPath $identity
if (-not $signature.SignerCertificate) { throw 'Sign the Explorer identity package before building the installer.' }
if ($signature.Status -ne 'Valid') {
    if (-not $DevelopmentCertificateThumbprint -or $signature.SignerCertificate.Thumbprint -cne $DevelopmentCertificateThumbprint -or $signature.Status -notin @('NotTrusted','UnknownError')) {
        throw 'A trusted package signature is required. Local test installers need an explicit development certificate thumbprint.'
    }
    Write-Output 'Local development installer: its certificate is not trusted automatically.'
}
$compiler = Join-Path $env:LOCALAPPDATA 'Programs/Inno Setup 6/ISCC.exe'
if (-not (Test-Path $compiler)) { $compiler = (Get-Command ISCC.exe -ErrorAction Stop).Source }
& $compiler /Q (Join-Path $PSScriptRoot 'sortypix.iss')
if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
$setup = Join-Path $PSScriptRoot "out/Sortypix-Setup-$version.exe"
$hash = (Get-FileHash $setup).Hash
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'out/SHA256.txt'), "$hash  Sortypix-Setup-$version.exe`n", (New-Object Text.UTF8Encoding($false)))
Write-Output "Installer: $setup"
Write-Output "SHA256: $hash"
