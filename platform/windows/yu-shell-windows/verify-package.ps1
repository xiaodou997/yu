param([Parameter(Mandatory=$true)][string]$ReleaseDirectory, [switch]$Smoke)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'release-utils.ps1')
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class YuPackageResources {
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] public static extern IntPtr LoadLibraryEx(string file, IntPtr handle, uint flags);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern IntPtr FindResource(IntPtr module, IntPtr name, IntPtr type);
    [DllImport("kernel32.dll")] public static extern bool FreeLibrary(IntPtr module);
}
'@
$release = (Resolve-Path -LiteralPath $ReleaseDirectory).Path
$audit = Get-Content -LiteralPath (Join-Path $release 'release-manifest.json') -Encoding UTF8 -Raw | ConvertFrom-Json
$package = Join-Path $release $audit.package
if ((Get-FileHash -LiteralPath $package).Hash -ne $audit.package_sha256) { throw 'Package changed since packing/signing.' }
$unpacked = Join-Path $release ('verified-' + [Guid]::NewGuid().ToString('N').Substring(0,8))
Invoke-ReleaseTool (Find-SdkTool 'makeappx') @('unpack','/p',$package,'/d',$unpacked) (Join-Path $release 'unpack.log')
$expected = @($audit.files.PSObject.Properties)
foreach ($entry in $expected) {
    if ($entry.Name -match '(^|/)\.\.(/|$)' -or [IO.Path]::IsPathRooted($entry.Name)) { throw 'Invalid inventory path.' }
    $file = Join-Path $unpacked $entry.Name
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Package file missing: $($entry.Name)" }
    if ((Get-Item -LiteralPath $file).Length -ne $entry.Value.bytes -or (Get-FileHash -LiteralPath $file).Hash -ne $entry.Value.sha256) { throw "Package payload changed: $($entry.Name)" }
}
$allowed = @($expected.Name) + @('AppxBlockMap.xml','[Content_Types].xml','AppxSignature.p7x','AppxMetadata/CodeIntegrity.cat')
$actual = Get-PayloadInventory $unpacked
foreach ($name in $actual.Keys) { if ($name -notin $allowed) { throw "Unexpected package file: $name" } }
[xml]$manifest = Get-Content -LiteralPath (Join-Path $unpacked 'AppxManifest.xml') -Encoding UTF8 -Raw
$identity = $manifest.Package.Identity
if ($identity.Name -cne $audit.identity_name -or $identity.Publisher -cne $audit.publisher -or $identity.Version -ne $audit.version -or $identity.ProcessorArchitecture -ne 'x64') { throw 'Manifest identity does not match release audit.' }
$application = $manifest.Package.Applications.Application
if ($application.Executable -ne 'yu-shell-windows.exe' -or $application.EntryPoint -ne 'windows.fullTrustApplication') { throw 'Wrong desktop entry point.' }
if (@($manifest.Package.Capabilities.ChildNodes).Count -ne 1 -or $manifest.Package.Capabilities.ChildNodes[0].Name -ne 'runFullTrust') { throw 'Unexpected package capabilities.' }
$languages = @($manifest.Package.Resources.Resource | ForEach-Object Language | Sort-Object)
if (($languages -join ',') -ne 'en-US,ja-JP,ko-KR,zh-Hans,zh-Hant') { throw 'Package language set differs from supported UI languages.' }
foreach ($name in @('yu-shell-windows.exe','yu-document-renderer.exe')) {
    $pe = Get-PeIdentity (Join-Path $unpacked $name)
    if ($pe.machine -ne 0x8664 -or $pe.optional_magic -ne 0x20b) { throw "Non-x64 PE32+ binary: $name" }
    if ($name -eq 'yu-shell-windows.exe' -and $pe.subsystem -ne 2) { throw 'Yu shell is not a GUI executable.' }
}
$executable = Join-Path $unpacked 'yu-shell-windows.exe'
$info = [Diagnostics.FileVersionInfo]::GetVersionInfo($executable)
if ($info.ProductName -ne 'Yu' -or ($info.ProductVersion + '.0') -ne $audit.version) { throw 'EXE version resource disagrees with MSIX version.' }
$module = [YuPackageResources]::LoadLibraryEx($executable, [IntPtr]::Zero, 2)
if ($module -eq [IntPtr]::Zero) { throw 'Cannot load EXE resources.' }
try {
    foreach ($type in @(14,16,24)) {
        if ([YuPackageResources]::FindResource($module, [IntPtr]1, [IntPtr]$type) -eq [IntPtr]::Zero) { throw "Missing EXE resource type $type (icon/version/manifest)." }
    }
}
finally { [void][YuPackageResources]::FreeLibrary($module) }
$exeManifest = Join-Path $release 'exe-manifest.xml'
Invoke-ReleaseTool (Find-SdkTool 'mt') @('-nologo',('-inputresource:' + $executable + ';#1'),('-out:' + $exeManifest)) (Join-Path $release 'exe-manifest.log')
[xml]$nativeManifest = Get-Content -LiteralPath $exeManifest -Encoding UTF8 -Raw
if ($nativeManifest.assembly.assemblyIdentity.version -ne $audit.version) { throw 'EXE assembly version disagrees with MSIX version.' }
if ($nativeManifest.SelectSingleNode("//*[local-name()='dpiAwareness']").InnerText -ne 'PerMonitorV2' -or $nativeManifest.SelectSingleNode("//*[local-name()='longPathAware']").InnerText -ne 'true' -or $nativeManifest.SelectSingleNode("//*[local-name()='requestedExecutionLevel']").GetAttribute('level') -ne 'asInvoker') { throw 'Wrong EXE DPI or execution-level manifest.' }
foreach ($row in @(@('StoreLogo.png',50),@('Square44x44Logo.png',44),@('Square150x150Logo.png',150))) {
    $image = [Drawing.Image]::FromFile((Join-Path $unpacked ('Assets/' + $row[0])))
    try { if ($image.Width -ne $row[1] -or $image.Height -ne $row[1]) { throw "Wrong asset size: $($row[0])" } }
    finally { $image.Dispose() }
}
if ($audit.signed) {
    Invoke-ReleaseTool (Find-SdkTool 'signtool') @('verify','/pa','/all','/v',$package) (Join-Path $release 'signature-verification.log')
    $signature = Get-AuthenticodeSignature -LiteralPath $package
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -cne $audit.publisher -or $signature.SignerCertificate.Thumbprint -ne $audit.certificate_thumbprint) { throw 'MSIX signature or signer mismatch.' }
    foreach ($name in @('yu-shell-windows.exe','yu-document-renderer.exe')) {
        $binarySignature = Get-AuthenticodeSignature -LiteralPath (Join-Path $unpacked $name)
        if ($binarySignature.Status -ne 'Valid' -or $binarySignature.SignerCertificate.Thumbprint -ne $audit.certificate_thumbprint) { throw "Binary signature mismatch: $name" }
    }
} elseif (Test-Path -LiteralPath (Join-Path $unpacked 'AppxSignature.p7x')) { throw 'Unsigned release unexpectedly contains a signature.' }
$result = @{ status='PASS'; payload_files=$expected.Count; package_sha256=$audit.package_sha256; version=$audit.version; architecture='x64'; signature=$(if ($audit.signed) { 'VALID' } else { 'UNSIGNED' }); installation='NOT_RUN'; smoke='NOT_RUN'; extracted_directory=$unpacked }
if ($Smoke) {
    $process = Start-Process -FilePath $executable -ArgumentList '--window-self-check' -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(30000)) { Stop-Process -Id $process.Id; throw 'Packaged shell HWND smoke timed out.' }
    if ($process.ExitCode -ne 0) { throw "Packaged shell HWND smoke failed: $($process.ExitCode)" }
    Invoke-ReleaseTool 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-Mta','-File',(Join-Path $PSScriptRoot 'verify-accessibility.ps1'),'-Executable',$executable,'-OutputDirectory',(Join-Path $release 'uia-client')) (Join-Path $release 'uia.log')
    $helperInfo = New-Object Diagnostics.ProcessStartInfo
    $helperInfo.FileName = Join-Path $unpacked 'yu-document-renderer.exe'
    $helperInfo.UseShellExecute = $false; $helperInfo.CreateNoWindow = $true
    $helperInfo.RedirectStandardInput = $true; $helperInfo.RedirectStandardOutput = $true
    $helperInfo.StandardOutputEncoding = New-Object Text.UTF8Encoding($false)
    $helper = [Diagnostics.Process]::Start($helperInfo)
    $rendered = @()
    try {
        foreach ($request in @(@{id=1;document=7;revision=1;kind='math';source='e^{i\pi}+1=0'},@{id=2;document=7;revision=1;kind='mermaid';source="flowchart LR`nA[Image] --> B[Math]"})) {
            $helper.StandardInput.WriteLine(($request | ConvertTo-Json -Compress))
            $helper.StandardInput.Flush()
            $responseTask = $helper.StandardOutput.ReadLineAsync()
            if (-not $responseTask.Wait(30000)) { throw 'Packaged renderer response timed out.' }
            $response = $responseTask.Result | ConvertFrom-Json
            if ($response.id -ne $request.id -or $response.document -ne 7 -or $response.revision -ne 1 -or $response.status -ne 'ready' -or $response.vector.width -le 0 -or $response.vector.height -le 0 -or -not $response.vector.svg.Contains('<svg')) { throw "Packaged renderer failed: $($request.kind)" }
            $rendered += @{kind=$request.kind; width=$response.vector.width; height=$response.vector.height; svg_bytes=[Text.Encoding]::UTF8.GetByteCount($response.vector.svg)}
        }
        $helper.StandardInput.Close()
        if (-not $helper.WaitForExit(10000) -or $helper.ExitCode -ne 0) { throw 'Packaged renderer did not shut down successfully.' }
    }
    finally { if (-not $helper.HasExited) { $helper.Kill(); $helper.WaitForExit() }; $helper.Dispose() }
    $result.renderer = $rendered
    $result.smoke = 'PASS: extracted HWND + external UIA client + math/Mermaid helper'
}
Write-ReleaseJson $result (Join-Path $release 'verification.json')
Write-Host "Package verification PASS: $($expected.Count) payload files; signature $($result.signature)"
