# Hydra installer for Windows — PowerShell
#
#   irm https://github.com/lggyx/Hydra/releases/download/v4.23.3/install.ps1 | iex
#
# Env overrides:
#   $env:HYDRA_VERSION   release tag to install (default: v4.23.3)
#   $env:HYDRA_PREFIX    install dir (default: %LOCALAPPDATA%\Hydra)
# IMPORTANT: when changing install paths, registry edits, or filenames here,
# also update scripts/uninstall.ps1 AND
# crates/hydra-core/src/uninstall/paths.rs. The CI parity test guards
# the manifest, but binary path / PATH edit are not checked.

$ErrorActionPreference = "Stop"

$Version = if ($env:HYDRA_VERSION) { $env:HYDRA_VERSION } else { "v4.23.3" }
$RepoBase = "https://github.com/lggyx/Hydra/releases/download"

# --- detect arch ---
# Prefer PROCESSOR_ARCHITEW6432 (set only when a 32-bit process runs on a 64-bit
# OS — it holds the real OS arch). Fall back to PROCESSOR_ARCHITECTURE.
# Avoids RuntimeInformation::OSArchitecture which is empty on older PS 5.1/.NET.
$RealArch = if ($env:PROCESSOR_ARCHITEW6432) {
    $env:PROCESSOR_ARCHITEW6432
} else {
    $env:PROCESSOR_ARCHITECTURE
}

switch ($RealArch) {
    "AMD64" { $ArchTag = "x64" }
    "ARM64" { $ArchTag = "arm64" }
    default {
        Write-Host "Unsupported architecture: $RealArch (supported: AMD64, ARM64)" -ForegroundColor Red
        exit 1
    }
}

$BinName = "hydra-$Version-windows-$ArchTag.exe"
$Url = "$RepoBase/$Version/$BinName"

# --- pick install dir ---
$Prefix = if ($env:HYDRA_PREFIX) {
    $env:HYDRA_PREFIX
} else {
    Join-Path $env:LOCALAPPDATA "Hydra"
}

if (-not (Test-Path $Prefix)) {
    New-Item -ItemType Directory -Path $Prefix -Force | Out-Null
}

# --- download ---
$Dest = Join-Path $Prefix "hydra.exe"
$TmpFile = Join-Path $env:TEMP "hydra-download.exe"

Write-Host "==> Downloading $BinName"
Write-Host "    from $Url"

try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $ProgressPreference = 'SilentlyContinue'
    Invoke-WebRequest -Uri $Url -OutFile $TmpFile -UseBasicParsing
} catch {
    Write-Host "Error: download failed." -ForegroundColor Red
    Write-Host "       $_" -ForegroundColor Red
    Write-Host "       URL: $Url" -ForegroundColor Red
    exit 1
}

# Sanity check: must not be an HTML page
$Header = [System.IO.File]::ReadAllBytes($TmpFile)[0..3]
if ([char]$Header[0] -eq '<') {
    Write-Host "Error: download looks like an HTML page, not a binary." -ForegroundColor Red
    Write-Host "       The release may not exist, or the URL is wrong." -ForegroundColor Red
    Write-Host "       URL: $Url" -ForegroundColor Red
    Remove-Item $TmpFile -Force -ErrorAction SilentlyContinue
    exit 1
}

# --- install ---
# Move-Item -Force is unreliable on Windows PowerShell 5.1 when the destination
# already exists (see: fails with "当文件已存在时，无法创建该文件"). Do an explicit
# Remove-Item first, and surface a clear message if the old binary is locked
# (hydra.exe still running in another terminal).
Write-Host "==> Installing to $Dest"
if (Test-Path $Dest) {
    try {
        Remove-Item $Dest -Force -ErrorAction Stop
    } catch {
        Write-Host "Error: cannot replace existing $Dest" -ForegroundColor Red
        Write-Host "       It may be in use. Close any running hydra.exe and re-run this installer." -ForegroundColor Red
        Write-Host "       $_" -ForegroundColor Red
        Remove-Item $TmpFile -Force -ErrorAction SilentlyContinue
        exit 1
    }
}
Move-Item -Path $TmpFile -Destination $Dest

# --- add to PATH ---
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$Prefix*") {
    $NewPath = "$Prefix;$UserPath"
    [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
    # Also update current session so user can use it immediately
    $env:Path = "$Prefix;$env:Path"
    Write-Host ""
    Write-Host "Added $Prefix to user PATH." -ForegroundColor Green
    Write-Host "New terminal windows will pick it up automatically."
}

# --- done ---
Write-Host ""
Write-Host "Installed: $Dest" -ForegroundColor Green
try {
    & $Dest --version
} catch {
    # ignore
}

Write-Host ""
Write-Host "Run 'hydra' to get started." -ForegroundColor Cyan
