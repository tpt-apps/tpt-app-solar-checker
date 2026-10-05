# TPT Solar Checker - build pipeline (Windows).
#
# Outputs (separate directories - they never overwrite each other):
#   dist\hub\      free wasm bundle for the website
#                  (solar-checker.js + solar-checker_bg.wasm)
#   dist\          trunk bundle the Pro exe serves (embedded into the exe)
#   release\TPT Solar Checker Pro\   staged exe package (-Pro)
#   release\TPT-Solar-Checker-Pro.zip  zip-ready for Gumroad (-Pro)
#
# Website upload:
#   .\build.ps1 -WebRepo C:\path\to\web-repo
#     (or set $env:TPT_WEB_REPO and just run .\build.ps1)
#   -> copies the free bundle into <web repo>\public\apps\solar-checker\
#   then set the app's registry entry in the web repo:
#     status: 'coming-soon' (flip to 'live' once the $9 Pro exe is on
#             Gumroad — apps.md §T2-18)
#     wasm: { entry: '/apps/solar-checker/solar-checker.js' }
#
# Pro edition:  .\build.ps1 -Pro
#   pro wasm (multi-scenario comparison + financing calculator + PDF
#   export), trunk bundle for the exe, and a staged+zip desktop package.

[CmdletBinding()]
param(
    # Build the Pro bundle (--features pro) instead of the free one.
    [switch]$Pro,
    # Only run the wasm-bindgen step against an existing release build.
    [switch]$SkipBuild,
    # Web repo checkout; the hub bundle is copied into it.
    # Defaults to $env:TPT_WEB_REPO.
    [string]$WebRepo = $env:TPT_WEB_REPO
)

$ErrorActionPreference = "Stop"
Set-Location -LiteralPath $PSScriptRoot

function Show-GzipSize {
    param([string]$WasmPath)
    $raw = (Get-Item $WasmPath).Length
    Add-Type -AssemblyName System.IO.Compression
    $rawBytes = [System.IO.File]::ReadAllBytes($WasmPath)
    $memoryStream = [System.IO.MemoryStream]::new()
    $gzipStream = [System.IO.Compression.GZipStream]::new($memoryStream, [System.IO.Compression.CompressionLevel]::Optimal)
    $gzipStream.Write($rawBytes, 0, $rawBytes.Length)
    $gzipStream.Dispose()
    $gzipped = $memoryStream.ToArray().Length
    $memoryStream.Dispose()
    $gzKb = [math]::Round($gzipped / 1KB, 1)
    Write-Host "-- $(Split-Path $WasmPath -Leaf): $raw bytes raw, $gzipped bytes gzipped ($gzKb KB gz)" -ForegroundColor Cyan
    if ($gzipped -gt 600KB) {
        Write-Warning "payload exceeds the ~600 KB gzip budget for first-load comfort"
    }
}

$Crate = "tpt-app-solar-checker"
$DesktopCrate = "tpt-solar-desktop"
$Target = "wasm32-unknown-unknown"
$OutName = "solar-checker"

# Validate the web repo: an existing directory with a public/ folder. A wrong
# path would scatter the bundle into an unrelated tree, so fail loudly before
# building anything.
if ($WebRepo) {
    if (-not (Test-Path -LiteralPath $WebRepo -PathType Container)) {
        throw "web repo path does not exist or is not a directory: $WebRepo"
    }
    if (-not (Test-Path -LiteralPath (Join-Path $WebRepo "public") -PathType Container)) {
        throw "web repo path has no public\ directory: $WebRepo"
    }
    $WebRepo = (Resolve-Path -LiteralPath $WebRepo).Path
}

$HubDir = Join-Path $PSScriptRoot "dist\hub"
$Dest = if ($WebRepo) { Join-Path $WebRepo "public\apps\solar-checker" } else { $HubDir }

Write-Host "== TPT Solar Checker build ==" -ForegroundColor Cyan
Write-Host " edition: $(if ($Pro) { 'pro (multi-scenario + financing + PDF export)' } else { 'free (single-property estimate)' })"
Write-Host " hub bundle:  $HubDir"
if ($WebRepo) { Write-Host " deploy to:   $Dest" }

