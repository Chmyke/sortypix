$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $metadata = cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect dependency licenses' }
    $packages = @{}
    $nodes = @{}
    foreach ($package in $metadata.packages) { $packages[$package.id] = $package }
    foreach ($node in $metadata.resolve.nodes) { $nodes[$node.id] = $node }
    $entry = $metadata.packages | Where-Object { $_.name -eq 'sortypix-ui' }
    $queue = New-Object 'System.Collections.Generic.Queue[string]'
    $queue.Enqueue($entry.id)
    $visited = New-Object 'System.Collections.Generic.HashSet[string]'
    while ($queue.Count -gt 0) {
        $id = $queue.Dequeue()
        if (-not $visited.Add($id)) { continue }
        foreach ($dep in $nodes[$id].deps) {
            if (@($dep.dep_kinds | Where-Object { $_.kind -ne 'dev' }).Count -gt 0) { $queue.Enqueue($dep.pkg) }
        }
    }
    $text = New-Object Text.StringBuilder
    [void]$text.AppendLine('Sortypix - third-party notices (Windows runtime and build dependencies)')
    foreach ($id in ($visited | Sort-Object)) {
        $p = $packages[$id]
        if (-not $p.source) { continue }
        $chosen = $p.license
        if ($chosen -eq 'Apache-2.0 OR GPL-2.0-only') { $chosen = 'Apache-2.0 (permissive option selected)' }
        if (-not $chosen -or $chosen -match 'GPL') { throw "Review incompatible license: $($p.name) $($p.license)" }
        [void]$text.AppendLine("`n=== $($p.name) $($p.version) - $chosen ===")
        [void]$text.AppendLine($p.repository)
        $directory = Split-Path -Parent $p.manifest_path
        $licenses = Get-ChildItem -LiteralPath $directory -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)' }
        foreach ($license in $licenses) { [void]$text.AppendLine([IO.File]::ReadAllText($license.FullName)) }
    }
    [IO.File]::WriteAllText((Join-Path $root 'packaging/THIRD-PARTY-NOTICES.txt'), $text.ToString(), (New-Object Text.UTF8Encoding($false)))
    Write-Output "Dependency licenses checked: $($visited.Count) packages"
} finally { Pop-Location }
