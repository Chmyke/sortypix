param([string]$CertificateThumbprint = $env:SORTYPIX_SIGN_THUMBPRINT)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$version = [regex]::Match([IO.File]::ReadAllText((Join-Path $root 'Cargo.toml')), 'version = "([^"]+)"').Groups[1].Value
$sdk = Get-ChildItem 'C:/Program Files (x86)/Windows Kits/10/bin' -Directory |
    Where-Object { Test-Path (Join-Path $_.FullName 'x64/makeappx.exe') } |
    Sort-Object Name -Descending | Select-Object -First 1
if (-not $sdk) { throw 'Windows SDK MakeAppx is required.' }
$stage = Join-Path $root ('target/explorer-package-' + $version)
$out = Join-Path $PSScriptRoot 'out'
[IO.Directory]::CreateDirectory($stage) | Out-Null
[IO.Directory]::CreateDirectory($out) | Out-Null
$manifest = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'explorer/AppxManifest.xml')).Replace('@VERSION@', $version)
[IO.File]::WriteAllText((Join-Path $stage 'AppxManifest.xml'), $manifest, (New-Object Text.UTF8Encoding($false)))
$package = Join-Path $out ('Sortypix-Explorer-' + $version + '.msix')
# Sparse identity references binaries installed by Inno; /nv permits external file paths.
& (Join-Path $sdk.FullName 'x64/makeappx.exe') pack /o /d $stage /nv /p $package
if ($LASTEXITCODE -ne 0) { throw 'Explorer identity packaging failed.' }
if ($CertificateThumbprint) {
    if ($CertificateThumbprint -notmatch '^[A-Fa-f0-9]{40}$') { throw 'Invalid signing thumbprint.' }
    $certificate = Get-Item -LiteralPath ('Cert:/CurrentUser/My/' + $CertificateThumbprint)
    if ($certificate.Subject -cne 'CN=Mickael Mancini' -or -not $certificate.HasPrivateKey) { throw 'Expected the publisher signing certificate and its private key.' }
    & (Join-Path $sdk.FullName 'x64/signtool.exe') sign /fd SHA256 /sha1 $CertificateThumbprint $package
    if ($LASTEXITCODE -ne 0) { throw 'Explorer identity signing failed.' }
    Write-Output "Signed Explorer package: $package"
} else {
    Write-Output "Unsigned Explorer package ready for signing: $package"
}
