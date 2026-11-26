param(
    [string]$AssetDirectory = (Join-Path $PSScriptRoot 'assets')
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
New-Item -ItemType Directory -Force -Path $AssetDirectory | Out-Null

function Convert-HexColor {
    param([string]$Hex, [int]$Alpha = 255)

    $value = [Convert]::ToInt32($Hex.TrimStart('#'), 16)
    return [System.Drawing.Color]::FromArgb(
        $Alpha,
        ($value -shr 16) -band 255,
        ($value -shr 8) -band 255,
        $value -band 255
    )
}

function Set-GraphicsQuality {
    param([System.Drawing.Graphics]$Graphics)

    $Graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $Graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $Graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $Graphics.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
}

function Draw-OrbitMark {
    param(
        [System.Drawing.Graphics]$Graphics,
        [int]$X,
        [int]$Y,
        [int]$Size
    )

    $edge = Convert-HexColor '58C9F7'
    $inside = Convert-HexColor '102235'
    $markScale = $Size / 31.0
    $insideBrush = [System.Drawing.SolidBrush]::new($inside)
    $outlinePen = [System.Drawing.Pen]::new($edge, 1.35 * $markScale)
    $Graphics.FillEllipse($insideBrush, $X, $Y, $Size, $Size)
    $Graphics.DrawEllipse(
        $outlinePen,
        $X + 0.7 * $markScale,
        $Y + 0.7 * $markScale,
        $Size - 1.4 * $markScale,
        $Size - 1.4 * $markScale
    )
    $insideBrush.Dispose()
    $outlinePen.Dispose()

    $glyphPen = [System.Drawing.Pen]::new($edge, [Math]::Max(1.5 * $markScale, $Size / 15.0))
    $glyphPen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
    $glyphPen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
    $centerY = $Y + $Size * 0.5
    $Graphics.DrawLine($glyphPen, $X + $Size * 0.31, $centerY - $Size * 0.2, $X + $Size * 0.48, $centerY)
    $Graphics.DrawLine($glyphPen, $X + $Size * 0.48, $centerY, $X + $Size * 0.31, $centerY + $Size * 0.2)
    $Graphics.DrawLine($glyphPen, $X + $Size * 0.57, $centerY + $Size * 0.2, $X + $Size * 0.72, $centerY + $Size * 0.2)
    $glyphPen.Dispose()
}

function New-BitmapCanvas {
    param(
        [int]$Width,
        [int]$Height,
        [string]$TopColor,
        [string]$BottomColor
    )

    $bitmap = [System.Drawing.Bitmap]::new(
        $Width,
        $Height,
        [System.Drawing.Imaging.PixelFormat]::Format24bppRgb
    )
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    Set-GraphicsQuality $graphics
    $bounds = [System.Drawing.Rectangle]::new(0, 0, $Width, $Height)
    $background = [System.Drawing.Drawing2D.LinearGradientBrush]::new(
        $bounds,
        (Convert-HexColor $TopColor),
        (Convert-HexColor $BottomColor),
        [System.Drawing.Drawing2D.LinearGradientMode]::Vertical
    )
    $graphics.FillRectangle($background, $bounds)
    $background.Dispose()

    return @{ Bitmap = $bitmap; Graphics = $graphics }
}

function Save-BitmapCanvas {
    param($Canvas, [string]$Path)

    $Canvas.Graphics.Dispose()
    $Canvas.Bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Bmp)
    $Canvas.Bitmap.Dispose()
}

$wizardScale = 3
$wizard = New-BitmapCanvas (164 * $wizardScale) (314 * $wizardScale) '0B1724' '102A3F'
$g = $wizard.Graphics

# Low-contrast orbit paths keep the panel atmospheric without competing with copy.
foreach ($diameter in @(102, 132, 162)) {
    $orbit = [System.Drawing.Pen]::new((Convert-HexColor '6CCFF5' 24), $wizardScale)
    $g.DrawArc($orbit, 64 * $wizardScale, 166 * $wizardScale, $diameter * $wizardScale, $diameter * $wizardScale, 195, 185)
    $orbit.Dispose()
}
foreach ($dot in @(@(137, 52), @(119, 79), @(145, 205), @(99, 220), @(34, 218), @(141, 286))) {
    $star = [System.Drawing.SolidBrush]::new((Convert-HexColor '95DFFF' 115))
    $g.FillEllipse($star, $dot[0] * $wizardScale, $dot[1] * $wizardScale, 2.2 * $wizardScale, 2.2 * $wizardScale)
    $star.Dispose()
}

Draw-OrbitMark $g (18 * $wizardScale) (22 * $wizardScale) (31 * $wizardScale)
$brandFont = [System.Drawing.Font]::new('Segoe UI Semibold', 9.4 * $wizardScale, [System.Drawing.FontStyle]::Bold)
$brandBrush = [System.Drawing.SolidBrush]::new((Convert-HexColor 'F4F8FC'))
$g.DrawString('OrbitShell', $brandFont, $brandBrush, 57 * $wizardScale, 27 * $wizardScale)

