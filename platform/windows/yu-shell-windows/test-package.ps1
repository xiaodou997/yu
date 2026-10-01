param([Parameter(Mandatory=$true)][string]$ReleaseDirectory)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'release-utils.ps1')
$release = (Resolve-Path -LiteralPath $ReleaseDirectory).Path
$auditPath = Join-Path $release 'release-manifest.json'
$audit = Get-Content -LiteralPath $auditPath -Encoding UTF8 -Raw | ConvertFrom-Json
$testRoot = Join-Path $release ('pipeline-tests-' + [Guid]::NewGuid().ToString('N').Substring(0,8))
[IO.Directory]::CreateDirectory($testRoot) | Out-Null
$cases = New-Object 'Collections.Generic.List[string]'
function Expect-Refusal([string]$Name, [string[]]$Arguments, [string]$Expected) {
    $log = Join-Path $testRoot ($Name + '.log')
    $refused = $false
    try { Invoke-ReleaseTool 'powershell.exe' (@('-NoProfile','-ExecutionPolicy','Bypass') + $Arguments) $log }
    catch { $refused = $true }
    if (-not $refused -or -not ([IO.File]::ReadAllText($log).Contains($Expected))) { throw "Expected refusal failed: $Name (see $log)" }
    $cases.Add($Name); Write-Host "PASS refusal: $Name"
}
$builder = Join-Path $PSScriptRoot 'build-package.ps1'
$verifier = Join-Path $PSScriptRoot 'verify-package.ps1'
Expect-Refusal 'missing-store-identity' @('-File',$builder,'-Channel','Store','-OutputDirectory',(Join-Path $testRoot 'missing-identity')) 'require the exact IdentityName and Publisher'
Expect-Refusal 'development-store-identity' @('-File',$builder,'-Channel','Store','-IdentityName','Yu.Editor.Development','-Publisher','CN=Yu Development','-OutputDirectory',(Join-Path $testRoot 'wrong-identity')) 'Development identity cannot be used'
Expect-Refusal 'debug-store-build' @('-File',$builder,'-Channel','Store','-Profile','Debug','-IdentityName','Yu.Test.Refusal','-Publisher','CN=Refusal','-OutputDirectory',(Join-Path $testRoot 'debug-store')) 'require Release profile'
Expect-Refusal 'unsafe-timestamp-scheme' @('-File',$builder,'-TimestampUrl','http://example.invalid/timestamp','-OutputDirectory',(Join-Path $testRoot 'timestamp')) 'TimestampUrl must use HTTPS'
Expect-Refusal 'package-version-mismatch' @('-File',$builder,'-Version','99.99.99.0','-OutputDirectory',(Join-Path $testRoot 'version')) 'must match the Cargo shell version'
Expect-Refusal 'existing-output' @('-File',$builder,'-OutputDirectory',$release) 'Output already exists'
Expect-Refusal 'missing-signing-certificate' @('-File',$builder,'-CertificateThumbprint',('0' * 40),'-OutputDirectory',(Join-Path $testRoot 'certificate')) 'Signing certificate was not found'
$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$marker = Join-Path $repositoryRoot ('.yu-package-test-' + [Guid]::NewGuid().ToString('N') + '.tmp')
try {
    [IO.File]::WriteAllText($marker, 'Temporary untracked fixture for dirty-checkout refusal.')
    Expect-Refusal 'dirty-store-checkout' @('-File',$builder,'-Channel','Store','-IdentityName','Yu.Test.Refusal','-Publisher','CN=Refusal','-OutputDirectory',(Join-Path $testRoot 'dirty-store')) 'require a clean checkout'
}
finally { if (Test-Path -LiteralPath $marker) { Remove-Item -LiteralPath $marker } }
function Copy-TestRelease([string]$Name) {
    $directory = Join-Path $testRoot $Name
    [IO.Directory]::CreateDirectory($directory) | Out-Null
    Copy-Item -LiteralPath $auditPath -Destination $directory
    Copy-Item -LiteralPath (Join-Path $release $audit.package) -Destination $directory
    return $directory
}
$tampered = Copy-TestRelease 'tampered'
$file = [IO.File]::Open((Join-Path $tampered $audit.package), [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite)
try { $value = $file.ReadByte(); $file.Position=0; $file.WriteByte([byte]($value -bxor 1)) } finally { $file.Dispose() }
Expect-Refusal 'package-hash-drift' @('-File',$verifier,'-ReleaseDirectory',$tampered) 'Package changed since packing/signing'
$wrongHash = Copy-TestRelease 'payload-hash'
$changed = Get-Content -LiteralPath $auditPath -Encoding UTF8 -Raw | ConvertFrom-Json
$changed.files.'yu-document-renderer.exe'.sha256 = '0' * 64
Write-ReleaseJson $changed (Join-Path $wrongHash 'release-manifest.json')
Expect-Refusal 'payload-hash-drift' @('-File',$verifier,'-ReleaseDirectory',$wrongHash) 'Package payload changed: yu-document-renderer.exe'
$unexpected = Copy-TestRelease 'unexpected'
$changed = Get-Content -LiteralPath $auditPath -Encoding UTF8 -Raw | ConvertFrom-Json
$changed.files.PSObject.Properties.Remove('Assets/StoreLogo.png')
Write-ReleaseJson $changed (Join-Path $unexpected 'release-manifest.json')
Expect-Refusal 'unexpected-payload-entry' @('-File',$verifier,'-ReleaseDirectory',$unexpected) 'Unexpected package file: Assets/StoreLogo.png'
$mismatch = Copy-TestRelease 'identity'
$changed = Get-Content -LiteralPath $auditPath -Encoding UTF8 -Raw | ConvertFrom-Json
$changed.publisher = 'CN=RefusedIdentity'
Write-ReleaseJson $changed (Join-Path $mismatch 'release-manifest.json')
Expect-Refusal 'manifest-identity-mismatch' @('-File',$verifier,'-ReleaseDirectory',$mismatch) 'Manifest identity does not match release audit'
$spacePath = Copy-TestRelease ('space path ' + [char]0x4e2d + [char]0x6587)
Invoke-ReleaseTool 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',$verifier,'-ReleaseDirectory',$spacePath) (Join-Path $testRoot 'space-path.log')
$cases.Add('spaces-and-Unicode-release-path'); Write-Host 'PASS: spaces and Unicode release path'
Write-ReleaseJson @{status='PASS'; cases=$cases.ToArray(); source_package_sha256=$audit.package_sha256} (Join-Path $testRoot 'results.json')
Write-Host "Pipeline tests PASS: $($cases.Count)"
