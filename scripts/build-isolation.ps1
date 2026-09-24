param([switch]$NoTests)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$workspace = Split-Path -Parent $repo
$toolchain = Join-Path $workspace '.toolchain'
$cache = Join-Path $workspace '.bootstrap'
$editionName = Split-Path -Leaf $repo
$packageVersion = (Get-Content -LiteralPath (Join-Path $repo 'package.json') -Raw | ConvertFrom-Json).version
New-Item -ItemType Directory -Path $toolchain,$cache,(Join-Path $repo 'build-logs') -Force | Out-Null
$log = Join-Path $repo ('build-logs\build-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.log')

function Download-Verified([string]$Url, [string]$Path, [string]$Sha256) {
    if ((Test-Path -LiteralPath $Path) -and (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -eq $Sha256) { return }
    & curl.exe -fL --retry 3 --connect-timeout 20 --max-time 240 $Url -o $Path
    if ($LASTEXITCODE -ne 0) { throw "Download failed: $Url" }
    if ((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -ne $Sha256) { throw "Checksum mismatch: $Path" }
}
function Run-Step([string]$File, [string[]]$Arguments) {
    Write-Host ('> ' + $File + ' ' + ($Arguments -join ' ')) -ForegroundColor Cyan
    # Windows PowerShell 5 wraps native stderr as error records. Preserve output without treating
    # compiler progress lines as terminating PowerShell errors; always check the actual exit code.
    $savedPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & $File @Arguments 2>&1 | ForEach-Object { $_.ToString() } | Tee-Object -FilePath $log -Append | ForEach-Object { Write-Host $_ }
    $code = $LASTEXITCODE
    $ErrorActionPreference = $savedPreference
    if ($code -ne 0) { throw "Command failed ($code): $File $($Arguments -join ' '). Close this project's dev server before building. Log: $log" }
}

Push-Location $repo
try {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (!(Test-Path -LiteralPath $vswhere)) { throw 'Install Visual Studio 2022 Build Tools with Desktop development with C++ and Windows SDK first.' }
    $vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (!$vs) { throw 'MSVC x64 tools missing. Install Desktop development with C++ in Visual Studio Installer.' }

    $nodeVersion = 'v22.23.2'
    $nodeDir = Join-Path $toolchain "node-$nodeVersion-win-x64"
    if (!(Test-Path -LiteralPath (Join-Path $nodeDir 'node.exe'))) {
        $sums = (Invoke-WebRequest -UseBasicParsing "https://nodejs.org/dist/$nodeVersion/SHASUMS256.txt").Content
        $line = ($sums -split "`n" | Where-Object { $_ -match "node-$nodeVersion-win-x64.zip$" } | Select-Object -First 1)
        if (!$line) { throw 'Official Node.js checksum missing' }
        Download-Verified "https://nodejs.org/dist/$nodeVersion/node-$nodeVersion-win-x64.zip" (Join-Path $cache 'node.zip') (($line.Trim() -split '\s+')[0])
        Expand-Archive -LiteralPath (Join-Path $cache 'node.zip') -DestinationPath $toolchain -Force
    }
    $goDir = Join-Path $toolchain 'go'
    if (!(Test-Path -LiteralPath (Join-Path $goDir 'bin\go.exe'))) {
        Download-Verified 'https://go.dev/dl/go1.27.1.windows-amd64.zip' (Join-Path $cache 'go.zip') 'a3911b5e0e1b1053f25ed0675f4c1c6aad1e2bfcf253df2b9be4caabd2edd95d'
        Expand-Archive -LiteralPath (Join-Path $cache 'go.zip') -DestinationPath $toolchain -Force
    }
    $env:CARGO_HOME = Join-Path $toolchain 'cargo'
    $env:RUSTUP_HOME = Join-Path $toolchain 'rustup'
    $env:PATH = "$nodeDir;$(Join-Path $goDir 'bin');$(Join-Path $env:CARGO_HOME 'bin');$env:PATH"
    if (!(Test-Path -LiteralPath (Join-Path $env:CARGO_HOME 'bin\rustup.exe'))) {
        $rustupUrl = 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe'
        $rustupHash = ((Invoke-WebRequest -UseBasicParsing ($rustupUrl + '.sha256')).Content.Trim() -split '\s+')[0]
        Download-Verified $rustupUrl (Join-Path $cache 'rustup-init.exe') $rustupHash
        Run-Step (Join-Path $cache 'rustup-init.exe') @('-y','--no-modify-path','--profile','minimal','--default-toolchain','1.94.0')
    }
    $pinnedRustc = Join-Path $env:RUSTUP_HOME 'toolchains\1.94.0-x86_64-pc-windows-msvc\bin\rustc.exe'
    if (!(Test-Path -LiteralPath $pinnedRustc)) {
        Run-Step 'rustup.exe' @('toolchain','install','1.94.0','--profile','minimal')
    }
    $env:CARGO_BUILD_JOBS = '2'
    $env:CARGO_PROFILE_DEV_DEBUG = '0'
    $env:GOMAXPROCS = '3'
    $env:GOMODCACHE = Join-Path $toolchain 'gomod'
    $env:GOCACHE = Join-Path $toolchain 'gocache'
    if (!$env:GOPROXY) { $env:GOPROXY = 'https://goproxy.cn,https://proxy.golang.org,direct' }
    $env:ELECTRON_SKIP_BINARY_DOWNLOAD = '1'
    # The pinned Node 22 process can terminate with 0xC0000005 during this
    # project's large TypeScript check unless V8 has an explicit heap budget.
    $env:NODE_OPTIONS = (($env:NODE_OPTIONS + ' --max-old-space-size=4096').Trim())
    $env:COCKPIT_TOOLS_ISOLATION_BUILD = '1'

    Download-Verified 'https://github.com/XTLS/Xray-core/releases/download/v26.3.27/Xray-windows-64.zip' (Join-Path $cache 'xray.zip') 'd004c39288ce9ada487c6f398c7c545f7d749e44bdfdd59dbc9f865afba4e1ad'
    $xrayExtract = Join-Path $cache 'xray-v26.3.27'
    Expand-Archive -LiteralPath (Join-Path $cache 'xray.zip') -DestinationPath $xrayExtract -Force
    New-Item -ItemType Directory -Path (Join-Path $repo 'src-tauri\proxy-core') -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $xrayExtract 'xray.exe'),(Join-Path $xrayExtract 'LICENSE') -Destination (Join-Path $repo 'src-tauri\proxy-core') -Force
    Download-Verified 'https://github.com/SagerNet/sing-box/releases/download/v1.14.1/sing-box-1.14.1-windows-amd64.zip' (Join-Path $cache 'sing-box-1.14.1-windows-amd64.zip') '5197f16d492d93202dc623622149a6ed040f8eca263128f91d603f2b901baa89'
    $singExtract = Join-Path $cache 'sing-box-v1.14.1'
    Expand-Archive -LiteralPath (Join-Path $cache 'sing-box-1.14.1-windows-amd64.zip') -DestinationPath $singExtract -Force
    Copy-Item -LiteralPath (Join-Path $singExtract 'sing-box-1.14.1-windows-amd64\sing-box.exe') -Destination (Join-Path $repo 'src-tauri\proxy-core\sing-box.exe') -Force
    Copy-Item -LiteralPath (Join-Path $singExtract 'sing-box-1.14.1-windows-amd64\LICENSE') -Destination (Join-Path $repo 'src-tauri\proxy-core\sing-box-LICENSE') -Force
    Download-Verified 'https://github.com/SagerNet/sing-box/archive/refs/tags/v1.14.1.zip' (Join-Path $cache 'sing-box-1.14.1-source.zip') 'c854475860fd536cc3c7d7f40457722b1729bde13e81f95cfd345fcec6d7000f'
    Download-Verified 'https://www.gnu.org/licenses/gpl-3.0.txt' (Join-Path $cache 'GPL-3.0.txt') '3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986'
    Copy-Item -LiteralPath (Join-Path $cache 'sing-box-1.14.1-source.zip') -Destination (Join-Path $repo 'src-tauri\proxy-core\sing-box-1.14.1-source.zip') -Force
    Copy-Item -LiteralPath (Join-Path $cache 'GPL-3.0.txt') -Destination (Join-Path $repo 'src-tauri\proxy-core\GPL-3.0.txt') -Force
    Run-Step 'npm.cmd' @('ci','--no-audit','--no-fund')
    if (!$NoTests) {
        Run-Step 'cargo.exe' @('test','--locked','-p','cockpit-account-proxy')
        Run-Step 'node.exe' @('--test','scripts\isolation-contract.test.cjs')
        Push-Location (Join-Path $repo 'sidecars\cockpit-cliproxy')
        try { Run-Step 'go.exe' @('test','-p','2','./...') } finally { Pop-Location }
    }
    $buildArguments = @('run','tauri','--','build','--bundles','nsis')
    Run-Step 'npm.cmd' $buildArguments

    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $output = Join-Path $repo "build-logs\staging-$stamp"
    $portable = Join-Path $output 'portable'
    New-Item -ItemType Directory -Path $portable,(Join-Path $portable 'proxy-core') -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $repo 'target\release\cockpit-tools.exe') -Destination $portable
    Copy-Item -LiteralPath (Join-Path $repo 'sidecars\cockpit-cliproxy\bin\cockpit-cliproxy-x86_64-pc-windows-msvc.exe') -Destination (Join-Path $portable 'cockpit-cliproxy.exe')
    Copy-Item -LiteralPath (Join-Path $repo 'src-tauri\proxy-core\xray.exe'),(Join-Path $repo 'src-tauri\proxy-core\LICENSE') -Destination (Join-Path $portable 'proxy-core')
    # Mirror every Tauri resource so the unpacked executable behaves like the installed app.
    $config = Get-Content -LiteralPath (Join-Path $repo 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
    foreach ($resource in $config.bundle.resources.PSObject.Properties) {
        $source = Join-Path (Join-Path $repo 'src-tauri') $resource.Name
        $target = Join-Path $portable $resource.Value
        New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
        Copy-Item -LiteralPath $source -Destination $target -Force
    }
    Copy-Item -LiteralPath (Join-Path $repo 'LICENSE'),(Join-Path $repo 'V4-ISOLATION-README.md'),(Join-Path $repo 'V4-START-HERE.md'),(Join-Path $repo 'V4-HANDOFF.md') -Destination $portable
    Copy-Item -LiteralPath (Join-Path $repo 'sidecars\cockpit-cliproxy\third_party\CLIProxyAPI\LICENSE') -Destination (Join-Path $portable 'CLIProxyAPI-LICENSE')
    $installers = @(Get-ChildItem -LiteralPath (Join-Path $repo 'target\release\bundle\nsis') -Filter '*.exe')
    if ($installers.Count -ne 1) { throw 'Expected exactly one NSIS setup.exe' }
    $installers | Copy-Item -Destination $output
    $report = Join-Path $output 'proxy-self-test.json'
    $testProcess = Start-Process -FilePath (Join-Path $portable 'cockpit-tools.exe') -ArgumentList @('--proxy-self-test',('"' + $report + '"')) -WindowStyle Hidden -PassThru
    if (!$testProcess.WaitForExit(60000)) { $testProcess.Kill(); throw 'Executable proxy acceptance test timed out' }
    if ($testProcess.ExitCode -ne 0 -or !(Test-Path -LiteralPath $report)) { throw "Executable proxy acceptance test failed: $report" }
    $testReport = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
    if (!$testReport.passed) { throw "Executable proxy acceptance test failed: $report" }
    # Generated build metadata, not hand-edited source files.
    Get-ChildItem -LiteralPath $output -Recurse -File | Where-Object { $_.Name -ne 'SHA256.json' } | ForEach-Object {
        [PSCustomObject]@{ File = $_.FullName.Substring($output.Length + 1); SHA256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash }
    } | ConvertTo-Json | Out-File -LiteralPath (Join-Path $output 'SHA256.json') -Encoding utf8
    $finalOutput = [System.IO.Path]::GetFullPath((Join-Path $repo "artifacts\$stamp"))
    $stagingRoot = [System.IO.Path]::GetFullPath((Join-Path $repo 'build-logs')) + '\'
    $artifactRoot = [System.IO.Path]::GetFullPath((Join-Path $repo 'artifacts')) + '\'
    if (![System.IO.Path]::GetFullPath($output).StartsWith($stagingRoot,[System.StringComparison]::OrdinalIgnoreCase) -or !$finalOutput.StartsWith($artifactRoot,[System.StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid packaging destination' }
    New-Item -ItemType Directory -Path (Join-Path $repo 'artifacts') -Force | Out-Null
    if (Test-Path -LiteralPath $finalOutput) { throw 'Artifact destination already exists; refusing to overwrite' }
    # Xray can briefly retain its executable handle after the acceptance process exits.
    # Retry only this exact staging directory; never overwrite an existing release.
    $moved = $false
    for ($attempt = 1; $attempt -le 12; $attempt++) {
        try {
            Move-Item -LiteralPath $output -Destination $finalOutput -ErrorAction Stop
            $moved = $true
            break
        } catch {
            if ((Test-Path -LiteralPath $finalOutput) -or $attempt -eq 12) { throw }
            Start-Sleep -Seconds 1
        }
    }
    if (!$moved) { throw 'Could not move release staging directory' }
    $zipPath = Join-Path $repo ("artifacts\$editionName-$packageVersion-win-x64-$stamp.zip")
    if (Test-Path -LiteralPath $zipPath) { throw '版本 ZIP 已存在，拒绝覆盖' }
    # User requests exactly one distributable ZIP containing the installer EXE.
    Get-ChildItem -LiteralPath $finalOutput -Filter '*setup.exe' | Compress-Archive -DestinationPath $zipPath -CompressionLevel Optimal
    Get-FileHash -LiteralPath $zipPath -Algorithm SHA256 | Select-Object Hash,Path | ConvertTo-Json | Out-File -LiteralPath (Join-Path $finalOutput 'ZIP-SHA256.json') -Encoding utf8
    Write-Host "SUCCESS: $zipPath" -ForegroundColor Green
} catch {
    Write-Host $_ -ForegroundColor Red
    exit 1
} finally { Pop-Location }
