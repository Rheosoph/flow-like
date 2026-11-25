$ErrorActionPreference = 'Stop'
$signer = Join-Path $PSScriptRoot '../sign-windows.ps1'
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) "flow-like signing tests-$([Guid]::NewGuid())"
$settings = @{
    AZURE_CLIENT_ID = 'test-client'
    AZURE_TENANT_ID = 'test-tenant'
    AZURE_TRUSTED_SIGNING_ENDPOINT = 'https://weu.codesigning.azure.net'
    AZURE_TRUSTED_SIGNING_ACCOUNT_NAME = 'test-account'
    AZURE_TRUSTED_SIGNING_CERTIFICATE_PROFILE_NAME = 'test-profile'
    ACTIONS_ID_TOKEN_REQUEST_URL = 'https://example.invalid/oidc?request=test'
    ACTIONS_ID_TOKEN_REQUEST_TOKEN = 'mock-request-token'
    FLOW_LIKE_WINDOWS_SIGNING_FAILURE_FILE = Join-Path $fixtureRoot 'signing-failed'
}
$originalEnvironment = @{}
foreach ($name in @($settings.Keys) + @('AZURE_FEDERATED_TOKEN_FILE')) {
    $originalEnvironment[$name] = [Environment]::GetEnvironmentVariable($name)
}

$previousTestState = Get-Variable -Name FlowLikeWindowsSigningTestState -Scope Global -ErrorAction SilentlyContinue
$global:FlowLikeWindowsSigningTestState = @{
    tokenRequests = 0
    signingCalls = 0
    tokenPaths = [Collections.Generic.List[string]]::new()
    catalogPaths = [Collections.Generic.List[string]]::new()
    tokens = [Collections.Generic.List[string]]::new()
    signatureStatus = 'Valid'
    timestamp = [PSCustomObject]@{ Subject = 'Test timestamp' }
    failSigning = $false
}

function Assert-True([bool] $Condition, [string] $Message) {
    if (-not $Condition) { throw $Message }
}

function Assert-Throws([scriptblock] $Action, [string] $Pattern = '.') {
    $caught = $null
    try { & $Action } catch { $caught = $_ }
    Assert-True ($null -ne $caught) 'Expected the signer to reject this invocation.'
    Assert-True ($caught.Exception.Message -match $Pattern) "Unexpected failure: $($caught.Exception.Message)"
}

# Mock all network, module loading, and Windows-specific signing operations.
function Import-Module {
    [CmdletBinding()]
    param([string] $Name, [string] $RequiredVersion)
    Assert-True ($Name -eq 'ArtifactSigning') 'Unexpected signing module.'
    Assert-True ($RequiredVersion -eq '0.1.20') 'Signing module version must be pinned.'
}

function Invoke-RestMethod {
    [CmdletBinding()]
    param([string] $Uri, [hashtable] $Headers, [string] $Method, [int] $TimeoutSec)
    Assert-True ($Headers.Authorization -eq 'Bearer mock-request-token') 'OIDC request authentication is missing.'
    Assert-True ([Uri]::UnescapeDataString($Uri) -match '[?&]audience=api://AzureADTokenExchange(?:&|$)') 'OIDC audience is incorrect.'
    $global:FlowLikeWindowsSigningTestState.tokenRequests++
    return @{ value = "mock.oidc.token-$($global:FlowLikeWindowsSigningTestState.tokenRequests)" }
}

function Invoke-ArtifactSigning {
    # Capture splatted named arguments without depending on a Windows module.
    $parameters = @{}
    for ($index = 0; $index -lt $args.Count; $index += 2) {
        $parameters[([string] $args[$index]).TrimStart('-').TrimEnd(':')] = $args[$index + 1]
    }
    $global:FlowLikeWindowsSigningTestState.signingCalls++
    $global:FlowLikeWindowsSigningTestState.lastParameters = $parameters
    $catalogPath = $parameters.FilesCatalog
    Assert-True (Test-Path -LiteralPath $catalogPath -PathType Leaf) 'Signing catalog is missing.'
    $global:FlowLikeWindowsSigningTestState.catalogPaths.Add($catalogPath)
    $global:FlowLikeWindowsSigningTestState.lastSignedPath = Join-Path (Split-Path -Parent $catalogPath) ([IO.File]::ReadAllText($catalogPath))
    Assert-True (Test-Path -LiteralPath $global:FlowLikeWindowsSigningTestState.lastSignedPath -PathType Leaf) 'Signing catalog did not resolve to the input file.'
    $tokenPath = $env:AZURE_FEDERATED_TOKEN_FILE
    Assert-True (Test-Path -LiteralPath $tokenPath -PathType Leaf) 'Federated token file is missing while signing.'
    $global:FlowLikeWindowsSigningTestState.tokenPaths.Add($tokenPath)
    $global:FlowLikeWindowsSigningTestState.tokens.Add([IO.File]::ReadAllText($tokenPath))
    Assert-True ($global:FlowLikeWindowsSigningTestState.tokens[-1] -eq "mock.oidc.token-$($global:FlowLikeWindowsSigningTestState.tokenRequests)") 'Signing used an old or malformed token.'
    Assert-True (-not $parameters.ExcludeWorkloadIdentityCredential) 'Workload identity authentication was disabled.'
    foreach ($credential in @('Environment', 'ManagedIdentity', 'SharedTokenCache', 'VisualStudio', 'VisualStudioCode', 'AzureCli', 'AzurePowerShell', 'AzureDeveloperCli', 'InteractiveBrowser')) {
        Assert-True ($parameters["Exclude${credential}Credential"] -eq $true) "Unexpected authentication fallback: $credential."
    }
    if ($global:FlowLikeWindowsSigningTestState.failSigning) { throw 'Simulated signing service failure.' }
}

