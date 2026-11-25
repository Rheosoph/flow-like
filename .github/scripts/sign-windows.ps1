param(
    [Parameter(Mandatory = $true)]
    [string] $FilePath
)

$ErrorActionPreference = 'Stop'
$failurePath = $env:FLOW_LIKE_WINDOWS_SIGNING_FAILURE_FILE
if ([string]::IsNullOrWhiteSpace($failurePath)) {
    throw 'Windows signing requires FLOW_LIKE_WINDOWS_SIGNING_FAILURE_FILE from setup-windows-signing.ps1.'
}

# NSIS does not propagate !uninstfinalize failures. Latch every failure so the
# next signing call refuses to sign an installer containing a failed uninstaller.
try {
    if (Test-Path -LiteralPath $failurePath) {
        throw 'An earlier Windows signing operation failed. Refusing to sign further release artifacts.'
    }
    foreach ($name in @(
        'AZURE_CLIENT_ID',
        'AZURE_TENANT_ID',
        'AZURE_TRUSTED_SIGNING_ENDPOINT',
        'AZURE_TRUSTED_SIGNING_ACCOUNT_NAME',
        'AZURE_TRUSTED_SIGNING_CERTIFICATE_PROFILE_NAME',
        'ACTIONS_ID_TOKEN_REQUEST_URL',
        'ACTIONS_ID_TOKEN_REQUEST_TOKEN'
    )) {
        if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
            throw "Windows signing requires $name. Check the repository variables and id-token: write permissions."
        }
    }

    $file = Get-Item -LiteralPath $FilePath
    if ($file.PSIsContainer) {
        throw "Expected a file to sign: $FilePath"
    }

    Import-Module ArtifactSigning -RequiredVersion 0.1.20 -ErrorAction Stop

    $tokenPath = [IO.Path]::GetTempFileName()
    # Catalog entries are relative to the catalog. Keeping it beside the input also
    # handles commas in filenames and NSIS uninstallers on a different drive.
    $catalogPath = Join-Path $file.DirectoryName ('.flow-like-signing-' + [guid]::NewGuid() + '.txt')
    $previousTokenFile = $env:AZURE_FEDERATED_TOKEN_FILE
    try {
        # Fetch a fresh assertion after compilation, including for each installer.
        $separator = if ($env:ACTIONS_ID_TOKEN_REQUEST_URL.Contains('?')) { '&' } else { '?' }
        $audience = [Uri]::EscapeDataString('api://AzureADTokenExchange')
        $response = Invoke-RestMethod -Method Get `
            -Uri "$($env:ACTIONS_ID_TOKEN_REQUEST_URL)${separator}audience=$audience" `
            -Headers @{ Authorization = "Bearer $env:ACTIONS_ID_TOKEN_REQUEST_TOKEN" } `
            -TimeoutSec 60
        if ([string]::IsNullOrWhiteSpace($response.value)) {
            throw 'GitHub did not return an OIDC token for Windows signing.'
        }
        Write-Host "::add-mask::$($response.value)"
        [IO.File]::WriteAllText($tokenPath, $response.value)
        $env:AZURE_FEDERATED_TOKEN_FILE = $tokenPath
        [IO.File]::WriteAllText($catalogPath, $file.Name)

        $parameters = @{
            Endpoint = $env:AZURE_TRUSTED_SIGNING_ENDPOINT
            CodeSigningAccountName = $env:AZURE_TRUSTED_SIGNING_ACCOUNT_NAME
            CertificateProfileName = $env:AZURE_TRUSTED_SIGNING_CERTIFICATE_PROFILE_NAME
            FilesCatalog = $catalogPath
            FileDigest = 'SHA256'
            TimestampRfc3161 = 'http://timestamp.acs.microsoft.com'
            TimestampDigest = 'SHA256'
            Description = 'Flow Like'
            DescriptionUrl = 'https://flow-like.com'
            ExcludeEnvironmentCredential = $true
            ExcludeWorkloadIdentityCredential = $false
            ExcludeManagedIdentityCredential = $true
            ExcludeSharedTokenCacheCredential = $true
            ExcludeVisualStudioCredential = $true
            ExcludeVisualStudioCodeCredential = $true
            ExcludeAzureCliCredential = $true
            ExcludeAzurePowerShellCredential = $true
            ExcludeAzureDeveloperCliCredential = $true
            ExcludeInteractiveBrowserCredential = $true
        }
        Invoke-ArtifactSigning @parameters

        # SignTool can return a warning when timestamping fails. The short-lived
        # Azure certificate needs a timestamp to remain valid after it expires.
        $signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
        if ($signature.Status -ne 'Valid' -or $null -eq $signature.TimeStamperCertificate) {
            throw "Windows signing verification failed for $($file.FullName): status=$($signature.Status), timestamp=$($null -ne $signature.TimeStamperCertificate)"
        }
        Write-Host "Verified Windows signature and timestamp: $($file.FullName)"
    } finally {
        $env:AZURE_FEDERATED_TOKEN_FILE = $previousTokenFile
        Remove-Item -LiteralPath $tokenPath, $catalogPath -Force -ErrorAction SilentlyContinue
    }
} catch {
    [IO.File]::WriteAllText($failurePath, 'Windows signing failed. See the first signing error in this build.')
    throw
}
