param(
    [Parameter(Mandatory=$true)][string]$ReleaseDirectory,
    [string]$CompilerPath = $env:YU_INNO_COMPILER,
    [switch]$Smoke
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'release-utils.ps1')
$release = (Resolve-Path -LiteralPath $ReleaseDirectory).Path
$audit = Get-Content -LiteralPath (Join-Path $release 'release-manifest.json') -Encoding UTF8 -Raw | ConvertFrom-Json
if ($audit.channel -ne 'GitHub' -or $audit.format -ne 'portable-zip' -or $audit.signed) { throw 'Installer input must be a GitHub unsigned portable release.' }
if (-not $CompilerPath) {
    $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($command) { $CompilerPath = $command.Source }
    foreach ($directory in @((Join-Path $env:ProgramFiles 'Inno Setup 7'),(Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6'))) {
        if (-not $CompilerPath -and (Test-Path -LiteralPath (Join-Path $directory 'ISCC.exe'))) { $CompilerPath = Join-Path $directory 'ISCC.exe' }
    }
}
if (-not $CompilerPath -or -not (Test-Path -LiteralPath $CompilerPath -PathType Leaf)) { throw 'Inno Setup compiler not found; provide CompilerPath or YU_INNO_COMPILER.' }
$compiler = (Resolve-Path -LiteralPath $CompilerPath).Path
$payload = Join-Path $release 'installer-payload'
if (Test-Path -LiteralPath $payload) { throw 'Installer output already exists; choose a fresh release directory.' }
Invoke-ReleaseTool 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $PSScriptRoot 'verify-package.ps1'),'-ReleaseDirectory',$release) (Join-Path $release 'installer-input-verification.log')
$verification = Get-Content -LiteralPath (Join-Path $release 'verification.json') -Encoding UTF8 -Raw | ConvertFrom-Json
Copy-Item -LiteralPath $verification.extracted_directory -Destination $payload -Recurse
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Installer/README.txt') -Destination (Join-Path $payload 'README.txt') -Force
Copy-Item -LiteralPath (Join-Path (Split-Path $compiler) 'license.txt') -Destination (Join-Path $payload 'Licenses/InnoSetup.txt')
$version = $audit.version.Substring(0,$audit.version.Length-2)
Invoke-ReleaseTool $compiler @('/Qp',('/DPayloadDirectory=' + $payload),('/DReleaseVersion=' + $version),('/DOutputDirectory=' + $release),(Join-Path $PSScriptRoot 'Installer/Yu.iss')) (Join-Path $release 'installer-build.log')
$installer = Join-Path $release "Yu-$version-windows-x64-setup.exe"
if (-not (Test-Path -LiteralPath $installer) -or (Get-AuthenticodeSignature -LiteralPath $installer).Status -ne 'NotSigned') { throw 'Expected unsigned EXE installer was not generated.' }
$record = @{schema_version=1; source_commit=$audit.source_commit; candidate=$audit.candidate; profile=$audit.profile; version=$version; architecture='x64'; signed=$false; installer=[IO.Path]::GetFileName($installer); installer_sha256=(Get-FileHash -LiteralPath $installer).Hash; portable_package_sha256=$audit.package_sha256; compiler_version=(Get-Item -LiteralPath $compiler).VersionInfo.ProductVersion; compiler_sha256=(Get-FileHash -LiteralPath $compiler).Hash; script_sha256=(Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'Installer/Yu.iss')).Hash; files=(Get-PayloadInventory $payload)}
Write-ReleaseJson $record (Join-Path $release 'installer-manifest.json')
[IO.File]::AppendAllText((Join-Path $release 'SHA256SUMS.txt'), ($record.installer_sha256.ToLowerInvariant() + '  ' + $record.installer + "`n"), (New-Object Text.UTF8Encoding($false)))
& (Join-Path $PSScriptRoot 'verify-installer.ps1') -ReleaseDirectory $release -Smoke:$Smoke
Write-Host "GitHub unsigned EXE installer: $installer"