function Get-AuthenticodeSignature {
    [CmdletBinding()]
    param([string] $FilePath, [string] $LiteralPath)
    $path = if ($LiteralPath) { $LiteralPath } else { $FilePath }
    Assert-True (Test-Path -LiteralPath $path -PathType Leaf) 'Signature verification targeted a missing file.'
    return [PSCustomObject]@{
        Status = $global:FlowLikeWindowsSigningTestState.signatureStatus
        StatusMessage = 'Mock signature result'
        TimeStamperCertificate = $global:FlowLikeWindowsSigningTestState.timestamp
    }
}

function Assert-TokenCleanup {
    Assert-True ($env:AZURE_FEDERATED_TOKEN_FILE -eq $global:FlowLikeWindowsSigningTestState.previousTokenPath) 'The prior federated token setting was not restored.'
    foreach ($path in @($global:FlowLikeWindowsSigningTestState.tokenPaths) + @($global:FlowLikeWindowsSigningTestState.catalogPaths)) {
        Assert-True (-not (Test-Path -LiteralPath $path)) 'A temporary signing token or catalog remains on disk.'
    }
    Assert-True ([IO.File]::ReadAllText($global:FlowLikeWindowsSigningTestState.previousTokenPath) -eq 'existing-token') 'The existing token file was changed.'
}

function Reset-FailedBuild {
    Assert-True (Test-Path -LiteralPath $env:FLOW_LIKE_WINDOWS_SIGNING_FAILURE_FILE -PathType Leaf) 'Signing failure was not persisted for the rest of the build.'
    # Each following failure case represents a separate build.
    Remove-Item -LiteralPath $env:FLOW_LIKE_WINDOWS_SIGNING_FAILURE_FILE
}

