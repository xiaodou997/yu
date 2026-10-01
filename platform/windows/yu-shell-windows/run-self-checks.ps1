$ErrorActionPreference = "Stop"

$root = Resolve-Path (Join-Path $PSScriptRoot "../../..")
Push-Location $root
try {
    Write-Host "==> Windows companion math / Mermaid helper"
    cargo build -p yu-document-renderer
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Host "==> Windows DirectWrite / D3D / shell tests"
    cargo test -p yu-font-windows -p yu-render-windows -p yu-shell-windows
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host "==> Windows resources / actual D3D readback / recovery"
    cargo test -p yu-shell-windows native_group5_resources_present_recover_and_preserve_source -- --ignored --nocapture
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
