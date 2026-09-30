$ErrorActionPreference = "Stop"

$root = Resolve-Path (Join-Path $PSScriptRoot "../../..")
Push-Location $root
try {
    Write-Host "==> Windows shell model tests"
    cargo test -p yu-shell-windows
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host "==> Windows shell native compile"
    cargo check -p yu-shell-windows
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host "==> Windows HWND launch smoke"
    cargo run -p yu-shell-windows -- --window-self-check
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
finally {
    Pop-Location
}