if ($Pro) {
    if (-not $SkipBuild) {
        # The Pro trunk bundle goes into dist\ (the desktop shell's default
        # serving directory and the include_dir! source). It is a separate
        # build from the free one and deliberately never touches the
        # website's free bundle: the hub must never be built with --features
        # pro, or the site would give away every paid feature.
        if (Test-Path (Join-Path $PSScriptRoot "dist")) {
            # Clean stale hashed trunk artifacts from previous bundles.
            Remove-Item (Join-Path $PSScriptRoot "dist") -Recurse -Force
        }
        Write-Host "-- trunk build --release --features pro" -ForegroundColor Cyan
        trunk build --release --features pro
        if ($LASTEXITCODE -ne 0) { throw "trunk build failed" }
        $proWasm = Get-ChildItem (Join-Path $PSScriptRoot "dist") -Filter "*_bg.wasm" | Select-Object -First 1
        if ($proWasm) { Show-GzipSize -WasmPath $proWasm.FullName }
    }

    Write-Host "-- cargo build --release -p $DesktopCrate (webview exe)" -ForegroundColor Cyan
    cargo build --release -p $DesktopCrate
    if ($LASTEXITCODE -ne 0) { throw "desktop build failed" }
    $exe = Join-Path $PSScriptRoot "target\release\tpt-solar-checker-pro.exe"
    if (-not (Test-Path $exe)) { throw "missing $exe" }

    # Stage a portable, upload-ready package: exe + the dist it serves.
    $stageDir = Join-Path $PSScriptRoot "release\TPT Solar Checker Pro"
    New-Item -ItemType Directory -Force -Path (Join-Path $stageDir "dist") | Out-Null
    Copy-Item $exe $stageDir -Force
    Copy-Item (Join-Path $PSScriptRoot "dist\*") (Join-Path $stageDir "dist") -Recurse -Force
    Set-Content -Path (Join-Path $stageDir "README.txt") -Value @(
        "TPT Solar Checker Pro",
        "",
        "Run tpt-solar-checker-pro.exe.",
        "Requires Windows 10+ with the WebView2 runtime",
        "(preinstalled on Windows 10 20H2+ and Windows 11)."
    )
    $zip = Join-Path $PSScriptRoot "release\TPT-Solar-Checker-Pro.zip"
    Compress-Archive -Path $stageDir -DestinationPath $zip -Force
    Write-Host "   package: $stageDir" -ForegroundColor Cyan
    Write-Host "   zip:     $zip" -ForegroundColor Cyan
} else {
    if (-not $SkipBuild) {
        Write-Host "-- cargo build --release --target $Target" -ForegroundColor Cyan
        cargo build --release --target $Target -p $Crate
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    }

    $Wasm = Join-Path $PSScriptRoot "target\$Target\release\tpt_app_solar_checker.wasm"
    if (-not (Test-Path $Wasm)) { throw "missing $Wasm - run the cargo build step first" }

    # The hub bundle is always written to dist\hub (never clobbered by trunk),
    # then copied into the web repo when one is configured.
    Write-Host "-- wasm-bindgen --target web --out-name $OutName -> dist\hub" -ForegroundColor Cyan
    New-Item -ItemType Directory -Force -Path $HubDir | Out-Null
    wasm-bindgen --target web --out-name $OutName --out-dir $HubDir $Wasm
    if ($LASTEXITCODE -ne 0) { throw "wasm-bindgen failed" }

    $Glue = Join-Path $HubDir "$OutName.js"
    $BgWasm = Join-Path $HubDir "$($OutName)_bg.wasm"
    foreach ($artifact in @($Glue, $BgWasm)) {
        if (-not (Test-Path $artifact)) { throw "wasm-bindgen did not produce $artifact" }
    }

    if ($WebRepo) {
        Write-Host "-- copying hub bundle into $Dest" -ForegroundColor Cyan
        New-Item -ItemType Directory -Force -Path $Dest | Out-Null
        Copy-Item $Glue, $BgWasm -Destination $Dest -Force
    }

    # Payload size report (Phase 4 budget: < ~600 KB gzipped).
    Show-GzipSize -WasmPath $BgWasm
}

Write-Host "-- done" -ForegroundColor Green
if (-not $Pro) {
    Write-Host "   website upload: dist\hub\ holds the FREE bundle - copy its two files into the"
    Write-Host "   web repo's public\apps\solar-checker\ (or rerun with -WebRepo <path>)."
} else {
    Write-Host "   (dist\ now holds the Pro trunk bundle the exe serves; dist\hub, if present,"
    Write-Host "   was cleaned - rerun the free build to regenerate the website bundle.)"
}
