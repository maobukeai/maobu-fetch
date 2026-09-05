# =====================================================================
#  猫步下载器 (Maobu Fetch) - 一键启动与开发管理脚本 (PowerShell)
# =====================================================================

param (
    [string]$Action = ""
)

[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$Host.UI.RawUI.WindowTitle = "猫步下载器 (Maobu Fetch) - 启动器"

$RootDir = Split-Path -Parent $PSScriptRoot
Set-Location $RootDir

function Print-Banner {
    Write-Host ""
    Write-Host " ==================================================================== " -ForegroundColor Cyan
    Write-Host "            🐱 猫步下载器 (Maobu Fetch) - 一键启动器                   " -ForegroundColor Cyan
    Write-Host " ==================================================================== " -ForegroundColor Cyan
    Write-Host ""
}

function Check-Environment {
    Write-Host "[*] 正在检查基础开发环境..." -ForegroundColor Gray

    # 1. 检查 Node.js
    $node = Get-Command node -ErrorAction SilentlyContinue
    if (-not $node) {
        Write-Host "[X] 错误: 未检测到 Node.js，请先安装 Node.js (推荐 v20 或更高版本)。" -ForegroundColor Red
        Write-Host "    官网下载: https://nodejs.org/" -ForegroundColor Yellow
        if (-not [Console]::IsInputRedirected) {
            Read-Host "按回车键退出..."
        }
        exit 1
    }

    # 2. 检查 pnpm
    $pnpm = Get-Command pnpm -ErrorAction SilentlyContinue
    if (-not $pnpm) {
        Write-Host "[!] 提示: 未检测到全局 pnpm，正在尝试通过 npm 自动安装..." -ForegroundColor Yellow
        npm install -g pnpm
        $pnpm = Get-Command pnpm -ErrorAction SilentlyContinue
        if (-not $pnpm) {
            Write-Host "[X] 错误: pnpm 安装失败，请手动在终端运行: npm install -g pnpm" -ForegroundColor Red
            if (-not [Console]::IsInputRedirected) {
                Read-Host "按回车键退出..."
            }
            exit 1
        }
        Write-Host "[V] pnpm 安装完成。" -ForegroundColor Green
    }

    # 3. 检查 node_modules 依赖
    if (-not (Test-Path "node_modules")) {
        Write-Host "[*] 首次运行，正在自动安装项目前端依赖 (pnpm install)..." -ForegroundColor Yellow
        pnpm install
        if ($LASTEXITCODE -ne 0) {
            Write-Host "[X] 错误: 依赖安装失败，请检查网络设置后重试。" -ForegroundColor Red
            if (-not [Console]::IsInputRedirected) {
                Read-Host "按回车键退出..."
            }
            exit 1
        }
        Write-Host "[V] 前端依赖安装成功。" -ForegroundColor Green
    }

    # 4. 检查 Cargo / Rust
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargo) {
        Write-Host "[!] 警告: 未检测到 Rust (cargo) 工具链。运行桌面端需要安装 Rust。" -ForegroundColor Yellow
        Write-Host "    官网下载: https://rustup.rs/" -ForegroundColor Yellow
    }
}

function Find-VsDevCmd {
    $vsDevCmd = $null

    # 优先通过 vswhere 动态定位 Visual Studio
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $vsInstall = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
        if ($vsInstall -and (Test-Path (Join-Path $vsInstall "Common7\Tools\VsDevCmd.bat"))) {
            $vsDevCmd = Join-Path $vsInstall "Common7\Tools\VsDevCmd.bat"
        }
    }

    # 备用常用固定路径探测
    if (-not $vsDevCmd) {
        $candidates = @(
            "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat",
            "C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\Tools\VsDevCmd.bat",
            "C:\Program Files\Microsoft Visual Studio\2022\Professional\Common7\Tools\VsDevCmd.bat",
            "C:\Program Files\Microsoft Visual Studio\2022\Enterprise\Common7\Tools\VsDevCmd.bat"
        )
        foreach ($path in $candidates) {
            if (Test-Path $path) {
                $vsDevCmd = $path
                break
            }
        }
    }

    return $vsDevCmd
}

