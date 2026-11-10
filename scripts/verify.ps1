param(
    [switch]$Package,
    [switch]$LiveMedia,
    [switch]$LiveLyrics,
    [switch]$LiveTranslation
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot

function Assert-Succeeded([string]$Step) {
    if ($LASTEXITCODE -ne 0) { throw "$Step failed (exit $LASTEXITCODE)" }
}

Push-Location -LiteralPath $projectRoot
try {
    & npm.cmd test
    Assert-Succeeded 'Frontend regression tests'
    & npm.cmd run build
    Assert-Succeeded 'TypeScript and production frontend'
    Push-Location -LiteralPath (Join-Path $projectRoot 'src-tauri')
    try {
        & cargo fmt --check
        Assert-Succeeded 'Rust formatting'
        & cargo clippy --offline --all-targets -- -D warnings
        Assert-Succeeded 'Rust static analysis'
        & cargo test --offline
        Assert-Succeeded 'Rust regression tests'
        if ($LiveMedia) {
            & cargo test --offline inspect_live_windows_media_sessions -- --ignored --nocapture
            Assert-Succeeded 'Live media inspection'
        }
        if ($LiveLyrics) {
            & cargo test --offline inspect_live_lyrics_providers -- --ignored --nocapture
            Assert-Succeeded 'Live lyrics inspection'
        }
        if ($LiveTranslation) {
            Write-Host 'Live translation uses the saved API Key and consumes a small number of tokens.'
            & cargo test --offline inspect_live_small_analysis_batch -- --ignored --nocapture
            Assert-Succeeded 'Live translation inspection'
        }
    }
    finally { Pop-Location }
    if ($Package) {
        & npm.cmd run tauri build -- --bundles nsis
        Assert-Succeeded 'Windows NSIS package'
    }
}
finally { Pop-Location }
