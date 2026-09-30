$ErrorActionPreference = "Stop"

$root = Resolve-Path (Join-Path $PSScriptRoot "../../..")
Push-Location $root
try {
    Write-Host "==> Windows DirectWrite / D3D / shell tests"
    cargo test -p yu-font-windows -p yu-render-windows -p yu-shell-windows
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host "==> Windows shell native compile"
    cargo check -p yu-shell-windows
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host "==> Windows HWND + DirectWrite + D3D render smoke"
    cargo run -p yu-shell-windows -- --window-self-check
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
finally {
    Pop-Location
}
