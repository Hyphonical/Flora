# Installs the latest Flora release binary for Windows from GitHub Releases and adds it to PATH.
$ErrorActionPreference = 'Stop'
$releases = 'https://github.com/Hyphonical/Flora/releases/latest/download'

switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { $arch = 'x86_64' }
    'ARM64' { $arch = 'aarch64' }
    default { throw 'Unsupported architecture' }
}
$dir = Join-Path $env:LOCALAPPDATA 'Flora'
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$temp = Join-Path $dir ('flora-' + [guid]::NewGuid().ToString() + '.tmp')
try {
    Invoke-WebRequest -UseBasicParsing -Uri "$releases/flora-windows-$arch.exe" -OutFile $temp
    Move-Item -Force $temp (Join-Path $dir 'flora.exe')
} finally {
    if (Test-Path $temp) { Remove-Item -Force $temp }
}

$path = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not $path) { $path = '' }
if (($path -split ';') -notcontains $dir) {
    [Environment]::SetEnvironmentVariable('Path', (($path.TrimEnd(';') + ';' + $dir).TrimStart(';')), 'User')
}
if (($env:Path -split ';') -notcontains $dir) { $env:Path += ';' + $dir }
Write-Host "Installed $(Join-Path $dir 'flora.exe')"
