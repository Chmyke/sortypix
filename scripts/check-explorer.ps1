$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$run = Join-Path $root ('target/shell-check-' + [Guid]::NewGuid().ToString('N'))
$folder = Join-Path $run ('photos espace ' + [char]0xE9 + ' & (test)')
$receipt = Join-Path $run 'received-folder.txt'
[IO.Directory]::CreateDirectory($folder) | Out-Null
Copy-Item -LiteralPath (Join-Path $root 'target/release/sortypix_shell.dll') -Destination (Join-Path $run 'sortypix_shell.dll')
Copy-Item -LiteralPath (Join-Path $root 'target/release/examples/launch_probe.exe') -Destination (Join-Path $run 'sortypix.exe')
$previous = $env:SORTYPIX_SHELL_PROBE_OUTPUT
try {
    $env:SORTYPIX_SHELL_PROBE_OUTPUT = $receipt
    & (Join-Path $root 'target/release/examples/shell_probe.exe') (Join-Path $run 'sortypix_shell.dll') $folder $receipt
    if ($LASTEXITCODE -ne 0) { throw 'Native shell extension probe failed.' }
} finally { $env:SORTYPIX_SHELL_PROBE_OUTPUT = $previous }
$version = [regex]::Match([IO.File]::ReadAllText((Join-Path $root 'Cargo.toml')), 'version = "([^"]+)"').Groups[1].Value
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead((Join-Path $root "packaging/out/Sortypix-Explorer-$version.msix"))
try {
    $reader = New-Object IO.StreamReader($zip.GetEntry('AppxManifest.xml').Open())
    try { [xml]$manifest = $reader.ReadToEnd() } finally { $reader.Dispose() }
    $class = $manifest.SelectSingleNode("//*[local-name()='Class']")
    $verb = $manifest.SelectSingleNode("//*[local-name()='Verb']")
    $type = $manifest.SelectSingleNode("//*[local-name()='ItemType']")
    if ($class.Id -cne '46DE7F78-C29E-4A6C-A1D7-AA3A6C960281' -or $verb.Clsid -cne $class.Id -or $type.Type -cne 'Directory') { throw 'Explorer manifest class or folder association mismatch.' }
    if ($class.Path -cne "sortypix_shell-$version.dll" -or $manifest.Package.Identity.Version -cne "$version.0") { throw 'Explorer package version mismatch.' }
} finally { $zip.Dispose() }
$untrusted = Join-Path $run 'untrusted-installer'
[IO.Directory]::CreateDirectory($untrusted) | Out-Null
Copy-Item -LiteralPath (Join-Path $root 'packaging/explorer/register.ps1') -Destination (Join-Path $untrusted 'register-explorer.ps1')
[IO.File]::WriteAllBytes((Join-Path $untrusted 'Sortypix-Explorer-test.msix'), [byte[]](1, 2, 3, 4))
$registrationLog = Join-Path $untrusted 'explorer-registration.txt'
& powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File (Join-Path $untrusted 'register-explorer.ps1') -LogPath $registrationLog
if ($LASTEXITCODE -ne 0) { throw 'An untrusted Explorer package must not fail the application install.' }
if ((Get-Content -LiteralPath $registrationLog -Raw) -notmatch '^SKIPPED:') {
    throw 'An untrusted Explorer package must leave an explicit skipped registration receipt.'
}
Write-Output "Sparse package manifest and native extension: PASS ($run)"
