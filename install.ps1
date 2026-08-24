# script install.ps1 - One-line installer for toolkitrs on Windows systems.
# Fetches the latest (or specified) release from GitHub, extracts the binary,
# and installs it to a local program directory, adding it to the user's PATH.

param(
    [string]$Version = ""
)

$ErrorActionPreference = "Stop"

# ----------------------- Configuration -----------------------

$Repo = "seyallius/toolkitrs"
$BinaryName = "toolkitrs"
$InstallDir = "$env:LOCALAPPDATA\Programs\toolkitrs"

# ----------------------- Arch Detection -----------------------

# Standard Rust Windows target is x86_64-pc-windows-msvc
$Arch = if ([Environment]::Is64BitOperatingSystem) { "x86_64" } else { "i686" }
$Target = "$Arch-pc-windows-msvc"
$AssetName = "$BinaryName-$Target.zip"

# ----------------------- Version Resolution -----------------------

if (-not $Version) {
    Write-Host "🔍 Fetching latest version..."
    $Release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
    $Version = $Release.tag_name
}

Write-Host "📦 Installing $BinaryName version $Version for $Target..."

# ----------------------- Download & Extract -----------------------

$DownloadUrl = "https://github.com/$Repo/releases/download/$Version/$AssetName"
$TempDir = New-TemporaryFile | ForEach-Object { Remove-Item $_; New-Item -ItemType Directory -Path $_ }
$ZipPath = Join-Path $TempDir $AssetName

Write-Host "⬇️  Downloading $AssetName..."
try {
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $ZipPath -ErrorAction Stop
} catch {
    Write-Error "❌ Failed to download asset. Does $AssetName exist in release $Version?"
    exit 1
}

Write-Host "📂 Extracting..."
Expand-Archive -Path $ZipPath -DestinationPath $TempDir -Force

$BinPath = Get-ChildItem -Path $TempDir -Recurse -Filter "$BinaryName.exe" | Select-Object -First 1
if (-not $BinPath) {
    Write-Error "❌ Could not find $BinaryName.exe in archive."
    exit 1
}

# ----------------------- Installation -----------------------

if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

Write-Host "🚀 Installing to $InstallDir..."
Move-Item -Path $BinPath.FullName -Destination "$InstallDir\$BinaryName.exe" -Force

# Add to PATH if not already there
$CurrentPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($CurrentPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$CurrentPath;$InstallDir", "User")
    Write-Host "🔗 Added $InstallDir to user PATH. You may need to restart your terminal."
}

# Cleanup
Remove-Item -Recurse -Force $TempDir

Write-Host "✅ Successfully installed $BinaryName to $InstallDir\$BinaryName.exe"
Write-Host "✨ Run '$BinaryName --help' to get started!"
