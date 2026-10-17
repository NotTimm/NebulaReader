# Builds the Windows packages from one staged folder (nebula-reader.exe plus
# the libarchive DLLs it needs):
#   - a portable .zip: unzip anywhere and run
#   - a per-user installer (Inno Setup, packaging/windows/nebula-reader.iss),
#     which is also what winget installs. Skipped with a warning if Inno
#     Setup isn't installed, unless REQUIRE_INSTALLER=1 (as in CI).
#
# Needs Rust (MSVC toolchain) and vcpkg with libarchive:
#   vcpkg install "libarchive[core,bzip2,lz4,lzma,zstd]:x64-windows"
# and VCPKG_ROOT pointing at the vcpkg checkout.
$ErrorActionPreference = 'Stop'

$Repo = Resolve-Path (Join-Path $PSScriptRoot '..')
$Dist = Join-Path $Repo 'dist'

if (-not $env:VCPKG_ROOT) {
    throw 'VCPKG_ROOT is not set (point it at your vcpkg checkout).'
}
$VcpkgBin = Join-Path $env:VCPKG_ROOT 'installed\x64-windows\bin'
if (-not (Test-Path (Join-Path $VcpkgBin 'archive.dll'))) {
    throw "libarchive not found in $VcpkgBin. Run: vcpkg install `"libarchive[core,bzip2,lz4,lzma,zstd]:x64-windows`""
}

# Version comes from Cargo.toml's [package] table, like the Linux scripts.
$Version = $null
$InPackage = $false
foreach ($line in Get-Content (Join-Path $Repo 'Cargo.toml')) {
    if ($line -match '^\[') { $InPackage = ($line -eq '[package]'); continue }
    if ($InPackage -and $line -match '^version\s*=\s*"([^"]+)"') { $Version = $Matches[1]; break }
}
if (-not $Version) { throw 'could not read version from Cargo.toml' }

# Link libarchive dynamically; its DLLs ship next to the exe.
$env:VCPKGRS_DYNAMIC = '1'
Write-Host '==> Building release binary'
Push-Location $Repo
try {
    $flags = @('build', '--release')
    if ($env:CARGO_BUILD_FLAGS) { $flags += $env:CARGO_BUILD_FLAGS.Split(' ') }
    cargo @flags
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
} finally {
    Pop-Location
}

$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Repo 'target' }
$Exe = Join-Path $TargetDir 'release\nebula-reader.exe'

$Name = "NebulaReader-$Version-windows-x64"
$Stage = Join-Path ([System.IO.Path]::GetTempPath()) $Name
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Path $Stage | Out-Null

Copy-Item $Exe $Stage
Copy-Item (Join-Path $VcpkgBin '*.dll') $Stage

# The exe and the vcpkg DLLs import VCRUNTIME140.dll, which a clean Windows
# doesn't have (only the separate VC++ Redistributable installs it), so the
# app would fail to start with STATUS_DLL_NOT_FOUND. Ship it app-locally,
# from Visual Studio's redist folder. The UCRT (api-ms-win-crt-*) is part of
# Windows 10+.
$Vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$VcRuntime = $null
if (Test-Path $Vswhere) {
    $Vs = & $Vswhere -latest -products * -property installationPath
    if ($Vs) {
        $VcRuntime = Get-ChildItem -Path (Join-Path $Vs 'VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT') `
            -Filter 'vcruntime140*.dll' -ErrorAction SilentlyContinue |
            Sort-Object FullName -Descending
    }
}
if (-not $VcRuntime) {
    $VcRuntime = Get-ChildItem -Path "$env:SystemRoot\System32" -Filter 'vcruntime140*.dll'
}
if (-not ($VcRuntime | Where-Object Name -eq 'vcruntime140.dll')) {
    throw 'vcruntime140.dll not found (install the Visual Studio C++ build tools)'
}
# One copy per file name, from the newest redist folder.
$VcRuntime | Group-Object Name | ForEach-Object { Copy-Item $_.Group[0].FullName $Stage }
Write-Host "==> Bundled $(($VcRuntime | Group-Object Name).Name -join ', ')"
Copy-Item (Join-Path $Repo 'LICENSE') (Join-Path $Stage 'LICENSE.txt')
@"
Nebula Reader $Version for Windows (portable)

Run nebula-reader.exe. Keep the .dll files next to it.

Your library and settings are stored in %APPDATA%\NebulaReader and the
cache in %LOCALAPPDATA%\NebulaReader\Cache, so you can replace this folder
with a newer version without losing anything.

https://github.com/NotTimm/nebula-reader
"@ | Set-Content -Encoding utf8 (Join-Path $Stage 'README.txt')

New-Item -ItemType Directory -Force -Path $Dist | Out-Null
$Zip = Join-Path $Dist "$Name.zip"
if (Test-Path $Zip) { Remove-Item $Zip }
Compress-Archive -Path $Stage -DestinationPath $Zip
Write-Host "==> $Zip"

# Installer. ISCC is Inno Setup's command-line compiler.
$Iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source
if (-not $Iscc) {
    $Iscc = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if ($Iscc) {
    # Windows file versions are numeric: 0.1.0-beta.3 -> 0.1.0
    $Numeric = ($Version -split '-')[0]
    & $Iscc /Q "/DAppVersion=$Version" "/DNumericVersion=$Numeric" `
        "/DStageDir=$Stage" "/DOutputDir=$Dist" `
        (Join-Path $PSScriptRoot 'windows\nebula-reader.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Inno Setup failed' }
    Write-Host "==> $(Join-Path $Dist "$Name-setup.exe")"
} elseif ($env:REQUIRE_INSTALLER -eq '1') {
    throw 'Inno Setup (ISCC.exe) not found; install it with: winget install JRSoftware.InnoSetup'
} else {
    Write-Warning 'Inno Setup not found; skipping the installer (winget install JRSoftware.InnoSetup)'
}

Remove-Item -Recurse -Force $Stage
