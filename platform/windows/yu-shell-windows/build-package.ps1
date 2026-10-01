param(
    [ValidateSet('Development','Store','GitHub')][string]$Channel = 'Development',
    [ValidateSet('Release','Debug')][string]$Profile = 'Release',
    [string]$IdentityName,
    [string]$Publisher,
    [string]$PublisherDisplayName = 'Yu contributors',
    [string]$Version,
    [string]$CertificateThumbprint,
    [uri]$TimestampUrl,
    [switch]$Smoke,
    [switch]$TestPipeline,
    [switch]$Candidate,
    [string]$OutputDirectory = (Join-Path $PSScriptRoot ('../../../artifacts/windows-group7/' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0,8)))
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'release-utils.ps1')
$root = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$output = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $output) { throw "Output already exists; choose a new directory: $output" }
if ($Channel -eq 'GitHub') {
    if ($Profile -ne 'Release' -and -not $Candidate) { throw 'GitHub release builds require Release profile unless Candidate is explicit.' }
    if ($CertificateThumbprint -or $TimestampUrl -or $IdentityName -or $Publisher) { throw 'GitHub portable builds use unsigned EXEs and do not require package identity or signing inputs.' }
} elseif ($Channel -eq 'Store') {
    if (-not $IdentityName -or -not $Publisher) { throw 'Store builds require the exact IdentityName and Publisher assigned by Partner Center.' }
    if ($IdentityName -eq 'Yu.Editor.Development' -or $Publisher -eq 'CN=Yu Development') { throw 'Development identity cannot be used for Store builds.' }
    if ($Profile -ne 'Release') { throw 'Store builds require Release profile.' }
} else {
    if (-not $IdentityName) { $IdentityName = 'Yu.Editor.Development' }
    if (-not $Publisher) { $Publisher = 'CN=Yu Development' }
}
if ($Channel -ne 'GitHub') {
    if ($IdentityName -notmatch '^[A-Za-z0-9.-]{3,50}$') { throw 'Invalid package IdentityName.' }
    if (-not $Publisher.StartsWith('CN=')) { throw 'Publisher must be a certificate subject beginning with CN=.' }
}
if ($TimestampUrl -and ($TimestampUrl.Scheme -ne 'https' -or $TimestampUrl.UserInfo)) { throw 'TimestampUrl must use HTTPS without embedded credentials.' }
if ($CertificateThumbprint -and $CertificateThumbprint -notmatch '^[A-Fa-f0-9]{40}$') { throw 'CertificateThumbprint must be a SHA1 certificate-store thumbprint.' }
Push-Location $root
try {
    $gitStatus = (git status --porcelain --untracked-files=normal | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect source checkout.' }
    if ($Channel -eq 'Store' -and $gitStatus) { throw 'Store builds require a clean checkout.' }
    if ($Channel -eq 'GitHub' -and $gitStatus -and -not $Candidate) { throw 'GitHub release builds require a clean checkout unless Candidate is explicit.' }
    $commit = (git rev-parse HEAD).Trim()
    $metadata = Read-CargoMetadata $root
    $shellVersion = ($metadata.packages | Where-Object name -EQ 'yu-shell-windows').version
    if (-not $Version) { $Version = $shellVersion + '.0' }
    if ($Version -notmatch '^[1-9]\d*\.\d+\.\d+\.0$' -and $Version -notmatch '^0\.[1-9]\d*\.\d+\.0$' -and $Version -notmatch '^0\.0\.[1-9]\d*\.0$') { throw 'Package Version must be nonzero major.minor.patch.0.' }
    if ($Version.Substring(0,$Version.Length-2) -ne $shellVersion) { throw 'Package version must match the Cargo shell version with revision zero.' }
    foreach ($part in $Version.Split('.')) { if ([int]$part -gt 65535) { throw 'Package version component exceeds 65535.' } }
    $certificate = $null
    if ($CertificateThumbprint) {
        $certificatePath = 'Cert:\CurrentUser\My\' + $CertificateThumbprint
        if (-not (Test-Path -LiteralPath $certificatePath)) { throw 'Signing certificate was not found in CurrentUser\My.' }
        $certificate = Get-Item -LiteralPath $certificatePath
        if (-not $certificate.HasPrivateKey -or $certificate.Subject -cne $Publisher -or $certificate.NotAfter -le (Get-Date) -or $certificate.NotBefore -gt (Get-Date)) { throw 'Signing certificate must be valid, hold a private key, and exactly match Publisher.' }
        if ($certificate.EnhancedKeyUsageList.ObjectId -notcontains '1.3.6.1.5.5.7.3.3') { throw 'Signing certificate must have Code Signing EKU.' }
        if ($Channel -eq 'Store' -and -not $TimestampUrl) { throw 'TimestampUrl is required for signed Store candidates.' }
    }
    [IO.Directory]::CreateDirectory($output) | Out-Null
    $payload = Join-Path $output 'payload'
    [IO.Directory]::CreateDirectory($payload) | Out-Null
    $buildArguments = @('build','--locked','--target','x86_64-pc-windows-msvc','-p','yu-shell-windows','-p','yu-document-renderer')
    if ($Profile -eq 'Release') { $buildArguments += '--release' }
    Invoke-ReleaseTool 'cargo' $buildArguments (Join-Path $output 'build.log')
    $target = Join-Path $root ('target/x86_64-pc-windows-msvc/' + $Profile.ToLowerInvariant())
    foreach ($name in @('yu-shell-windows.exe','yu-document-renderer.exe')) { Copy-Item -LiteralPath (Join-Path $target $name) -Destination $payload }
    if ($Channel -eq 'GitHub') { Rename-Item -LiteralPath (Join-Path $payload 'yu-shell-windows.exe') -NewName 'Yu.exe' }
    if ($certificate) {
        $binarySignArguments = @('sign','/fd','SHA256','/sha1',$certificate.Thumbprint)
        if ($TimestampUrl) { $binarySignArguments += @('/tr',$TimestampUrl.AbsoluteUri,'/td','SHA256') }
        foreach ($name in @('yu-shell-windows.exe','yu-document-renderer.exe')) {
            Invoke-ReleaseTool (Find-SdkTool 'signtool') ($binarySignArguments + @((Join-Path $payload $name))) (Join-Path $output ($name + '-sign.log'))
            Invoke-ReleaseTool (Find-SdkTool 'signtool') @('verify','/pa','/all','/v',(Join-Path $payload $name)) (Join-Path $output ($name + '-signature.log'))
        }
    }
    Export-CargoLicenses $metadata (Join-Path $payload 'Licenses') $root
    if ($Channel -eq 'GitHub') {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Portable/README.txt') -Destination $payload
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $package = Join-Path $output "Yu-$shellVersion-windows-x64.zip"
        [IO.Compression.ZipFile]::CreateFromDirectory($payload, $package, [IO.Compression.CompressionLevel]::Optimal, $false)
    } else {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'AppBundle/Assets') -Destination $payload -Recurse
        [xml]$manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'AppBundle/AppxManifest.xml') -Encoding UTF8 -Raw
        $manifest.Package.Identity.SetAttribute('Name', $IdentityName)
        $manifest.Package.Identity.SetAttribute('Publisher', $Publisher)
        $manifest.Package.Identity.SetAttribute('Version', $Version)
        $manifest.Package.Properties.PublisherDisplayName = $PublisherDisplayName
        $manifest.Save((Join-Path $payload 'AppxManifest.xml'))
        $makepri = Find-SdkTool 'makepri'
        $config = Join-Path $output 'priconfig.xml'
        Invoke-ReleaseTool $makepri @('createconfig','/cf',$config,'/dq','en-US','/o') (Join-Path $output 'pri-config.log')
        Invoke-ReleaseTool $makepri @('new','/pr',$payload,'/cf',$config,'/of',(Join-Path $payload 'resources.pri'),'/o') (Join-Path $output 'pri.log')
        $package = Join-Path $output "Yu-$Version-x64.msix"
        Invoke-ReleaseTool (Find-SdkTool 'makeappx') @('pack','/d',$payload,'/p',$package,'/h','SHA256') (Join-Path $output 'makeappx.log')
        if ($certificate) {
            $signArguments = @('sign','/fd','SHA256','/sha1',$certificate.Thumbprint)
            if ($TimestampUrl) { $signArguments += @('/tr',$TimestampUrl.AbsoluteUri,'/td','SHA256') }
            Invoke-ReleaseTool (Find-SdkTool 'signtool') ($signArguments + @($package)) (Join-Path $output 'sign.log')
            Invoke-ReleaseTool (Find-SdkTool 'signtool') @('verify','/pa','/all','/v',$package) (Join-Path $output 'signature-verification.log')
        }
    }
    $inventory = Get-PayloadInventory $payload
    $audit = @{ schema_version=1; channel=$Channel; profile=$Profile; candidate=($Candidate -or [bool]$gitStatus); source_commit=$commit; git_status=$gitStatus; version=$Version; identity_name=$IdentityName; publisher=$Publisher; architecture='x64'; signed=[bool]$certificate; certificate_thumbprint=$CertificateThumbprint; package=[IO.Path]::GetFileName($package); package_sha256=(Get-FileHash $package).Hash; files=$inventory; stage='packed' }
    $audit.format = if ($Channel -eq 'GitHub') { 'portable-zip' } else { 'msix' }
    if ($Channel -eq 'GitHub') {
        [IO.File]::WriteAllText((Join-Path $output 'SHA256SUMS.txt'), ($audit.package_sha256.ToLowerInvariant() + '  ' + $audit.package + "`n"), (New-Object Text.UTF8Encoding($false)))
    }
    $audit.created_at = [DateTime]::UtcNow.ToString('o')
    $audit.cargo_lock_sha256 = (Get-FileHash -LiteralPath (Join-Path $root 'Cargo.lock')).Hash
    $audit.pipeline_sha256 = @{}
    foreach ($name in @('build-package.ps1','verify-package.ps1','release-utils.ps1','build.rs','AppBundle/AppxManifest.xml')) { $audit.pipeline_sha256[$name] = (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot $name)).Hash }
    Write-ReleaseJson $audit (Join-Path $output 'release-manifest.json')
    & (Join-Path $PSScriptRoot 'verify-package.ps1') -ReleaseDirectory $output -Smoke:$Smoke
    if ($TestPipeline) { & (Join-Path $PSScriptRoot 'test-package.ps1') -ReleaseDirectory $output }
    Write-Host "Windows $Channel package: $package"
}
finally { Pop-Location }
