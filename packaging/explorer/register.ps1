param([switch]$Unregister, [string]$LogPath)
$ErrorActionPreference = 'Stop'
$name = 'MickaelMancini.Sortypix.Explorer'
$log = if ($LogPath) { $LogPath } else { Join-Path $env:LOCALAPPDATA 'Sortypix/explorer-registration.txt' }
try {
    if ($Unregister) {
        Get-AppxPackage -Name $name | Where-Object { $_.Name -ceq $name } | Remove-AppxPackage -ErrorAction Stop
    } else {
        $packages = @(Get-ChildItem -LiteralPath $PSScriptRoot -Filter 'Sortypix-Explorer-*.msix' -File)
        if ($packages.Count -ne 1) { throw 'Expected exactly one Explorer identity package.' }
        $trusted = $false
        $trustStatus = 'No Authenticode signature'
        try {
            $rawCertificate = [Security.Cryptography.X509Certificates.X509Certificate]::CreateFromSignedFile($packages[0].FullName)
            $certificate = New-Object Security.Cryptography.X509Certificates.X509Certificate2($rawCertificate)
            $chain = New-Object Security.Cryptography.X509Certificates.X509Chain
            $chain.ChainPolicy.RevocationMode = [Security.Cryptography.X509Certificates.X509RevocationMode]::NoCheck
            $trusted = $chain.Build($certificate)
            $trustStatus = if ($trusted) { 'Valid' } else { ($chain.ChainStatus.Status -join ', ') }
        } catch {
            $trustStatus = $_.Exception.Message
        }
        if (-not $trusted) {
            [IO.Directory]::CreateDirectory((Split-Path -Parent $log)) | Out-Null
            [IO.File]::WriteAllText(
                $log,
                "SKIPPED: Modern Explorer menu requires a trusted publisher signature. Status: $trustStatus",
                (New-Object Text.UTF8Encoding($false))
            )
            exit 0
        }
        Add-AppxPackage -Path $packages[0].FullName -ExternalLocation $PSScriptRoot -ErrorAction Stop
        if (-not (Get-AppxPackage -Name $name | Where-Object { $_.Name -ceq $name })) { throw 'Explorer identity is not registered.' }
    }
    [IO.Directory]::CreateDirectory((Split-Path -Parent $log)) | Out-Null
    [IO.File]::WriteAllText($log, 'PASS', (New-Object Text.UTF8Encoding($false)))
} catch {
    [IO.Directory]::CreateDirectory((Split-Path -Parent $log)) | Out-Null
    [IO.File]::WriteAllText($log, $_.Exception.Message, (New-Object Text.UTF8Encoding($false)))
    Write-Error $_
    exit 1
}
