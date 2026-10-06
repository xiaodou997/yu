param([Parameter(Mandatory=$true)][string]$ReleaseDirectory, [switch]$Smoke)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'release-utils.ps1')
$release = (Resolve-Path -LiteralPath $ReleaseDirectory).Path
$audit = Get-Content -LiteralPath (Join-Path $release 'installer-manifest.json') -Encoding UTF8 -Raw | ConvertFrom-Json
if ($audit.signed -or $audit.architecture -ne 'x64' -or [IO.Path]::GetFileName($audit.installer) -cne $audit.installer) { throw 'Expected unsigned x64 installer audit.' }
$installer = Join-Path $release $audit.installer
if ((Get-FileHash -LiteralPath $installer).Hash -ne $audit.installer_sha256) { throw 'Installer hash differs from audit.' }
if ((Get-AuthenticodeSignature -LiteralPath $installer).Status -ne 'NotSigned') { throw 'Installer must be unsigned for this channel.' }
$checksum = $audit.installer_sha256.ToLowerInvariant() + '  ' + $audit.installer
if (@([IO.File]::ReadAllLines((Join-Path $release 'SHA256SUMS.txt')) | Where-Object { $_ -ceq $checksum }).Count -ne 1) { throw 'Installer checksum differs from audit.' }
$pe = Get-PeIdentity $installer
if ($pe.subsystem -ne 2) { throw 'Installer is not a GUI executable.' }
$result = @{status='PASS'; signature='UNSIGNED'; installer_sha256=$audit.installer_sha256; installation='NOT_RUN'; reinstall='NOT_RUN'; uninstall='NOT_RUN'}
function Invoke-InstallerProcess([string]$Executable, [string]$Arguments) {
    $process = Start-Process -FilePath $Executable -ArgumentList $Arguments -WindowStyle Hidden -PassThru
    try {
        if (-not $process.WaitForExit(60000)) { throw 'Installer process timed out.' }
        if ($process.ExitCode -ne 0) { throw "Installer process failed: $($process.ExitCode)" }
    } finally { $process.Dispose() }
}
function Assert-InstalledPayload([string]$Directory) {
    foreach ($entry in $audit.files.PSObject.Properties) {
        if ($entry.Name -match '(^|/)\.\.(/|$)|:' -or [IO.Path]::IsPathRooted($entry.Name)) { throw 'Invalid installer inventory path.' }
        $file = Join-Path $Directory $entry.Name
        if (-not (Test-Path -LiteralPath $file -PathType Leaf) -or (Get-Item -LiteralPath $file).Length -ne $entry.Value.bytes -or (Get-FileHash -LiteralPath $file).Hash -ne $entry.Value.sha256) { throw "Installed payload mismatch: $($entry.Name)" }
    }
}
if ($Smoke) {
    $registry = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Yu.Editor.GitHub_is1'
    if (Test-Path -LiteralPath $registry) { throw 'Existing Yu installation found; isolated smoke must not replace it.' }
    $id = [Guid]::NewGuid().ToString('N')
    $test = Join-Path $release ('installer-test-' + $id)
    $app = Join-Path $test 'app'
    $group = 'Yu-acceptance-' + $id
    $shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) ($group + '\Yu.lnk')
    [void][IO.Directory]::CreateDirectory($test)
    $arguments = '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /DIR="' + $app + '" /GROUP="' + $group + '"'
    $userFile = Join-Path $app 'user-document.md'
    $externalFile = Join-Path $test 'external-document.md'
    $uninstaller = Join-Path $app 'unins000.exe'
    try {
        Invoke-InstallerProcess $installer ($arguments + ' /LOG="' + $test + '\install.log"')
        Assert-InstalledPayload $app
        $registration = Get-ItemProperty -LiteralPath $registry
        if ($registration.DisplayVersion -ne $audit.version -or $registration.UninstallString -notlike ('*' + $app + '*')) { throw 'Current-user uninstall registration differs from installation.' }
        if (-not (Test-Path -LiteralPath $shortcut)) { throw 'Start menu shortcut missing.' }
        $shell = New-Object -ComObject WScript.Shell
        try { if ($shell.CreateShortcut($shortcut).TargetPath -ne (Join-Path $app 'Yu.exe')) { throw 'Start menu shortcut has wrong target.' } }
        finally { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shell) }
        Invoke-InstallerProcess (Join-Path $app 'Yu.exe') '--window-self-check'
        Invoke-ReleaseTool 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-Mta','-File',(Join-Path $PSScriptRoot 'verify-accessibility.ps1'),'-Executable',(Join-Path $app 'Yu.exe'),'-OutputDirectory',(Join-Path $test 'uia-client')) (Join-Path $test 'uia.log')
        $uia = Get-Content -LiteralPath (Join-Path $test 'uia-client/results.json') -Encoding UTF8 -Raw | ConvertFrom-Json
        # The UIA suite grows with native controls. Verify its successful result
        # and the installed executable identity instead of an obsolete case count.
        $installedHash = (Get-FileHash -LiteralPath (Join-Path $app 'Yu.exe')).Hash
        if ($uia.status -ne 'PASS' -or @($uia.cases).Count -eq 0 -or $uia.sha256 -ne $installedHash) { throw 'Installed UIA smoke failed.' }
        $result.uia_cases = $uia.cases.Count
        $result.native_dpi = $uia.native_dpi
        $result.renderer = @(Test-ReleaseRenderer $app)
        $result.installation = 'PASS: per-user registration, shortcut, payload, HWND, UIA, math/Mermaid'
        [IO.File]::WriteAllText($userFile, '# User document preserved across reinstall and uninstall')
        [IO.File]::WriteAllText($externalFile, '# External user document preserved')
        $userHash = (Get-FileHash -LiteralPath $userFile).Hash
        $externalHash = (Get-FileHash -LiteralPath $externalFile).Hash
        Invoke-InstallerProcess $installer ($arguments + ' /LOG="' + $test + '\reinstall.log"')
        Assert-InstalledPayload $app
        if ((Get-FileHash -LiteralPath $userFile).Hash -ne $userHash -or (Get-FileHash -LiteralPath $externalFile).Hash -ne $externalHash) { throw 'Reinstallation changed user documents.' }
        $result.reinstall = 'PASS: same-version reinstall preserves user documents'
        Invoke-InstallerProcess $uninstaller ('/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG="' + $test + '\uninstall.log"')
        $deadline = [DateTime]::UtcNow.AddSeconds(20)
        while (((Test-Path -LiteralPath $registry) -or (Test-Path -LiteralPath $uninstaller)) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 200 }
        $removed = @($registry,(Split-Path $shortcut),$uninstaller)
        $removed += @($audit.files.PSObject.Properties | ForEach-Object { Join-Path $app $_.Name })
        foreach ($path in $removed) {
            if (Test-Path -LiteralPath $path) { throw "Uninstaller left installed entry: $path" }
        }
        if ((Get-FileHash -LiteralPath $userFile).Hash -ne $userHash -or (Get-FileHash -LiteralPath $externalFile).Hash -ne $externalHash) { throw 'Uninstallation changed user documents.' }
        $result.uninstall = 'PASS: binaries, shortcut and registration removed; user documents preserved'
        $result.evidence_directory = $test
    } finally {
        if (Test-Path -LiteralPath $registry) {
            $registered = Get-ItemProperty -LiteralPath $registry
            if ($registered.UninstallString -like ('*' + $app + '*') -and (Test-Path -LiteralPath $uninstaller)) {
                Invoke-InstallerProcess $uninstaller '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART'
            }
        }
    }
}
Write-ReleaseJson $result (Join-Path $release 'installer-verification.json')
Write-Host "Installer verification PASS; signature $($result.signature); installation $($result.installation)"
