# Installs the latest Lucid release for Windows x86-64.
#
#   irm https://lucidlang.dev/install.ps1 | iex
#
# It reads the release list at https://lucidlang.dev/releases.txt,
# downloads the archive over HTTPS, checks its SHA-256 against the list,
# and puts lucid.exe in %LOCALAPPDATA%\Lucid\bin. It needs no
# administrator rights, runs nothing it downloads, and changes no
# environment variable. Run it again to update.
#
# LUCID_HOME changes the install directory from %LOCALAPPDATA%\Lucid.
# LUCID_RELEASES changes where the release list is read from.
#
# Errors throw rather than call exit, so `irm ... | iex` does not close
# your shell.

function Install-Lucid {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $listUrl = if ($env:LUCID_RELEASES) { $env:LUCID_RELEASES } else { 'https://lucidlang.dev/releases.txt' }
    $home_ = if ($env:LUCID_HOME) { $env:LUCID_HOME } else { Join-Path $env:LOCALAPPDATA 'Lucid' }
    $bin = Join-Path $home_ 'bin'
    $exe = Join-Path $bin 'lucid.exe'

    if ($env:OS -ne 'Windows_NT') {
        throw 'lucid install: this script is for Windows. On Linux, run: curl -fsSL https://lucidlang.dev/install.sh | sh'
    }
    $arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
    if ($arch -ne 'AMD64') {
        throw "lucid install: Lucid has binaries for x86-64 only, and this machine is $arch."
    }

    try {
        $list = (Invoke-WebRequest -UseBasicParsing -Uri $listUrl).Content
    } catch {
        throw "lucid install: cannot read the release list at $listUrl"
    }
    if ($list -is [byte[]]) { $list = [Text.Encoding]::UTF8.GetString($list) }
    $line = $list -split "`n" |
        ForEach-Object { $_.Trim() } |
        Where-Object { $_ -and -not $_.StartsWith('#') } |
        Where-Object { ($_ -split '\s+')[1] -eq 'windows-x86_64' } |
        Select-Object -First 1
    if (-not $line) { throw 'lucid install: the release list has no Windows x86-64 build.' }
    $version, $platform, $url, $sum = $line -split '\s+'
    if (-not $url.StartsWith('https://')) {
        throw "lucid install: the release list gives a download that is not HTTPS: $url"
    }
    if ($sum -notmatch '^[0-9a-f]{64}$') {
        throw "lucid install: the release list gives a bad SHA-256 for $version."
    }

    if (Test-Path $exe) {
        $current = (& $exe --version 2>$null) -join ''
        if ($current -eq "lucid $version") {
            Write-Host "Lucid $version is already installed in $bin."
            return
        }
    }

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('lucid-' + [Guid]::NewGuid())
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        $zip = Join-Path $tmp 'lucid.zip'
        Write-Host "Downloading Lucid $version..."
        try {
            Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $zip
        } catch {
            throw "lucid install: the download failed: $url"
        }
        $got = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash.ToLower()
        if ($got -ne $sum) {
            throw "lucid install: the download's SHA-256 is $got, but the release list says $sum. Nothing was installed."
        }

        $x = Join-Path $tmp 'x'
        Expand-Archive -Path $zip -DestinationPath $x
        $new = Join-Path $x 'lucid.exe'
        if (-not (Test-Path $new)) { throw 'lucid install: the archive has no lucid.exe.' }
        New-Item -ItemType Directory -Force -Path $bin | Out-Null
        try {
            Copy-Item -Force -Path $new -Destination $exe
        } catch {
            throw "lucid install: cannot write $exe. Close any running lucid.exe and try again."
        }
    } finally {
        Remove-Item -Recurse -Force -Path $tmp -ErrorAction SilentlyContinue
    }

    Write-Host "Installed $((& $exe --version) -join '') in $bin."
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $onPath = ($env:Path -split ';') + ($userPath -split ';') | Where-Object { $_.TrimEnd('\') -ieq $bin.TrimEnd('\') }
    if (-not $onPath) {
        Write-Host ''
        Write-Host "$bin is not on your PATH. To add it for your user, run:"
        Write-Host ''
        Write-Host "    [Environment]::SetEnvironmentVariable('Path', [Environment]::GetEnvironmentVariable('Path', 'User') + ';$bin', 'User')"
        Write-Host ''
        Write-Host 'and open a new terminal.'
    }
    Write-Host ''
    Write-Host 'lucid build links with the MSVC linker, from Visual Studio Build Tools with the Windows SDK.'
}

Install-Lucid