function Start-DesktopDev {
    Write-Host ""
    Write-Host "======================================================================" -ForegroundColor Cyan
    Write-Host "  正在启动猫步下载器桌面端 (开发热重载模式)..." -ForegroundColor Cyan
    Write-Host "======================================================================" -ForegroundColor Cyan

    $vsDevCmd = Find-VsDevCmd
    if ($vsDevCmd) {
        Write-Host "[*] 已配置 MSVC 编译环境: $vsDevCmd" -ForegroundColor Gray
        cmd.exe /c "call `"$vsDevCmd`" -arch=x64 -host_arch=x64 >nul 2>nul && pnpm tauri dev"
    } else {
        Write-Host "[!] 未找到 VsDevCmd.bat，尝试直接调用系统 PATH 启动..." -ForegroundColor Yellow
        pnpm tauri dev
    }

    if ($LASTEXITCODE -ne 0) {
        Write-Host ""
        Write-Host "[X] 桌面应用退出或发生异常 (退出码: $LASTEXITCODE)" -ForegroundColor Red
        if (-not [Console]::IsInputRedirected) {
            Read-Host "按回车键继续..."
        }
    }
}

function Start-WebPreview {
    Write-Host ""
    Write-Host "======================================================================" -ForegroundColor Cyan
    Write-Host "  正在启动纯前端 Web 界面预览 (pnpm dev)..." -ForegroundColor Cyan
    Write-Host "======================================================================" -ForegroundColor Cyan
    pnpm dev
}

function Run-HealthCheck {
    Write-Host ""
    Write-Host "======================================================================" -ForegroundColor Cyan
    Write-Host "  正在运行项目全套自检与自动化测试 (TypeScript + 单元测试)..." -ForegroundColor Cyan
    Write-Host "======================================================================" -ForegroundColor Cyan
    pnpm run check
    if ($LASTEXITCODE -eq 0) {
        Write-Host ""
        Write-Host "[V] 所有测试与类型检查全部通过！" -ForegroundColor Green
    } else {
        Write-Host ""
        Write-Host "[X] 测试或自检未完全通过，请查看上方日志分析问题。" -ForegroundColor Red
    }
    Write-Host ""
    if (-not [Console]::IsInputRedirected) {
        Read-Host "按回车键返回菜单..."
    }
}

function Build-ReleasePackage {
    Write-Host ""
    Write-Host "======================================================================" -ForegroundColor Cyan
    Write-Host "  正在执行 Release 生产构建打包 (pnpm tauri build)..." -ForegroundColor Cyan
    Write-Host "======================================================================" -ForegroundColor Cyan

    $vsDevCmd = Find-VsDevCmd
    if ($vsDevCmd) {
        Write-Host "[*] 已配置 MSVC 编译环境: $vsDevCmd" -ForegroundColor Gray
        cmd.exe /c "call `"$vsDevCmd`" -arch=x64 -host_arch=x64 >nul 2>nul && pnpm tauri build"
    } else {
        pnpm tauri build
    }

    if ($LASTEXITCODE -eq 0) {
        Write-Host ""
        Write-Host "[V] 生产打包完成！安装包文件位于: src-tauri\target\release\bundle\" -ForegroundColor Green
    } else {
        Write-Host ""
        Write-Host "[X] 打包失败，请检查上方报错输出。" -ForegroundColor Red
    }
    Write-Host ""
    if (-not [Console]::IsInputRedirected) {
        Read-Host "按回车键返回菜单..."
    }
}

function Clean-BuildCache {
    Write-Host ""
    Write-Host "======================================================================" -ForegroundColor Cyan
    Write-Host "  正在清理编译垃圾与临时构建产物..." -ForegroundColor Cyan
    Write-Host "======================================================================" -ForegroundColor Cyan

    pnpm run clean
    if (Test-Path "dist") {
        Remove-Item -Recurse -Force "dist" -ErrorAction SilentlyContinue
    }
    if (Test-Path "extension\dist") {
        Remove-Item -Recurse -Force "extension\dist" -ErrorAction SilentlyContinue
    }

    Write-Host ""
    Write-Host "[V] 编译缓存已全部安全清理完毕！" -ForegroundColor Green
    Write-Host ""
    if (-not [Console]::IsInputRedirected) {
        Read-Host "按回车键返回菜单..."
    }
}

# --- 主入口 ---
Print-Banner
Check-Environment

# 支持命令行参数快速调用 (例如: .\一键启动.bat dev 或 .\start.bat clean)
switch ($Action.ToLower()) {
    "dev"     { Start-DesktopDev; exit 0 }
    "desktop" { Start-DesktopDev; exit 0 }
    "web"     { Start-WebPreview; exit 0 }
    "check"   { Run-HealthCheck; exit 0 }
    "test"    { Run-HealthCheck; exit 0 }
    "build"   { Build-ReleasePackage; exit 0 }
    "clean"   { Clean-BuildCache; exit 0 }
}

while ($true) {
    Write-Host "请选择要执行的操作：" -ForegroundColor White
    Write-Host "----------------------------------------------------------------------" -ForegroundColor DarkGray
    Write-Host " [1] 启动桌面客户端 (推荐开发调试，支持热重载，默认选项)" -ForegroundColor Green
    Write-Host " [2] 启动纯前端界面 (Web 预览，仅调试 UI 页面)" -ForegroundColor Cyan
    Write-Host " [3] 运行自动化测试与自检 (TypeScript + 单元测试)" -ForegroundColor Yellow
    Write-Host " [4] 打包桌面发布版 (生成 Release 安装包及便携版)" -ForegroundColor Magenta
    Write-Host " [5] 一键清理编译垃圾缓存 (释放数十 GB 编译临时文件)" -ForegroundColor DarkCyan
    Write-Host " [0] 退出" -ForegroundColor Gray
    Write-Host "----------------------------------------------------------------------" -ForegroundColor DarkGray

    $choice = ""
    if ([Console]::IsInputRedirected) {
        $line = [Console]::In.ReadLine()
        if ($null -ne $line) {
            $choice = $line.Trim()
        }
    } else {
        $choice = Read-Host "请输入编号 [0-5] (直接回车默认启动 [1])"
    }

    if ([string]::IsNullOrWhiteSpace($choice)) {
        $choice = "1"
    }

    switch ($choice) {
        "1" { Start-DesktopDev; exit 0 }
        "2" { Start-WebPreview; exit 0 }
        "3" { Run-HealthCheck }
        "4" { Build-ReleasePackage }
        "5" { Clean-BuildCache }
        "0" { Write-Host "已退出启动器。" -ForegroundColor Gray; exit 0 }
        default {
            Write-Host "[!] 输入无效，请输入 0-5 之间的数字。" -ForegroundColor Red
            Write-Host ""
        }
    }

    if ([Console]::IsInputRedirected -and [string]::IsNullOrWhiteSpace($line)) {
        break
    }
}