# Builds Daggerfall Dungeon Maker as a release executable.
#
# Requirements:
#   * Rust (https://rustup.rs) with the x86_64-pc-windows-gnu toolchain, or the
#     MSVC toolchain with Visual Studio Build Tools installed.
#   * For the GNU toolchain, MinGW-w64 on the PATH (for example `scoop install mingw`),
#     which provides the assembler and dlltool needed by the Windows API crates.
#
# The executable is written to target\release\daggerfall-dungeon-maker.exe.

# Cargo writes progress to stderr, which Windows PowerShell treats as an error
# under "Stop", so failures are detected from the exit code instead.
$ErrorActionPreference = "Continue"

# Shells opened before Rust or MinGW were installed may not have them on the PATH yet.
$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
$scoopMingw = Join-Path $env:USERPROFILE "scoop\apps\mingw\current\bin"
foreach ($toolFolder in @($cargoBin, $scoopMingw)) {
    if ((Test-Path $toolFolder) -and -not ($env:PATH -split ";" -contains $toolFolder)) {
        $env:PATH = "$toolFolder;$env:PATH"
    }
}

cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "Built $(Join-Path $PSScriptRoot 'target\release\daggerfall-dungeon-maker.exe')"