try {
    New-Item -ItemType Directory -Path $fixtureRoot | Out-Null
    foreach ($name in $settings.Keys) {
        [Environment]::SetEnvironmentVariable($name, $settings[$name])
    }
    $global:FlowLikeWindowsSigningTestState.previousTokenPath = Join-Path $fixtureRoot 'existing token.jwt'
    [IO.File]::WriteAllText($global:FlowLikeWindowsSigningTestState.previousTokenPath, 'existing-token')
    $env:AZURE_FEDERATED_TOKEN_FILE = $global:FlowLikeWindowsSigningTestState.previousTokenPath
    $executable = Join-Path $fixtureRoot 'Flow Like, desktop.exe'
    $uninstaller = Join-Path $fixtureRoot 'Flow Like uninstaller.tmp'
    $installer = Join-Path $fixtureRoot 'Flow Like installer.exe'
    [IO.File]::WriteAllText($executable, 'test executable')
    [IO.File]::WriteAllText($uninstaller, 'test NSIS uninstaller')
    [IO.File]::WriteAllText($installer, 'test NSIS installer')

    & $signer -FilePath $executable
    Assert-True ($global:FlowLikeWindowsSigningTestState.lastSignedPath -eq $executable) 'The filename containing a comma and spaces was changed.'
    Assert-True ($global:FlowLikeWindowsSigningTestState.lastParameters.Endpoint -eq $settings.AZURE_TRUSTED_SIGNING_ENDPOINT) 'Wrong signing endpoint.'
    Assert-True ($global:FlowLikeWindowsSigningTestState.lastParameters.CodeSigningAccountName -eq 'test-account') 'Wrong signing account.'
    Assert-True ($global:FlowLikeWindowsSigningTestState.lastParameters.CertificateProfileName -eq 'test-profile') 'Wrong certificate profile.'
    Assert-True ($global:FlowLikeWindowsSigningTestState.lastParameters.FileDigest -eq 'SHA256') 'File digest must be SHA256.'
    Assert-True ($global:FlowLikeWindowsSigningTestState.lastParameters.TimestampRfc3161 -eq 'http://timestamp.acs.microsoft.com') 'Microsoft timestamping is required.'
    Assert-True ($global:FlowLikeWindowsSigningTestState.lastParameters.TimestampDigest -eq 'SHA256') 'Timestamp digest must be SHA256.'
    Assert-TokenCleanup

    & $signer -FilePath $uninstaller
    Assert-True ($global:FlowLikeWindowsSigningTestState.lastSignedPath -eq $uninstaller) 'The NSIS .tmp uninstaller was not signed.'
    Assert-True ($global:FlowLikeWindowsSigningTestState.tokenRequests -eq 2) 'Each signing invocation must request a fresh token.'
    Assert-True ($global:FlowLikeWindowsSigningTestState.tokens[0] -ne $global:FlowLikeWindowsSigningTestState.tokens[1]) 'A federation token was reused.'
    Assert-TokenCleanup
    Assert-True (-not (Test-Path -LiteralPath $env:FLOW_LIKE_WINDOWS_SIGNING_FAILURE_FILE)) 'Successful signing marked the build as failed.'

    # NSIS may ignore its uninstaller signing exit code and continue packaging.
    $global:FlowLikeWindowsSigningTestState.failSigning = $true
    Assert-Throws { & $signer -FilePath $uninstaller } 'Simulated signing service failure'
    $global:FlowLikeWindowsSigningTestState.failSigning = $false
    Assert-TokenCleanup
    $requestsBefore = $global:FlowLikeWindowsSigningTestState.tokenRequests
    Assert-Throws { & $signer -FilePath $installer } 'earlier Windows signing operation failed'
    Assert-True ($global:FlowLikeWindowsSigningTestState.tokenRequests -eq $requestsBefore) 'Installer signing continued after an uninstaller signing failure.'
    Assert-TokenCleanup
    Reset-FailedBuild

    foreach ($status in @('HashMismatch', 'NotSigned')) {
        $global:FlowLikeWindowsSigningTestState.signatureStatus = $status
        Assert-Throws { & $signer -FilePath $executable }
        Assert-TokenCleanup
        Reset-FailedBuild
    }
    $global:FlowLikeWindowsSigningTestState.signatureStatus = 'Valid'
    $global:FlowLikeWindowsSigningTestState.timestamp = $null
    Assert-Throws { & $signer -FilePath $executable } 'timestamp'
    $global:FlowLikeWindowsSigningTestState.timestamp = [PSCustomObject]@{ Subject = 'Test timestamp' }
    Assert-TokenCleanup
    Reset-FailedBuild

    foreach ($name in $settings.Keys) {
        $requestsBefore = $global:FlowLikeWindowsSigningTestState.tokenRequests
        [Environment]::SetEnvironmentVariable($name, $null)
        try {
            Assert-Throws { & $signer -FilePath $executable } $name
            Assert-True ($global:FlowLikeWindowsSigningTestState.tokenRequests -eq $requestsBefore) "Missing $name should fail before requesting a token."
        } finally {
            [Environment]::SetEnvironmentVariable($name, $settings[$name])
        }
        Assert-TokenCleanup
        if ($name -ne 'FLOW_LIKE_WINDOWS_SIGNING_FAILURE_FILE') {
            Reset-FailedBuild
        }
    }

    Assert-Throws { & $signer -FilePath (Join-Path $fixtureRoot 'missing.exe') }
    Assert-TokenCleanup
    Reset-FailedBuild

    [Environment]::SetEnvironmentVariable('AZURE_FEDERATED_TOKEN_FILE', $null)
    & $signer -FilePath $executable
    Assert-True ([string]::IsNullOrEmpty($env:AZURE_FEDERATED_TOKEN_FILE)) 'Signing left a token environment variable behind.'
    $env:AZURE_FEDERATED_TOKEN_FILE = $global:FlowLikeWindowsSigningTestState.previousTokenPath
    Assert-TokenCleanup
    Write-Host 'Windows signing tests passed: filenames, OIDC renewal, signature validation, NSIS failure propagation, and cleanup.'
} finally {
    foreach ($name in $originalEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $originalEnvironment[$name])
    }
    Remove-Item -LiteralPath $fixtureRoot -Recurse -Force -ErrorAction SilentlyContinue
    if ($null -ne $previousTestState) {
        Set-Variable -Name FlowLikeWindowsSigningTestState -Scope Global -Value $previousTestState.Value
    } else {
        Remove-Variable -Name FlowLikeWindowsSigningTestState -Scope Global
    }
}
