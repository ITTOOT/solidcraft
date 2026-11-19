<#
.SYNOPSIS
    Quick start for the ITTOOT Solidcraft OT Connection Bridge.

.DESCRIPTION
    Builds what is missing and opens the bridge on the HoistGantry project in one command:
    the bridge itself, and the Solidcraft window it drives (so the orange START SOLIDCRAFT
    button on the Model page works). Your saved settings, connections and models are kept.

    First start: the project is added and Auto-config runs. Later starts open as you left it.

.PARAMETER Project
    The TIA project (.ap18) to use. Not given: the newest HoistGantry project under
    ...\ITTOOT\Projects\Siemens (v1.0 and later are preferred), else the saved one.

.PARAMETER Page
    Open on this page: settings, connections, tables, equipment, model, databases, documents.
    Default: model.

.PARAMETER Test
    Run the tests and clippy for the bridge crates instead of opening the app.

.PARAMETER NoSolidcraft
    Do not build Solidcraft's window (the START SOLIDCRAFT button then needs a solidcraft.exe
    path in the model's Details).

.PARAMETER Release
    Build and run the release profile.

.PARAMETER Fresh
    Move the saved settings, secrets and models aside first, to see first-run behaviour.

.PARAMETER Design
    Also start the design agent (AI Office\design-agent, http://127.0.0.1:9740) that the Design page talks to.

.PARAMETER Save
    Headless: write the deployment record for the project and stop (no window).

.EXAMPLE
    .\scripts\start-bridge.ps1
    Build what is missing, open the bridge on the Model page.

.EXAMPLE
    .\scripts\start-bridge.ps1 -Page settings -Fresh
    See the first-run Settings page.

.EXAMPLE
    .\scripts\start-bridge.ps1 -Test
#>
[CmdletBinding()]
param(
    [string] $Project = '',
    [string] $Page = 'model',
    [switch] $Test,
    [switch] $NoSolidcraft,
    [switch] $Release,
    [switch] $Fresh,
    [switch] $Save,
    [switch] $Design
)

$ErrorActionPreference = 'Stop'
# The script lives in scripts\; the workspace is one level up.
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $root

function Say([string] $text, [string] $colour = 'Gray') {
    Write-Host "  $text" -ForegroundColor $colour
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Say 'cargo was not found. Install Rust from https://rustup.rs and open a new window.' 'Red'
    exit 1
}

$profile_ = if ($Release) { 'release' } else { 'debug' }
$flag = if ($Release) { @('--release') } else { @() }
$bin = Join-Path $root "target\$profile_"

if ($Test) {
    Say 'Testing the bridge crates...' 'Cyan'
    cargo test -p ittoot-bridge -p ittoot-opcua -p ittoot-db
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    cargo clippy -p ittoot-bridge -p ittoot-opcua -p ittoot-db -p ittoot-bridge-app --all-targets -- -D warnings
    exit $LASTEXITCODE
}

# Which project: the one asked for, else the newest HoistGantry beside the other Siemens projects.
function Find-Project {
    $siemens = Join-Path (Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $root))) 'Projects\Siemens'
    if (-not (Test-Path -LiteralPath $siemens)) { return '' }
    $found = Get-ChildItem -LiteralPath $siemens -Recurse -Depth 3 -Filter '*HoistGantry*.ap18' -ErrorAction SilentlyContinue |
        Sort-Object { [version](($_.BaseName -replace '.*_v(\d+(\.\d+)*)_.*', '$1') -replace '^$', '0') } -Descending
    if ($found) { return $found[0].FullName }
    return ''
}
if (-not $Project) { $Project = Find-Project }
if ($Project -and -not (Test-Path -LiteralPath $Project)) {
    Say "Project not found: $Project" 'Red'
    exit 1
}

$data = Join-Path $env:APPDATA 'ITTOOT\Bridge'
if ($Fresh -and (Test-Path $data)) {
    $aside = "$data.old-" + (Get-Date -Format 'yyyyMMdd-HHmmss')
    Move-Item -LiteralPath $data -Destination $aside
    Say "Saved settings moved to $aside" 'Yellow'
}

# Build. Solidcraft's window is large; it is built once and left alone after that.
Say 'Building the bridge...' 'Cyan'
cargo build -p ittoot-bridge-app @flag
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$solid = Join-Path $bin 'solidcraft.exe'
if (-not $NoSolidcraft -and -not (Test-Path $solid)) {
    Say 'Building Solidcraft (first time only; this takes a long while)...' 'Cyan'
    cargo build -p solidcraft @flag
    if ($LASTEXITCODE -ne 0) { Say 'Solidcraft did not build; the bridge still opens.' 'Yellow' }
}
if (Test-Path $solid) {
    $env:ITTOOT_SOLIDCRAFT_EXE = $solid
    Say "Solidcraft: $solid" 'Green'
} else {
    Say 'No solidcraft.exe yet: set its path in the model Details, or re-run without -NoSolidcraft.' 'Yellow'
}

# The design agent, when asked for and not already answering.
if ($Design) {
    $agent = Join-Path (Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $root))) 'Projects\AI Office\design-agent'
    $up = $false
    try { $up = (Invoke-RestMethod -Uri 'http://127.0.0.1:9740/api/health' -TimeoutSec 2).ok } catch { }
    if ($up) { Say 'Design agent already running on 9740.' 'Green' }
    elseif (Test-Path (Join-Path $agent 'server.mjs')) {
        Start-Process -FilePath 'node' -ArgumentList 'server.mjs' -WorkingDirectory $agent -WindowStyle Minimized
        Say "Design agent started: $agent" 'Green'
    } else { Say "Design agent not found at $agent" 'Yellow' }
}

# Close a bridge that is already open, so the build can be replaced.
Get-Process -Name 'ittoot-bridge' -ErrorAction SilentlyContinue | ForEach-Object {
    Say 'Closing the bridge that is already open...' 'DarkGray'
    $_ | Stop-Process -Force
}

$exe = Join-Path $bin 'ittoot-bridge.exe'
$argv = @('--page', $Page.ToLower())
if ($Project) {
    $argv += @('--project', $Project)
    Say "Project: $Project" 'Green'
} else {
    Say 'No HoistGantry project found; give one with -Project, or add it on the Settings page.' 'Yellow'
}

Write-Host ''
Say 'Model page: press the orange START SOLIDCRAFT button, then "Build the model in Solidcraft".' 'DarkGray'
Say '"+" beside Models adds another Solidcraft (its own port and token).' 'DarkGray'
Write-Host ''

if ($Save) {
    & $exe @argv --save-deployment
    exit $LASTEXITCODE
}
& $exe @argv
