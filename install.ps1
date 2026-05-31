# Hydra Installer for Windows (PowerShell)
# Installs Hydra CLI and Daemon for Ascend CANN operator development
# Usage: .\install.ps1 [-Version "latest"] [-InstallDir "$env:LOCALAPPDATA\Hydra"] [-BuildFromSource]

param(
    [string]$Version = "latest",
    [string]$InstallDir = "$env:LOCALAPPDATA\Hydra",
    [switch]$BuildFromSource
)

$ErrorActionPreference = "Stop"
$RepoOwner = "lggyx"
$RepoName = "Hydra"

Write-Host @"
  __  __           __
 / / / /_  ______/ /________
/ /_/ / / / / __  / ___/ __  /
/ __  / /_/ / /_/ / /  / /_/ /
/_/ /_/\__, /\__,_/_/   \__,_/
      /____/

  Ascend CANN Operator Development & Testing
"@ -ForegroundColor Cyan

Write-Host "Installing Hydra $Version..." -ForegroundColor Green

# Create install directory
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
}

$BinDir = Join-Path $InstallDir "bin"
if (-not (Test-Path $BinDir)) {
    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
}

if ($BuildFromSource) {
    Write-Host "Building from source..." -ForegroundColor Yellow
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-Host "Error: Rust (cargo) is required. Install from https://rustup.rs/" -ForegroundColor Red
        exit 1
    }
    Push-Location $PSScriptRoot
    cargo build --release -p hydra-daemon -p hydra
    if ($LASTEXITCODE -ne 0) { Write-Host "Build failed!" -ForegroundColor Red; exit 1 }
    Copy-Item "target\release\hydra-daemon.exe" $BinDir -Force
    Copy-Item "target\release\hydra.exe" $BinDir -Force
    Pop-Location
} else {
    # Download from GitHub releases
    $Arch = if ([Environment]::Is64BitOperatingSystem) { "x86_64" } else { "i686" }
    $OsName = "windows"
    $BaseUrl = "https://github.com/$RepoOwner/$RepoName/releases"
    if ($Version -eq "latest") {
        $BaseUrl = "https://github.com/$RepoOwner/$RepoName/releases/latest/download"
    } else {
        $BaseUrl = "https://github.com/$RepoOwner/$RepoName/releases/download/$Version"
    }

    $Files = @(
        @{Name="hydra-daemon.exe"; Url="$BaseUrl/hydra-daemon-$OsName-$Arch.exe"},
        @{Name="hydra.exe"; Url="$BaseUrl/hydra-$OsName-$Arch.exe"}
    )

    foreach ($File in $Files) {
        $Dest = Join-Path $BinDir $File.Name
        Write-Host "Downloading $($File.Name)..." -ForegroundColor Yellow
        try {
            Invoke-WebRequest -Uri $File.Url -OutFile $Dest -UseBasicParsing
        } catch {
            Write-Host "Download failed: $($File.Url)" -ForegroundColor Red
            Write-Host "Try building from source: .\install.ps1 -BuildFromSource" -ForegroundColor Yellow
            exit 1
        }
    }
}

# Add to PATH
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$BinDir*") {
    Write-Host "Adding $BinDir to PATH..." -ForegroundColor Yellow
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$BinDir", "User")
    $env:Path = "$env:Path;$BinDir"
}

# Set default environment variables
[Environment]::SetEnvironmentVariable("HYDRA_DAEMON_PORT", "13456", "User")

Write-Host @"

Installation complete!

  hydra         - Launch TUI
  hydra-daemon  - Start API server

Quick start:
  1. Start daemon:  hydra-daemon
  2. In another terminal:  hydra
  3. In TUI:  /login  (free API quota)
  4. Create orchestrator:  /agents create --kind orchestrator
  5. Start operator dev:  /agents <id> start "implement Mul operator"

  Review layer: https://gitcode.com/cann/cannbot-skills
  Docs: https://github.com/$RepoOwner/$RepoName

"@ -ForegroundColor Green
