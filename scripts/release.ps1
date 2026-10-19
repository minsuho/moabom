[CmdletBinding()]
param([switch]$SkipBundle)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root 'artifacts\release'
New-Item -ItemType Directory -Force $out | Out-Null
$oldFlags = $env:CARGO_ENCODED_RUSTFLAGS
$oldRustFlags = $env:RUSTFLAGS
$oldTargetDir = $env:CARGO_TARGET_DIR
Push-Location $root
try {
    if ($oldRustFlags -and -not $oldFlags) {
        throw 'RUSTFLAGS is set. Use CARGO_ENCODED_RUSTFLAGS instead so flags can be preserved without unsafe splitting.'
    }
    $flags = [System.Collections.Generic.List[string]]::new()
    if ($oldFlags) { foreach ($flag in $oldFlags.Split([char]31)) { $flags.Add($flag) } }
    # Source paths in panic locations/file! are compile-time metadata; no runtime decoder.
    $flags.Add("--remap-path-prefix=$root=/app")
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
    $rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE '.rustup' }
    $flags.Add("--remap-path-prefix=$cargoHome=/deps")
    $flags.Add("--remap-path-prefix=$rustupHome=/toolchain")
    $sourceIndex = 0
    foreach ($file in Get-ChildItem -LiteralPath (Join-Path $root 'src-tauri\src') -Recurse -Filter '*.rs' | Sort-Object FullName) {
        $alias = "/r/$sourceIndex.rs"
        $sourceIndex++
        $relative = $file.FullName.Substring((Join-Path $root 'src-tauri').Length + 1)
        $flags.Add("--remap-path-prefix=$($file.FullName)=$alias")
        $flags.Add("--remap-path-prefix=$relative=$alias")
        $flags.Add("--remap-path-prefix=$($relative.Replace('\', '/'))=$alias")
    }
    $env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]31
    # Build and package the same known output tree, even in a customized shell.
    $env:CARGO_TARGET_DIR = Join-Path $root 'src-tauri\target'
    npm ci --ignore-scripts --no-audit --no-fund
    if ($LASTEXITCODE) { throw 'npm ci failed' }
    npm test
    if ($LASTEXITCODE) { throw 'Asset compatibility tests failed' }
    $config = Join-Path $out 'local-bundle-config.json'
    $signed = [bool]$env:TAURI_SIGNING_PRIVATE_KEY
    # Keep the existing updater public key; a missing private key only disables local .sig generation.
    $override = @{ bundle = @{ createUpdaterArtifacts = $signed } } | ConvertTo-Json -Depth 5
    [IO.File]::WriteAllText($config, $override, [Text.UTF8Encoding]::new($false))
    Push-Location (Join-Path $root 'src-tauri')
    try {
        $buildArgs = @('tauri', 'build', '--config', $config)
        if ($SkipBundle) { $buildArgs += '--no-bundle' }
        $buildArgs += @('--', '--locked')
        & cargo @buildArgs
        if ($LASTEXITCODE) { throw 'Release build failed' }
    } finally { Pop-Location }
    python scripts/audit-release.py src-tauri/target/release/moabom.exe --enforce
    if ($LASTEXITCODE) { throw 'Release binary metadata audit failed' }
    $version = (Get-Content -Raw -Encoding UTF8 -LiteralPath 'src-tauri\tauri.conf.json' | ConvertFrom-Json).version
    $portable = Join-Path $out "Moabom-$version-portable"
    # Start from a clean generated staging directory so stale PDBs/maps cannot leak.
    if (Test-Path -LiteralPath $portable) {
        $resolved = (Resolve-Path -LiteralPath $portable).Path
        if (-not $resolved.StartsWith($out + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid output path' }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
    New-Item -ItemType Directory -Force $portable | Out-Null
    Copy-Item -LiteralPath 'src-tauri\target\release\moabom.exe' -Destination (Join-Path $portable 'Moabom.exe') -Force
    $extensions = Join-Path $portable 'extensions'
    Copy-Item -Recurse -LiteralPath 'src-tauri\extensions' -Destination $extensions
    Copy-Item -LiteralPath 'LICENSE', 'THIRD_PARTY.md' -Destination $portable -Force
    $zip = Join-Path $out "moabom_${version}_x64-portable.zip"
    Compress-Archive -Path (Join-Path $portable '*') -DestinationPath $zip -Force
    $files = @($zip, (Join-Path $portable 'Moabom.exe'))
    if (-not $SkipBundle) {
        $setup = Get-Item -LiteralPath "src-tauri\target\release\bundle\nsis\moabom_${version}_x64-setup.exe"
        Copy-Item -LiteralPath $setup.FullName -Destination $out -Force
        $files += Join-Path $out $setup.Name
        if ($signed) { Copy-Item -LiteralPath ($setup.FullName + '.sig') -Destination $out -Force }
    }
    $hashes = foreach ($file in $files) {
        $hash = Get-FileHash -LiteralPath $file -Algorithm SHA256
        "$($hash.Hash.ToLowerInvariant())  $([IO.Path]::GetFileName($file))"
    }
    [IO.File]::WriteAllLines((Join-Path $out 'SHA256SUMS.txt'), $hashes, [Text.UTF8Encoding]::new($false))
    Write-Output "Release artifacts: $out (updater signatures: $signed)"
} finally {
    $env:CARGO_ENCODED_RUSTFLAGS = $oldFlags
    $env:RUSTFLAGS = $oldRustFlags
    $env:CARGO_TARGET_DIR = $oldTargetDir
    Pop-Location
}
exit 0
