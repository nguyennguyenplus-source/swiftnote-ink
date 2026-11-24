[CmdletBinding()]
param(
    [ValidateSet("auto", "x64", "arm64")]
    [string]$Arch = "auto",
    [string]$Version = "latest"
)

$ErrorActionPreference = "Stop"

if ($Arch -eq "auto") {
    $procArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
    switch ($procArch) {
        "X64"   { $Arch = "x64" }
        "Arm64" { $Arch = "arm64" }
        default { throw "Unsupported Windows architecture: $procArch. Pass -Arch x64 or arm64 explicitly." }
    }
}

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$ArchiveName = "pdfium-win-$Arch.tgz"
if ($Version -eq "latest") {
    $DownloadUrl = "https://github.com/bblanchon/pdfium-binaries/releases/latest/download/$ArchiveName"
} else {
    $DownloadUrl = "https://github.com/bblanchon/pdfium-binaries/releases/download/$Version/$ArchiveName"
}

$TempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("swiftnote-pdfium-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $TempDir | Out-Null

try {
    $ArchivePath = Join-Path $TempDir $ArchiveName
    $ExtractDir = Join-Path $TempDir "extract"
    New-Item -ItemType Directory -Path $ExtractDir | Out-Null

    Write-Host "Downloading $DownloadUrl ..."
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $ArchivePath -UseBasicParsing

    Write-Host "Extracting $ArchiveName ..."
    & tar.exe -xzf $ArchivePath -C $ExtractDir
    if ($LASTEXITCODE -ne 0) {
        throw "tar.exe extraction failed with exit code $LASTEXITCODE"
    }

    $DllFile = Get-ChildItem -Path $ExtractDir -Recurse -Filter "pdfium.dll" | Select-Object -First 1
    if (-not $DllFile) {
        throw "pdfium.dll not found inside extracted archive."
    }

    $RepoDll = Join-Path $RepoRoot "pdfium.dll"
    Copy-Item -Path $DllFile.FullName -Destination $RepoDll -Force
    Write-Host "Installed $RepoDll"

    foreach ($Profile in @("debug", "release")) {
        $TargetDir = Join-Path $RepoRoot "target\$Profile"
        if (Test-Path $TargetDir) {
            $TargetDll = Join-Path $TargetDir "pdfium.dll"
            Copy-Item -Path $DllFile.FullName -Destination $TargetDll -Force
            Write-Host "Copied $TargetDll"
        }
    }
} finally {
    if (Test-Path $TempDir) {
        Remove-Item -Path $TempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
