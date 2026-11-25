$ErrorActionPreference = 'Stop'

$required = @{
    AZURE_CLIENT_ID = 'WINDOWS_SIGNING_CLIENT_ID'
    AZURE_TENANT_ID = 'WINDOWS_SIGNING_TENANT_ID'
    AZURE_TRUSTED_SIGNING_ENDPOINT = 'WINDOWS_SIGNING_ENDPOINT'
    AZURE_TRUSTED_SIGNING_ACCOUNT_NAME = 'WINDOWS_SIGNING_ACCOUNT_NAME'
    AZURE_TRUSTED_SIGNING_CERTIFICATE_PROFILE_NAME = 'WINDOWS_SIGNING_CERTIFICATE_PROFILE_NAME'
}
foreach ($entry in $required.GetEnumerator()) {
    if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($entry.Key))) {
        throw "Set the GitHub Actions repository variable $($entry.Value) before publishing a Windows release."
    }
}
foreach ($name in @('RUNNER_TEMP', 'GITHUB_OUTPUT', 'GITHUB_ENV', 'ACTIONS_ID_TOKEN_REQUEST_URL', 'ACTIONS_ID_TOKEN_REQUEST_TOKEN')) {
    if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
        throw "Windows signing requires $name. Grant id-token: write through every calling workflow."
    }
}

# Match the official Azure action's .NET 8-compatible toolchain. Install its
# dependencies before requesting the short-lived OIDC assertion in sign-windows.
Install-Module -Name ArtifactSigning -RequiredVersion 0.1.20 -Repository PSGallery -Scope CurrentUser -Force -AllowClobber
$module = Import-Module ArtifactSigning -RequiredVersion 0.1.20 -PassThru -ErrorAction Stop
& $module { Get-EveryDependency } | Out-Null

$failurePath = Join-Path $env:RUNNER_TEMP ('windows-signing-failed-' + [guid]::NewGuid() + '.txt')
Add-Content -LiteralPath $env:GITHUB_ENV -Value "FLOW_LIKE_WINDOWS_SIGNING_FAILURE_FILE=$failurePath" -Encoding utf8

$configPath = Join-Path $env:RUNNER_TEMP 'tauri.windows-signing.conf.json'
$config = @{
    bundle = @{
        windows = @{
            signCommand = @{
                cmd = (Get-Process -Id $PID).Path
                args = @(
                    '-NoProfile', '-NonInteractive', '-File',
                    (Join-Path $PSScriptRoot 'sign-windows.ps1'), '%1'
                )
            }
        }
    }
}
$config | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $configPath -Encoding utf8
Add-Content -LiteralPath $env:GITHUB_OUTPUT -Value "args=--config `"$configPath`"" -Encoding utf8
