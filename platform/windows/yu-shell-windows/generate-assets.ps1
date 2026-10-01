param([string]$OutputDirectory = (Join-Path $PSScriptRoot 'AppBundle'))
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$source = [Drawing.Image]::FromFile((Join-Path $PSScriptRoot '../../macos/yu-shell-macos/AppBundle/Resources/Yu.png'))
function Resize-Icon([int]$Size) {
    $bitmap = New-Object Drawing.Bitmap($Size, $Size)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.Clear([Drawing.Color]::Transparent)
        $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $graphics.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $graphics.DrawImage($source, 0, 0, $Size, $Size)
        $stream = New-Object IO.MemoryStream
        try { $bitmap.Save($stream, [Drawing.Imaging.ImageFormat]::Png); return ,$stream.ToArray() }
        finally { $stream.Dispose() }
    }
    finally { $graphics.Dispose(); $bitmap.Dispose() }
}
try {
    $assets = Join-Path $OutputDirectory 'Assets'
    [IO.Directory]::CreateDirectory($assets) | Out-Null
    foreach ($row in @(@('StoreLogo',50), @('Square44x44Logo',44), @('Square150x150Logo',150))) {
        foreach ($scale in @(100,125,150,200,400)) {
            $suffix = if ($scale -eq 100) { '' } else { '.scale-' + $scale }
            [IO.File]::WriteAllBytes((Join-Path $assets ($row[0] + $suffix + '.png')), (Resize-Icon ([int]($row[1] * $scale / 100))))
        }
    }
    foreach ($size in @(16,24,32,48,64,256)) {
        [IO.File]::WriteAllBytes((Join-Path $assets "Square44x44Logo.targetsize-${size}_altform-unplated.png"), (Resize-Icon $size))
    }
    $sizes = @(16,24,32,48,64,128,256)
    $images = @($sizes | ForEach-Object { ,(Resize-Icon $_) })
    $file = [IO.File]::Create((Join-Path $OutputDirectory 'Yu.ico'))
    $writer = New-Object IO.BinaryWriter($file)
    try {
        $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$sizes.Count)
        $offset = 6 + 16 * $sizes.Count
        for ($index = 0; $index -lt $sizes.Count; $index++) {
            $dimension = if ($sizes[$index] -eq 256) { 0 } else { $sizes[$index] }
            $writer.Write([byte]$dimension); $writer.Write([byte]$dimension)
            $writer.Write([byte]0); $writer.Write([byte]0); $writer.Write([uint16]1); $writer.Write([uint16]32)
            $writer.Write([uint32]$images[$index].Length); $writer.Write([uint32]$offset)
            $offset += $images[$index].Length
        }
        foreach ($bytes in $images) { $writer.Write([byte[]]$bytes) }
    }
    finally { $writer.Dispose(); $file.Dispose() }
}
finally { $source.Dispose() }
