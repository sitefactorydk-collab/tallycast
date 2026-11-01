# Publishes a new Tallycast version.
#
#   npm.cmd run release -- 0.2.0
#
# Sets the version in package.json, package-lock.json, tauri.conf.json, Cargo.toml and
# Cargo.lock, commits "Release v0.2.0", tags v0.2.0 and pushes both. GitHub then builds
# the installer and publishes the release (.github/workflows/release.yml), which the app
# offers to everyone running an older version.

param([Parameter(Mandatory = $true, Position = 0)][string]$Version)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Invoke-Git {
    & git @args
    if ($LASTEXITCODE -ne 0) { throw "git $($args -join ' ') failed." }
}

$Version = $Version.Trim().TrimStart('v')
if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Use a version like 0.2.0 (got '$Version')."
}

# Start from a clean, up-to-date main branch.
$branch = (& git rev-parse --abbrev-ref HEAD).Trim()
if ($branch -ne 'main') { throw "Switch to the main branch first (you are on '$branch')." }
if (& git status --porcelain) { throw 'You have uncommitted changes. Commit or discard them first (see: git status).' }
Invoke-Git fetch origin --tags --quiet
Invoke-Git pull --ff-only --quiet
if (& git tag --list "v$Version") { throw "v$Version already exists. Pick a higher version." }

$current = (Get-Content (Join-Path $root 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
if ([version]$Version -le [version]$current) {
    throw "The new version must be higher than the current one ($current)."
}

# Replace the version in place, keeping each file's formatting (UTF-8, no BOM).
function Set-Version([string]$Path, [string]$Pattern, [int]$Count) {
    $full = Join-Path $root $Path
    $text = [IO.File]::ReadAllText($full)
    $regex = [regex]$Pattern
    if (-not $regex.IsMatch($text)) { throw "Could not find the version in $Path." }
    $text = $regex.Replace($text, '${1}' + $Version + '${2}', $Count)
    [IO.File]::WriteAllText($full, $text)
}

Set-Version 'package.json' '(?m)^(\s*"version":\s*")[^"]+(")' 1
Set-Version 'package-lock.json' '("name":\s*"tallycast",\s*"version":\s*")[^"]+(")' -1
Set-Version 'src-tauri\tauri.conf.json' '(?m)^(\s*"version":\s*")[^"]+(")' 1
Set-Version 'src-tauri\Cargo.toml' '(?m)^(version\s*=\s*")[^"]+(")' 1
Set-Version 'src-tauri\Cargo.lock' '(name = "tallycast"\r?\nversion = ")[^"]+(")' 1

Invoke-Git add package.json package-lock.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
Invoke-Git commit --quiet -m "Release v$Version"
Invoke-Git tag -a "v$Version" -m "Tallycast $Version"
Invoke-Git push --quiet origin main
Invoke-Git push --quiet origin "v$Version"

Write-Host ""
Write-Host "Version $Version is on its way (was $current)." -ForegroundColor Green
Write-Host "GitHub is now building the installer; this takes about 10-20 minutes."
Write-Host "Follow it here: https://github.com/sitefactorydk-collab/tallycast/actions"
Write-Host "When it is done, the release appears at: https://github.com/sitefactorydk-collab/tallycast/releases"