$eyebrowFont = [System.Drawing.Font]::new('Segoe UI', 6.6 * $wizardScale, [System.Drawing.FontStyle]::Bold)
$eyebrowBrush = [System.Drawing.SolidBrush]::new((Convert-HexColor '72CFF5'))
$g.DrawString('REMOTE WORKSPACE', $eyebrowFont, $eyebrowBrush, 18 * $wizardScale, 78 * $wizardScale)

$headlineFont = [System.Drawing.Font]::new('Microsoft YaHei UI', 13.7 * $wizardScale, [System.Drawing.FontStyle]::Bold)
$headlineBrush = [System.Drawing.SolidBrush]::new((Convert-HexColor 'F5F8FC'))
$g.DrawString('连接远端，', $headlineFont, $headlineBrush, 17 * $wizardScale, 103 * $wizardScale)
$g.DrawString('专注作业', $headlineFont, $headlineBrush, 17 * $wizardScale, 130 * $wizardScale)

$subtitleFont = [System.Drawing.Font]::new('Microsoft YaHei UI', 7.2 * $wizardScale, [System.Drawing.FontStyle]::Regular)
$mutedBrush = [System.Drawing.SolidBrush]::new((Convert-HexColor 'A7BDCE'))
$g.DrawString('终端、文件与自动化工具', $subtitleFont, $mutedBrush, 18 * $wizardScale, 169 * $wizardScale)

$divider = [System.Drawing.Pen]::new((Convert-HexColor 'A6C7D9' 48), $wizardScale)
$g.DrawLine($divider, 18 * $wizardScale, 211 * $wizardScale, 146 * $wizardScale, 211 * $wizardScale)
$divider.Dispose()

$featureFont = [System.Drawing.Font]::new('Microsoft YaHei UI', 7.1 * $wizardScale, [System.Drawing.FontStyle]::Regular)
$featureBrush = [System.Drawing.SolidBrush]::new((Convert-HexColor 'D4E1EA'))
foreach ($feature in @(
    @{ Y = 231; Text = '安全 SSH 会话' },
    @{ Y = 253; Text = 'SFTP 远程文件' },
    @{ Y = 275; Text = 'MCP 工具接入' }
)) {
    $bullet = [System.Drawing.SolidBrush]::new((Convert-HexColor '58C9F7'))
    $g.FillEllipse($bullet, 19 * $wizardScale, ($feature.Y + 4) * $wizardScale, 4 * $wizardScale, 4 * $wizardScale)
    $bullet.Dispose()
    $g.DrawString($feature.Text, $featureFont, $featureBrush, 30 * $wizardScale, $feature.Y * $wizardScale)
}

$footerFont = [System.Drawing.Font]::new('Segoe UI', 5.8 * $wizardScale, [System.Drawing.FontStyle]::Bold)
$footerBrush = [System.Drawing.SolidBrush]::new((Convert-HexColor '6F8DA3'))
$g.DrawString('ORBIT  /  0.2', $footerFont, $footerBrush, 18 * $wizardScale, 297 * $wizardScale)
Save-BitmapCanvas $wizard (Join-Path $AssetDirectory 'OrbitShellWizard.bmp')

$headerScale = 3
$header = New-BitmapCanvas (150 * $headerScale) (57 * $headerScale) 'F4F7FB' 'F4F7FB'
$g = $header.Graphics
Draw-OrbitMark $g (10 * $headerScale) (13 * $headerScale) (30 * $headerScale)
$headerBrandFont = [System.Drawing.Font]::new('Segoe UI Semibold', 9.0 * $headerScale, [System.Drawing.FontStyle]::Bold)
$headerBrandBrush = [System.Drawing.SolidBrush]::new((Convert-HexColor '182A3B'))
$g.DrawString('OrbitShell', $headerBrandFont, $headerBrandBrush, 48 * $headerScale, 12 * $headerScale)
$headerSubFont = [System.Drawing.Font]::new('Segoe UI', 5.9 * $headerScale, [System.Drawing.FontStyle]::Bold)
$headerSubBrush = [System.Drawing.SolidBrush]::new((Convert-HexColor '718396'))
$g.DrawString('SSH / SFTP / MCP', $headerSubFont, $headerSubBrush, 49 * $headerScale, 30 * $headerScale)
$accent = [System.Drawing.Pen]::new((Convert-HexColor '60C9F4'), 1.5 * $headerScale)
$g.DrawLine($accent, 10 * $headerScale, 54 * $headerScale, 140 * $headerScale, 54 * $headerScale)
$accent.Dispose()
Save-BitmapCanvas $header (Join-Path $AssetDirectory 'OrbitShellHeader.bmp')

Write-Output "Created installer artwork in $AssetDirectory"
