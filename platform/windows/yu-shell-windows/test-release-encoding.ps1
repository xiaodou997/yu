$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'release-utils.ps1')
$previous = [Console]::InputEncoding
try {
    foreach ($encoding in @([Text.Encoding]::ASCII, (New-Object Text.UTF8Encoding($true)))) {
        [Console]::InputEncoding = $encoding
        $info = New-Object Diagnostics.ProcessStartInfo
        $info.FileName = (Get-Command powershell.exe).Source
        $probe = '$stream = [Console]::OpenStandardInput(); $bytes = New-Object byte[] 3; for ($i = 0; $i -lt 3; $i++) { $value = $stream.ReadByte(); if ($value -lt 0) { exit 2 }; $bytes[$i] = $value }; [Console]::WriteLine([BitConverter]::ToString($bytes))'
        $info.Arguments = '-NoProfile -NonInteractive -EncodedCommand ' + [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($probe))
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $true
        $info.RedirectStandardInput = $true
        $info.RedirectStandardOutput = $true
        $process = Start-ReleaseJsonProcess $info
        try {
            $bytes = [Text.Encoding]::UTF8.GetBytes('{"id":1}')
            $process.StandardInput.BaseStream.Write($bytes, 0, $bytes.Length)
            $process.StandardInput.BaseStream.Close()
            $result = $process.StandardOutput.ReadLineAsync()
            if (-not $result.Wait(10000) -or -not $process.WaitForExit(10000)) { throw 'JSON pipe probe timed out.' }
            if ($process.ExitCode -ne 0 -or $result.Result -ne '7B-22-69') { throw "JSON pipe contains a preamble or corrupt bytes: $($result.Result)" }
            if ([Console]::InputEncoding.CodePage -ne $encoding.CodePage) { throw 'Caller console encoding was not restored.' }
            Write-Host "JSON pipe starts with JSON bytes under PowerShell $($PSVersionTable.PSVersion), console $($encoding.CodePage)."
        }
        finally {
            if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
            $process.Dispose()
        }
    }
}
finally { [Console]::InputEncoding = $previous }
