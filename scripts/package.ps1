# Builds the release and everything a GitHub release publishes, into target\dist:
#   Rustcreate-<version>-x64.msi            per-user installer
#   Rustcreate-<version>-x64-portable.exe   standalone executable
#   SHA256SUMS.txt                          checksums the in-app updater verifies (ADR 0002)
# Needs the repo's local WiX tool (`dotnet tool restore`, see dotnet-tools.json).

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# Cargo reports where it put the executable and the build script's OUT_DIR (where build.rs
# writes the icon). Asking beats guessing: the build directory layout changes between Cargo
# versions. Diagnostics still go to the console.
$messages = cargo build --release --locked -p rustcreate --message-format=json-render-diagnostics | ForEach-Object { $_ | ConvertFrom-Json }
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$app = $messages | Where-Object { $_.reason -eq "compiler-artifact" -and $_.target.name -eq "rustcreate" -and $_.executable } | Select-Object -Last 1
if (-not $app) { throw "cargo did not report the rustcreate executable" }
$exe = $app.executable
$build = $messages | Where-Object { $_.reason -eq "build-script-executed" -and $_.package_id -eq $app.package_id } | Select-Object -Last 1
if (-not $build) { throw "cargo did not report the rustcreate build script" }
$icon = Get-Item (Join-Path $build.out_dir "rustcreate.ico") -ErrorAction SilentlyContinue
if (-not $icon) { throw "rustcreate.ico not found in $($build.out_dir): did build.rs run on Windows?" }

# The workspace version, e.g. 0.1.0.
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value

$dist = Join-Path $root "target\dist"
Remove-Item $dist -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $dist | Out-Null
$msi = Join-Path $dist "Rustcreate-$version-x64.msi"
$portable = Join-Path $dist "Rustcreate-$version-x64-portable.exe"

dotnet tool restore | Out-Null
dotnet tool run wix build wix\main.wxs -arch x64 -d Version=$version -d ExePath=$exe -d IconPath=$($icon.FullName) -o $msi
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Remove-Item (Join-Path $dist "*.wixpdb") -ErrorAction SilentlyContinue
Copy-Item $exe $portable

# sha256sum format ("<hex>  <name>"), LF line endings.
$sums = Get-ChildItem $dist -File | Sort-Object Name | ForEach-Object { "$((Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower())  $($_.Name)" }
[System.IO.File]::WriteAllText((Join-Path $dist "SHA256SUMS.txt"), (($sums -join "`n") + "`n"))

Get-ChildItem $dist | Select-Object Name, Length | Format-Table -AutoSize
