# 生成猫步下载器 NSIS 品牌图片：header 150x57 / sidebar 164x314（24bpp BMP）
# 配色与应用浅色主题一致：底 #F5F5F7、主文字 #1D1D1F、次文字 #86868B
Add-Type -AssemblyName System.Drawing

$dir = "C:\Users\20269\Desktop\下载器\src-tauri"
$iconPath = Join-Path $dir "icons\128x128.png"
$bgColor = [System.Drawing.Color]::FromArgb(0xF5, 0xF5, 0xF7)
$mainColor = [System.Drawing.Color]::FromArgb(0x1D, 0x1D, 0x1F)
$subColor = [System.Drawing.Color]::FromArgb(0x86, 0x86, 0x8B)

function Save-Bmp24($bmp, $path) {
  $rect = New-Object System.Drawing.Rectangle(0, 0, $bmp.Width, $bmp.Height)
  $locked = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadWrite, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
  $out = New-Object System.Drawing.Bitmap($bmp.Width, $bmp.Height, $locked.Stride, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb, $locked.Scan0)
  $out.Save($path, [System.Drawing.Imaging.ImageFormat]::Bmp)
  $out.Dispose()
  $bmp.UnlockBits($locked)
}

# ---- header 150x57：右侧猫图标 + 名称 ----
$hBmp = New-Object System.Drawing.Bitmap(150, 57)
$hg = [System.Drawing.Graphics]::FromImage($hBmp)
$hg.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$hg.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAlias
$hg.Clear($bgColor)
$icon = [System.Drawing.Image]::FromFile($iconPath)
$hg.DrawImage($icon, (150 - 8 - 36), 10, 36, 36)
$icon.Dispose()
$fontH = New-Object System.Drawing.Font("Microsoft YaHei", 12, [System.Drawing.FontStyle]::Regular, [System.Drawing.GraphicsUnit]::Pixel)
$brushMain = New-Object System.Drawing.SolidBrush($mainColor)
$fmtRight = New-Object System.Drawing.StringFormat
$fmtRight.Alignment = [System.Drawing.StringAlignment]::Far
$hg.DrawString("猫步下载器", $fontH, $brushMain, (150 - 8 - 36 - 6), 21, $fmtRight)
$hg.Dispose()
Save-Bmp24 $hBmp (Join-Path $dir "installer-header.bmp")
$hBmp.Dispose()

# ---- sidebar 164x314：竖排图标 + 名称 + 英文名 + 特性小字 ----
$sBmp = New-Object System.Drawing.Bitmap(164, 314)
$sg = [System.Drawing.Graphics]::FromImage($sBmp)
$sg.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$sg.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAlias
$sg.Clear($bgColor)
$icon2 = [System.Drawing.Image]::FromFile($iconPath)
$sg.DrawImage($icon2, ((164 - 84) / 2), 84, 84, 84)
$icon2.Dispose()
$fmtCenter = New-Object System.Drawing.StringFormat
$fmtCenter.Alignment = [System.Drawing.StringAlignment]::Center
$fontTitle = New-Object System.Drawing.Font("Microsoft YaHei", 20, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
$fontSub = New-Object System.Drawing.Font("Segoe UI", 12, [System.Drawing.FontStyle]::Regular, [System.Drawing.GraphicsUnit]::Pixel)
$fontFeat = New-Object System.Drawing.Font("Microsoft YaHei", 11, [System.Drawing.FontStyle]::Regular, [System.Drawing.GraphicsUnit]::Pixel)
$brushSub = New-Object System.Drawing.SolidBrush($subColor)
$sg.DrawString("猫步下载器", $fontTitle, $brushMain, 82, 184, $fmtCenter)
$sg.DrawString("Maobu Fetch", $fontSub, $brushSub, 82, 214, $fmtCenter)
$sg.DrawString("本地优先 · 断点续传", $fontFeat, $brushSub, 82, 268, $fmtCenter)
$sg.DrawString("浏览器接管 · 媒体下载", $fontFeat, $brushSub, 82, 286, $fmtCenter)
$sg.Dispose()
Save-Bmp24 $sBmp (Join-Path $dir "installer-sidebar.bmp")
$sBmp.Dispose()

Write-Host "branding BMPs generated"
