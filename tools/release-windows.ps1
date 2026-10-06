param(
    [string]$OutputDirectory,
    [string]$CompilerPath = $env:YU_INNO_COMPILER
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$shell = Join-Path $root 'platform/windows/yu-shell-windows'
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $root ('artifacts/releases/windows-' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss'))
}
if (-not $CompilerPath) {
    $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($command) { $CompilerPath = $command.Source }
    foreach ($base in @($env:ProgramFiles, ${env:ProgramFiles(x86)})) {
        if (-not $base) { continue }
        foreach ($version in @('7', '6')) {
            $candidate = Join-Path $base "Inno Setup $version/ISCC.exe"
            if (-not $CompilerPath -and (Test-Path -LiteralPath $candidate)) { $CompilerPath = $candidate }
        }
    }
}
if (-not $CompilerPath -or -not (Test-Path -LiteralPath $CompilerPath -PathType Leaf)) {
    throw 'Install Inno Setup first, or provide -CompilerPath pointing to ISCC.exe.'
}
$CompilerPath = (Resolve-Path -LiteralPath $CompilerPath).Path
if (Test-Path -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Yu.Editor.GitHub_is1') {
    throw 'Installer verification requires a Windows account without an existing Yu installation. Use a clean account or the GitHub Actions workflow.'
}
Push-Location $root
try {
    $status = (git status --porcelain --untracked-files=normal | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $status) { throw 'Release requires a clean Git checkout.' }
    cargo fmt --all --check
    if ($LASTEXITCODE -ne 0) { throw 'Rust formatting failed.' }
    cargo clippy --workspace --all-targets --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Rust clippy failed.' }
    cargo test --workspace --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust tests failed.' }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $shell 'run-self-checks.ps1')
    if ($LASTEXITCODE -ne 0) { throw 'Windows native self-checks failed.' }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $shell 'build-package.ps1') -Channel GitHub -Profile Release -Smoke -TestPipeline -OutputDirectory $OutputDirectory
    if ($LASTEXITCODE -ne 0) { throw 'Portable release build or verification failed.' }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $shell 'build-installer.ps1') -ReleaseDirectory $OutputDirectory -CompilerPath $CompilerPath -Smoke
    if ($LASTEXITCODE -ne 0) { throw 'Installer build or verification failed.' }
    Write-Host "Release complete: $OutputDirectory"
    Get-ChildItem -LiteralPath $OutputDirectory -File | Where-Object { $_.Name -match '(setup\.exe|\.zip|SHA256SUMS\.txt)$' } | Select-Object Name, FullName
}
finally { Pop-Location }
