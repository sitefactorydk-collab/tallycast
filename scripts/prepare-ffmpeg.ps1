# Copies an ffmpeg.exe into src-tauri\binaries so the installer can ship it.
# Tauri bundles "external binaries" named <name>-<target-triple>.exe and installs them
# next to Tallycast.exe as ffmpeg.exe, where the app looks first.
#
# Runs automatically before `npm run build` on Windows. Search order:
#   1. $env:TALLYCAST_FFMPEG (a path you set yourself)
#   2. The winget Gyan.FFmpeg package folder
#   3. Whatever `ffmpeg` resolves to on PATH

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$binDir = Join-Path $root 'src-tauri\binaries'
$target = Join-Path $binDir 'ffmpeg-x86_64-pc-windows-msvc.exe'

if (Test-Path $target) {
    Write-Host "FFmpeg already prepared: $target"
    exit 0
}

$candidates = @()

if ($env:TALLYCAST_FFMPEG) {
    $candidates += $env:TALLYCAST_FFMPEG
}

$wingetPackages = Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Packages'
if (Test-Path $wingetPackages) {
    $candidates += Get-ChildItem -Path $wingetPackages -Directory -Filter '*FFmpeg*' -ErrorAction SilentlyContinue |
        ForEach-Object { Get-ChildItem -Path $_.FullName -Recurse -Filter 'ffmpeg.exe' -ErrorAction SilentlyContinue } |
        Sort-Object LastWriteTime -Descending |
        ForEach-Object { $_.FullName }
}

$onPath = Get-Command ffmpeg -ErrorAction SilentlyContinue
if ($onPath) {
    $candidates += $onPath.Source
}

$source = $candidates | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1

if (-not $source) {
    Write-Error @"
No ffmpeg.exe was found to bundle.
Install it with:  winget install Gyan.FFmpeg
or point to one:  `$env:TALLYCAST_FFMPEG = 'C:\path\to\ffmpeg.exe'
then run the build again.
"@
    exit 1
}

New-Item -ItemType Directory -Force -Path $binDir | Out-Null
# Resolve symlinks (winget's Links folder) so the real file is copied.
$item = Get-Item $source
if ($item.LinkType -and $item.Target) {
    $source = @($item.Target)[0]
}
Copy-Item -Path $source -Destination $target -Force
$sizeMb = [math]::Round((Get-Item $target).Length / 1MB, 1)
Write-Host "Bundling FFmpeg from $source ($sizeMb MB)"
