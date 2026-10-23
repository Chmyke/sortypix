# Writes the third-party notices of one platform's binary and refuses an incompatible license.
# -Features ai lists the crates of Sortypix AI (sortypix-ui built with its ai feature).
param(
    [string]$Triple = 'x86_64-pc-windows-msvc',
    [string]$Output,
    [string]$Platform = 'Windows',
    [string]$Features
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $Output) { $Output = Join-Path $root 'packaging/THIRD-PARTY-NOTICES.txt' }
Push-Location $root
try {
    $metadataFeatures = @()
    $treeFeatures = @()
    $product = 'Sortypix'
    if ($Features) {
        $metadataFeatures = @('--features', "sortypix-ui/$Features")
        $treeFeatures = @('--features', $Features)
        if ($Features -eq 'ai') { $product = 'Sortypix AI' }
    }
    $metadata = cargo metadata --locked --format-version 1 --filter-platform $Triple @metadataFeatures | ConvertFrom-Json
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
    # The resolve graph also holds optional crates a feature only names (wgpu behind
    # eframe's Wayland feature); cargo tree lists what is really compiled.
    $built = New-Object 'System.Collections.Generic.HashSet[string]'
    foreach ($line in (cargo tree --locked -p sortypix-ui @treeFeatures --target $Triple -e normal,build --prefix none --format '{p}')) {
        if ($line -match '^(\S+) v(\S+)') { [void]$built.Add("$($Matches[1]) $($Matches[2])") }
    }
    if ($LASTEXITCODE -ne 0 -or $built.Count -eq 0) { throw 'Cannot list compiled dependencies' }
    $visited = @($visited | Where-Object { $built.Contains("$($packages[$_].name) $($packages[$_].version)") })
    $text = New-Object Text.StringBuilder
    [void]$text.AppendLine("$product - third-party notices ($Platform runtime and build dependencies)")
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
    [IO.File]::WriteAllText($Output, $text.ToString(), (New-Object Text.UTF8Encoding($false)))
    Write-Output "Dependency licenses checked: $($visited.Count) packages"
} finally { Pop-Location }
