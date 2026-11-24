# nooma installer for Windows:
#   irm https://raw.githubusercontent.com/lacodda/nooma/main/tools/install.ps1 | iex
$ErrorActionPreference = "Stop"

$repo = "lacodda/nooma"

# The tag comes from the /releases/latest redirect rather than the REST API:
# unauthenticated API calls are capped at 60 per hour per IP, and an installer
# that fails because someone else on the same address ran it is no installer.
$tag = $env:NOOMA_VERSION
if (-not $tag) {
    $request = [Net.HttpWebRequest]::Create("https://github.com/$repo/releases/latest")
    $request.AllowAutoRedirect = $false
    $request.UserAgent = "nooma-installer"
    try {
        $response = $request.GetResponse()
        $tag = ($response.Headers["Location"] -split "/")[-1]
        $response.Close()
    } catch {
        throw "Cannot resolve the latest release of ${repo}: $($_.Exception.Message)"
    }
}
if (-not $tag -or $tag -notmatch '^v\d') {
    throw "Cannot resolve the latest release of $repo - set `$env:NOOMA_VERSION to a release tag (vX.Y.Z)"
}

$name = "nooma-$tag-x86_64-pc-windows-msvc"
$url = "https://github.com/$repo/releases/download/$tag/$name.zip"
$dir = if ($env:NOOMA_INSTALL_DIR) { $env:NOOMA_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\nooma" }
$tmp = Join-Path ([IO.Path]::GetTempPath()) "nooma-install-$([guid]::NewGuid())"
New-Item -ItemType Directory -Force $tmp | Out-Null

$target = Join-Path $dir "nooma.exe"
try {
    Write-Host "Downloading $url"
    Invoke-WebRequest $url -OutFile (Join-Path $tmp "nooma.zip")
    # tar by its full path, not Expand-Archive: Windows PowerShell started
    # from PowerShell 7 inherits its module path, loads that Archive module,
    # and Expand-Archive then fails on an overload .NET Framework lacks - a
    # release's install step stopped there. Windows has carried bsdtar, which
    # reads a zip, since 10 1803; a bare `tar` may be some other one on PATH.
    & (Join-Path $env:SystemRoot "System32\tar.exe") -xf (Join-Path $tmp "nooma.zip") -C $tmp
    if ($LASTEXITCODE -ne 0) { throw "tar could not unpack nooma.zip" }
    New-Item -ItemType Directory -Force $dir | Out-Null
    # A running nooma.exe cannot be overwritten, but it can be renamed: the
    # old one steps aside, so an update does not fail while the program runs.
    # It is deleted below, or by the next install once it no longer runs.
    if (Test-Path $target) {
        Move-Item $target "$target.old-$([guid]::NewGuid())" -Force
    }
    Copy-Item (Join-Path $tmp "$name\nooma.exe") $target -Force
} finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
Get-ChildItem $dir -Filter "nooma.exe.old-*" -ErrorAction SilentlyContinue |
    Remove-Item -Force -ErrorAction SilentlyContinue

# $env:NOOMA_NO_PATH leaves the PATH alone: an install into a scratch
# directory - the release check that proves the installer works - must not
# add that directory to the user's PATH.
if ($env:NOOMA_NO_PATH) {
    Write-Host "Installed nooma $tag to $target"
    return
}

# The PATH is edited through the registry rather than with
# [Environment]::SetEnvironmentVariable, which stores the value as a plain
# string. A user PATH is nearly always REG_EXPAND_SZ, because entries like
# `%JAVA_HOME%\bin` are written unexpanded - demoting the type would leave
# every one of them as literal text.
$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $true)
try {
    $userPath = $key.GetValue("Path", "", [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    if (($userPath -split ";" | Where-Object { $_ }) -notcontains $dir) {
        $updated = if ($userPath) { "$userPath;$dir" } else { $dir }
        $kind = [Microsoft.Win32.RegistryValueKind]::ExpandString
        if ($userPath) {
            try {
                if ($key.GetValueKind("Path") -eq [Microsoft.Win32.RegistryValueKind]::String) {
                    $kind = [Microsoft.Win32.RegistryValueKind]::String
                }
            } catch {
                # Leave it expandable.
            }
        }
        $key.SetValue("Path", $updated, $kind)
        Write-Host "Added $dir to your user PATH - restart the terminal to pick it up."
    }
} finally {
    $key.Close()
}

Write-Host "Installed nooma $tag to $target"
