# Install script for virgil-cli (Windows)
# Usage: irm https://raw.githubusercontent.com/Delanyo32/virgil-cli/master/install.ps1 | iex

$ErrorActionPreference = "Stop"

$Repo = "Delanyo32/virgil-cli"
$Binary = "virgil-cli"
$Target = "x86_64-pc-windows-msvc"
$InstallDir = if ($env:VIRGIL_INSTALL_DIR) { $env:VIRGIL_INSTALL_DIR } else { Join-Path $env:USERPROFILE ".local\bin" }

# Fetch latest release tag
Write-Host "Fetching latest release..."
$Release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
$Version = $Release.tag_name
Write-Host "Latest version: $Version"

# Find the matching asset download URL directly from the release API
# This avoids constructing URLs manually which can break with tags containing slashes
$Asset = $Release.assets | Where-Object { $_.name -eq "$Binary-$Target.zip" }
if (-not $Asset) {
    Write-Error "Error: could not find asset $Binary-$Target.zip in release $Version"
    exit 1
}
$Url = $Asset.browser_download_url

Write-Host "Downloading $Binary $Version for $Target..."

# Download to temp
$TmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null

try {
    $ZipPath = Join-Path $TmpDir "$Binary-$Target.zip"
    Invoke-WebRequest -Uri $Url -OutFile $ZipPath -UseBasicParsing

    # Extract
    Expand-Archive -Path $ZipPath -DestinationPath $TmpDir -Force

    # Install
    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }

    $BinaryPath = Join-Path $InstallDir "$Binary.exe"
    Move-Item -Path (Join-Path $TmpDir "$Binary.exe") -Destination $BinaryPath -Force

    Write-Host "Installed $Binary to $BinaryPath"

    # Add to PATH if not already present
    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($UserPath -notlike "*$InstallDir*") {
        [Environment]::SetEnvironmentVariable("Path", "$InstallDir;$UserPath", "User")
        $env:Path = "$InstallDir;$env:Path"
        Write-Host "Added $InstallDir to user PATH (restart your terminal for it to take effect)"
    }

    # Verify
    $VersionOutput = & $BinaryPath --version 2>&1
    Write-Host "Verification: $VersionOutput"
}
finally {
    Remove-Item -Path $TmpDir -Recurse -Force -ErrorAction SilentlyContinue
}
