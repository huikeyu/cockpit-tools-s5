param([string]$OutputName)
$ErrorActionPreference = 'Stop'

$repo = [System.IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$folderName = 'cockpit-tools-s5-v4'
$stageRoot = [System.IO.Path]::GetFullPath((Join-Path $repo "build-logs\public-source-$stamp"))
$sourceRoot = Join-Path $stageRoot $folderName
$artifactRoot = [System.IO.Path]::GetFullPath((Join-Path $repo 'artifacts'))
if (!$OutputName) { $OutputName = "cockpit-tools-s5-v4-source-$stamp.zip" }
if ($OutputName -notmatch '^cockpit-tools-s5-v4-source-[0-9]{8}-[0-9]{6}\.zip$') {
    throw 'Invalid public source archive name'
}
$output = [System.IO.Path]::GetFullPath((Join-Path $artifactRoot $OutputName))
if (!$stageRoot.StartsWith(([System.IO.Path]::GetFullPath((Join-Path $repo 'build-logs')) + '\'), [System.StringComparison]::OrdinalIgnoreCase) -or
    !$output.StartsWith(($artifactRoot + '\'), [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'Public source paths escaped the V4 workspace'
}
if ((Test-Path -LiteralPath $stageRoot) -or (Test-Path -LiteralPath $output)) {
    throw 'Public source staging directory or archive already exists'
}
New-Item -ItemType Directory -Path $sourceRoot,$artifactRoot -Force | Out-Null

# Mechanical source-tree copy: no local toolchain, compiled outputs, runtime data, or internal handoff.
& robocopy.exe $repo $sourceRoot /E /XD .git node_modules target dist artifacts build-logs .tmp (Join-Path $repo 'V2') /XF *.exe *.dll *.msi *.pfx *.p12 *.key *.db *.sqlite *.sqlite3 *.jsonl *.log FIRST-MESSAGE.txt V4-HANDOFF.md build-clean-handoff.ps1 /NFL /NDL /NJH /NJS | Out-Null
if ($LASTEXITCODE -gt 7) { throw "Source copy failed ($LASTEXITCODE)" }

$sourceFiles = @(Get-ChildItem -LiteralPath $sourceRoot -Recurse -File)
if ($sourceFiles.Count -lt 2000) { throw 'Source archive file count is unexpectedly small' }
$reparse = @(Get-ChildItem -LiteralPath $sourceRoot -Recurse -Attributes ReparsePoint -ErrorAction SilentlyContinue)
if ($reparse.Count) { throw 'Source staging contains symlinks or junctions' }

$envFile = Join-Path $sourceRoot '.env'
$expectedEnv = 'VITE_COCKPIT_ISOLATION=1' + [char]10 + 'VITE_COCKPIT_TOOLS_PROFILE=isolation-v4'
if (!(Test-Path -LiteralPath $envFile) -or
    [System.IO.File]::ReadAllText($envFile).Trim().Replace([Environment]::NewLine, [string][char]10) -ne $expectedEnv) {
    throw 'Source .env does not match the reviewed public configuration'
}

$privateRoots = @($env:USERPROFILE, $repo, (Split-Path -Parent $repo)) | Where-Object { $_ -and $_.Length -gt 4 }
$textExtensions = @('.rs','.go','.ts','.tsx','.js','.cjs','.mjs','.json','.toml','.md','.txt','.css','.html','.yml','.yaml','.ps1','.cmd','.mod','.sum','.rb')
foreach ($file in $sourceFiles) {
    if ($file.Name -match '^(auth\.json|config\.toml|.*\.env\.local)$') { throw "Runtime credential/config file found: $($file.FullName)" }
    if ($textExtensions -notcontains $file.Extension.ToLowerInvariant()) { continue }
    $content = [System.IO.File]::ReadAllText($file.FullName)
    foreach ($privateRoot in $privateRoots) {
        if ($content.IndexOf($privateRoot, [System.StringComparison]::OrdinalIgnoreCase) -ge 0 -or
            $content.IndexOf($privateRoot.Replace('\','/'), [System.StringComparison]::OrdinalIgnoreCase) -ge 0) {
            throw "Personal absolute path found: $($file.FullName)"
        }
    }
}

$forbidden = @(Get-ChildItem -LiteralPath $sourceRoot -Recurse -File | Where-Object {
    $_.Extension.ToLowerInvariant() -in @('.exe','.dll','.msi','.pfx','.p12','.key','.db','.sqlite','.sqlite3','.jsonl','.log')
})
if ($forbidden.Count) { throw 'Compiled, secret, or runtime files found in source staging' }

Compress-Archive -LiteralPath $sourceRoot -DestinationPath $output -CompressionLevel Optimal
Add-Type -AssemblyName System.IO.Compression
$zip = [System.IO.Compression.ZipFile]::OpenRead($output)
try {
    $names = @($zip.Entries | ForEach-Object { $_.FullName -replace '\\','/' })
    if ($names.Count -lt 2000 -or
        !($names -contains "$folderName/package.json") -or
        !($names -contains "$folderName/Cargo.lock") -or
        !($names -contains "$folderName/SOURCE-BUILD.md") -or
        !($names -contains "$folderName/src-tauri/proxy-core/sing-box-1.14.1-source.zip")) {
        throw 'Public source ZIP is missing required source files'
    }
} finally {
    $zip.Dispose()
}
Write-Host "PUBLIC SOURCE: $output ($($sourceFiles.Count) files)" -ForegroundColor Green
Get-FileHash -LiteralPath $output -Algorithm SHA256 | Select-Object Path,Hash
