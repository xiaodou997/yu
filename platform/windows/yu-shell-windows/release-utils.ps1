$ErrorActionPreference = 'Stop'
function Read-CargoMetadata([string]$Root) {
    # Read UTF-8 directly; PowerShell 5's native pipeline uses the console code page.
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = (Get-Command cargo.exe).Source
    $info.Arguments = 'metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc'
    $info.WorkingDirectory = $Root
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.StandardOutputEncoding = New-Object Text.UTF8Encoding($false)
    $process = [Diagnostics.Process]::Start($info)
    try {
        $json = $process.StandardOutput.ReadToEnd()
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) { throw 'Locked Cargo metadata failed.' }
        return ($json | ConvertFrom-Json)
    }
    finally { $process.Dispose() }
}
function Find-SdkTool([string]$Name) {
    $sdk = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/bin'
    $versions = @(Get-ChildItem -LiteralPath $sdk -Directory | Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } | Sort-Object { [version]$_.Name } -Descending)
    foreach ($version in $versions) {
        $tool = Join-Path $version.FullName ('x64/' + $Name + '.exe')
        if (Test-Path -LiteralPath $tool -PathType Leaf) { return $tool }
    }
    throw "Windows SDK x64 tool not found: $Name"
}
function Invoke-ReleaseTool([string]$Tool, [string[]]$Arguments, [string]$Log) {
    # PowerShell 5 treats native stderr as ErrorRecord even on successful Cargo runs.
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { & $Tool @Arguments *> $Log; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $previous }
    if ($code -ne 0) { throw "$Tool failed ($code); see $Log" }
}
function Write-ReleaseJson($Value, [string]$Path) {
    [IO.File]::WriteAllText($Path, (($Value | ConvertTo-Json -Depth 30) + "`n"), (New-Object Text.UTF8Encoding($false)))
}
function Get-PayloadInventory([string]$Directory) {
    $rows = @{}
    foreach ($file in Get-ChildItem -LiteralPath $Directory -Recurse -File) {
        if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Payload reparse point: $($file.FullName)" }
        $relative = $file.FullName.Substring($Directory.Length + 1).Replace('\','/')
        $rows[$relative] = @{ bytes = $file.Length; sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash }
    }
    return $rows
}
function Get-PeIdentity([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    $reader = New-Object IO.BinaryReader($stream)
    try {
        if ($reader.ReadUInt16() -ne 0x5a4d) { throw "Missing MZ header: $Path" }
        $stream.Position = 0x3c; $offset = $reader.ReadUInt32(); $stream.Position = $offset
        if ($reader.ReadUInt32() -ne 0x4550) { throw "Missing PE signature: $Path" }
        $machine = $reader.ReadUInt16(); $stream.Position = $offset + 24
        $magic = $reader.ReadUInt16(); $stream.Position = $offset + 24 + 68
        return @{ machine = $machine; optional_magic = $magic; subsystem = $reader.ReadUInt16() }
    }
    finally { $reader.Dispose(); $stream.Dispose() }
}
function Export-CargoLicenses($Metadata, [string]$OutputDirectory, [string]$Root) {
    $packages = @{}; $nodes = @{}; $seen = @{}; $pending = New-Object 'Collections.Generic.Stack[string]'
    foreach ($package in $Metadata.packages) { $packages[$package.id] = $package }
    foreach ($node in $Metadata.resolve.nodes) { $nodes[$node.id] = $node }
    foreach ($package in $Metadata.packages) {
        if ($package.name -in @('yu-shell-windows','yu-document-renderer')) { $pending.Push($package.id) }
    }
    while ($pending.Count) {
        $id = $pending.Pop()
        if ($seen.ContainsKey($id)) { continue }
        $seen[$id] = $true
        foreach ($dependency in $nodes[$id].deps) {
            if (@($dependency.dep_kinds | Where-Object { $_.kind -ne 'dev' }).Count) { $pending.Push($dependency.pkg) }
        }
    }
    [IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null
    $rows = @(); $text = New-Object Text.StringBuilder
    [void]$text.AppendLine('Yu Windows locked Cargo dependency notices (includes build dependencies).')
    foreach ($package in @($seen.Keys | ForEach-Object { $packages[$_] } | Where-Object source | Sort-Object name,version)) {
        if (-not $package.license -and -not $package.license_file) { throw "Missing dependency license: $($package.name)" }
        $directory = Split-Path $package.manifest_path
        $files = @(Get-ChildItem -LiteralPath $directory -File | Where-Object { $_.Name -match '(?i)license|copying|notice|unlicense' })
        if ($package.license_file) {
            $declared = Join-Path $directory $package.license_file
            if (-not (Test-Path -LiteralPath $declared -PathType Leaf)) { throw "Missing declared license: $declared" }
            $files += Get-Item -LiteralPath $declared
        }
        $notices = @()
        foreach ($file in @($files | Sort-Object FullName -Unique)) {
            $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
            $notices += @{ name=$file.Name; bytes=$file.Length; sha256=$hash }
            [void]$text.AppendLine("`n===== $($package.name) $($package.version) | $($package.license) | $($file.Name) | $hash =====")
            [void]$text.AppendLine([IO.File]::ReadAllText($file.FullName))
        }
        $rows += @{ name=$package.name; version=$package.version; license=$package.license; source=$package.source; license_files=$notices; published_root_license_file_missing=($notices.Count -eq 0) }
    }
    [IO.File]::WriteAllText((Join-Path $OutputDirectory 'RustDependencyLicenses.txt'), $text.ToString(), (New-Object Text.UTF8Encoding($false)))
    Write-ReleaseJson @{ schema_version=1; target='x86_64-pc-windows-msvc'; roots=@('yu-shell-windows','yu-document-renderer'); cargo_lock_sha256=(Get-FileHash (Join-Path $Root 'Cargo.lock')).Hash; external_package_count=$rows.Count; packages_without_published_root_license_file=@($rows | Where-Object published_root_license_file_missing).Count; packages=$rows } (Join-Path $OutputDirectory 'RustDependencies.json')
    Copy-Item -LiteralPath (Join-Path $Root 'LICENSE') -Destination (Join-Path $OutputDirectory 'Yu-Apache-2.0.txt')
    Copy-Item -Path (Join-Path $Root 'tools/yu-document-renderer/licenses/*.txt') -Destination $OutputDirectory
    Copy-Item -LiteralPath (Join-Path $Root 'tools/yu-document-renderer/vendor/mitex/LICENSE') -Destination (Join-Path $OutputDirectory 'MiTeX.txt')
    Copy-Item -LiteralPath (Join-Path $Root 'tools/yu-document-renderer/vendor/xarrow/LICENSE') -Destination (Join-Path $OutputDirectory 'xarrow.txt')
    Copy-Item -LiteralPath (Join-Path $Root 'vendor/mermaid-rs-renderer/LICENSE') -Destination (Join-Path $OutputDirectory 'Mermaid-Vendored-MIT.txt')
}
