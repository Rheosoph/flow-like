use crate::config::CompilerConfig;
use crate::error::CompilerError;
use crate::jwt::{verify_jwt_async, CompilerClaims};
use flow_like_types_contracts::dispatch::{
    compilation_job_payload_hash, CompilationJob, CompilationResult, CompilationStatus,
    CompilationStorageProvider,
};
use flow_like_wasm::aot_cache::WASMTIME_MAJOR_VERSION;
use flow_like_wasm::{WasmConfig, WasmEngine, WasmSecurityConfig};
use reqwest::{Client, Url};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

const MAX_SIGNED_URL_BYTES: usize = 8 * 1024;
// Must stay well below the callback endpoint's request body limit or an
// oversized error can never deliver its terminal callback.
const MAX_CALLBACK_ERROR_BYTES: usize = 2 * 1024;
const STORAGE_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

pub async fn compile(
    job: CompilationJob,
    config: &CompilerConfig,
) -> Result<CompilationResult, CompilerError> {
    if config.max_parallel_targets == Some(0) {
        return Err(CompilerError::Config(
            "Compiler target parallelism must be positive".to_string(),
        ));
    }
    let claims = verify_jwt_async(&job.compiler_jwt).await?;
    if let Err(error) = validate_job_envelope(&job, &claims, config) {
        return Err(envelope_failure(&job, &claims, error, config).await);
    }

    info!(
        job_id = %job.job_id,
        package_id = %job.package_id,
        version = %job.version,
        targets = job.targets.len(),
        "Starting compilation job"
    );

    let storage_client = storage_client(config)?;
    let result = tokio::time::timeout(
        config.compilation_timeout(),
        compile_inner(&job, config, &storage_client),
    )
    .await;

    let compilation_result = match result {
        Ok(Ok((platforms, nodes))) => CompilationResult {
            job_id: job.job_id.clone(),
            package_id: job.package_id.clone(),
            version: job.version.clone(),
            status: CompilationStatus::Compiled,
            compiled_platforms: platforms,
            error: None,
            nodes,
        },
        // Object-store and callback infrastructure failures are not terminal
        // compilation outcomes. Returning them lets a queue retry without
        // durably marking the package failed.
        Ok(Err(error)) if is_retryable(&error) => return Err(error),
        // Every other failure is deterministic: a retry produces the same
        // error, so skipping the callback would leave the package pinned in
        // "compiling" forever.
        Ok(Err(error)) => CompilationResult {
            job_id: job.job_id.clone(),
            package_id: job.package_id.clone(),
            version: job.version.clone(),
            status: CompilationStatus::Failed,
            compiled_platforms: Vec::new(),
            error: Some(bounded_terminal_error(&error)),
            nodes: None,
        },
        Err(_) => CompilationResult {
            job_id: job.job_id.clone(),
            package_id: job.package_id.clone(),
            version: job.version.clone(),
            status: CompilationStatus::Failed,
            compiled_platforms: Vec::new(),
            error: Some("Compilation timed out".to_string()),
            nodes: None,
        },
    };

    info!(
        has_nodes = compilation_result.nodes.is_some(),
        platforms = compilation_result.compiled_platforms.len(),
        "Sending compilation callback"
    );

    send_callback(
        &claims.callback_url,
        &job.compiler_jwt,
        &compilation_result,
        config,
    )
    .await?;
    // A failed compilation whose terminal callback was acknowledged is a
    // successfully delivered queue job. Retrying it would duplicate expensive
    // work and can overwrite the acknowledged terminal state.
    Ok(compilation_result)
}

fn storage_client(config: &CompilerConfig) -> Result<Client, CompilerError> {
    Client::builder()
        // Cleartext stays off unless the operator allowlisted an http origin.
        // Every URL is still origin-checked in `validate_storage_url` first.
        .https_only(!config.allows_plaintext_storage())
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(STORAGE_CONNECT_TIMEOUT)
        .timeout(config.storage_timeout())
        .build()
        .map_err(|_| CompilerError::Config("failed to build hardened storage client".to_string()))
}

async fn download_wasm(
    client: &Client,
    url: &str,
    max_bytes: u64,
) -> Result<bytes::Bytes, CompilerError> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| CompilerError::Download("signed WASM GET request failed".to_string()))?;

    if !response.status().is_success() {
        return Err(CompilerError::Download(format!(
            "signed WASM GET returned {}",
            response.status()
        )));
    }

    if response
        .content_length()
        .is_some_and(|length| length > max_bytes)
    {
        return Err(CompilerError::Download(
            "signed WASM response exceeds the configured size limit".to_string(),
        ));
    }

    let mut body =
        Vec::with_capacity(response.content_length().unwrap_or_default().min(max_bytes) as usize);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| CompilerError::Download("failed to read signed WASM body".to_string()))?
    {
        let next_len = body.len().checked_add(chunk.len()).ok_or_else(|| {
            CompilerError::Download("signed WASM response length overflow".to_string())
        })?;
        if next_len as u64 > max_bytes {
            return Err(CompilerError::Download(
                "signed WASM response exceeds the configured size limit".to_string(),
            ));
        }
        body.extend_from_slice(&chunk);
    }

    Ok(bytes::Bytes::from(body))
}

async fn upload_artifact(
    client: &Client,
    url: &str,
    provider: CompilationStorageProvider,
    data: Vec<u8>,
    max_bytes: u64,
) -> Result<(), CompilerError> {
    if data.len() as u64 > max_bytes {
        return Err(CompilerError::Upload(
            "compiled artifact exceeds the configured size limit".to_string(),
        ));
    }

    let mut request = client
        .put(url)
        .header("Content-Type", "application/octet-stream");
    for (name, value) in required_upload_headers(provider) {
        request = request.header(*name, *value);
    }
    let response = request
        .body(data)
        .send()
        .await
        .map_err(|_| CompilerError::Upload("signed artifact PUT request failed".to_string()))?;

    if !response.status().is_success() {
        let status = response.status();
        return Err(CompilerError::Upload(format!(
            "signed artifact PUT returned {status}"
        )));
    }
    Ok(())
}

fn required_upload_headers(
    provider: CompilationStorageProvider,
) -> &'static [(&'static str, &'static str)] {
    match provider {
        CompilationStorageProvider::AzureBlob => &[("x-ms-blob-type", "BlockBlob")],
        CompilationStorageProvider::AwsS3 | CompilationStorageProvider::GoogleCloudStorage => &[],
    }
}

#[derive(Clone, Copy)]
enum StorageOperation {
    Download,
    Upload,
}

fn validate_job_envelope(
    job: &CompilationJob,
    claims: &CompilerClaims,
    config: &CompilerConfig,
) -> Result<(), CompilerError> {
    validate_identifier("job_id", &job.job_id, 128)?;
    validate_identifier("package_id", &job.package_id, 128)?;
    validate_identifier("version", &job.version, 128)?;
    validate_identifier("subject", &claims.sub, 256)?;
    validate_identifier("token id", &claims.jti, 128)?;

    if claims.token_type != "compiler" {
        return Err(invalid_job("compiler JWT has an invalid token type"));
    }
    if claims.job_id != job.job_id
        || claims.package_id != job.package_id
        || claims.version != job.version
    {
        return Err(invalid_job(
            "compiler JWT claims do not match the queued job",
        ));
    }
    if !is_lower_hex(&job.wasm_hash, 64) {
        return Err(invalid_job("WASM hash must be 64 lowercase hex characters"));
    }

    let payload_hash = compilation_job_payload_hash(job).map_err(|_| {
        CompilerError::Config("failed to canonicalize compilation job envelope".to_string())
    })?;
    if !is_lower_hex(&claims.payload_hash, 64) || claims.payload_hash != payload_hash {
        return Err(invalid_job(
            "compiler JWT does not bind the queued job envelope",
        ));
    }

    validate_callback_url(&claims.callback_url)?;

    if !(1..=32).contains(&job.targets.len()) {
        return Err(invalid_job(
            "compilation job must contain between 1 and 32 targets",
        ));
    }

    let wasm_path = format!("wasm/{}/{}/node.wasm", job.package_id, job.version);
    validate_storage_url(
        &job.wasm_download_url,
        job.wasm_download_provider,
        StorageOperation::Download,
        &wasm_path,
        config.azure_content_container.as_deref(),
        config,
    )?;

    let mut platforms = HashSet::with_capacity(job.targets.len());
    for target in &job.targets {
        validate_identifier("platform_key", &target.platform_key, 128)?;
        if target
            .platform_key
            .rsplit_once("-wt")
            .map(|(_, version)| version)
            != Some(WASMTIME_MAJOR_VERSION)
        {
            return Err(invalid_job(format!(
                "compilation target must use this worker's Wasmtime version (-wt{WASMTIME_MAJOR_VERSION})"
            )));
        }
        if !platforms.insert(target.platform_key.as_str()) {
            return Err(invalid_job("compilation target platform is duplicated"));
        }
        if let Some(triple) = &target.cross_triple {
            validate_identifier("cross_triple", triple, 128)?;
        }

        let artifact_path = format!(
            "wasm-compiled/{}/{}/{}.cwasm",
            job.package_id, job.version, target.platform_key
        );
        let checksum_path = format!("{artifact_path}.b3");
        validate_storage_url(
            &target.cwasm_upload_url,
            target.upload_provider,
            StorageOperation::Upload,
            &artifact_path,
            config.azure_meta_container.as_deref(),
            config,
        )?;
        validate_storage_url(
            &target.checksum_upload_url,
            target.upload_provider,
            StorageOperation::Upload,
            &checksum_path,
            config.azure_meta_container.as_deref(),
            config,
        )?;
    }

    Ok(())
}

fn validate_identifier(name: &str, value: &str, maximum: usize) -> Result<(), CompilerError> {
    let valid = !value.is_empty()
        && value.len() <= maximum
        && value != "."
        && value != ".."
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'+' | b'@' | b':')
        });
    if valid {
        Ok(())
    } else {
        Err(invalid_job(format!("{name} has an invalid shape")))
    }
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_callback_url(value: &str) -> Result<(), CompilerError> {
    let url = parse_bounded_url(value, "callback")?;
    if url.path() != "/api/v1/registry/compilation-callback"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid_job("compiler callback URL has an invalid path"));
    }

    match url.scheme() {
        "https" if url.port().is_none() || url.port() == Some(443) => Ok(()),
        // Retain explicit local/container development support. Public callback
        // hosts never get a bearer token over cleartext HTTP.
        "http" if url.host_str().is_some_and(is_private_callback_host) => Ok(()),
        _ => Err(invalid_job(
            "compiler callback URL must use HTTPS outside private development networking",
        )),
    }
}

fn is_private_callback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") || !host.contains('.') {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(|address| match address {
        IpAddr::V4(address) => {
            address.is_private() || address.is_loopback() || address.is_link_local()
        }
        IpAddr::V6(address) => address.is_loopback() || address.is_unique_local(),
    })
}

fn parse_bounded_url(value: &str, purpose: &str) -> Result<Url, CompilerError> {
    if value.len() > MAX_SIGNED_URL_BYTES {
        return Err(invalid_job(format!("{purpose} URL is too long")));
    }
    let url = Url::parse(value).map_err(|_| invalid_job(format!("{purpose} URL is invalid")))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid_job(format!(
            "{purpose} URL must not contain user information"
        )));
    }
    Ok(url)
}

/// Scheme, host and port of a signed URL, in the exact form an operator would
/// add to `COMPILER_ALLOWED_STORAGE_HOSTS`. Never includes the signature.
fn storage_origin(url: &Url, host: &str) -> String {
    match url.port_or_known_default() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

fn validate_storage_url(
    value: &str,
    provider: CompilationStorageProvider,
    operation: StorageOperation,
    logical_path: &str,
    azure_container: Option<&str>,
    config: &CompilerConfig,
) -> Result<(), CompilerError> {
    let url = parse_bounded_url(value, "storage")?;
    if url.fragment().is_some() {
        return Err(invalid_job("signed storage URL must not carry a fragment"));
    }
    let host = url
        .host_str()
        .ok_or_else(|| invalid_job("signed storage URL has no host"))?
        .to_ascii_lowercase();
    if host.parse::<IpAddr>().is_ok() {
        return Err(invalid_job(
            "signed storage URL must not target an IP address",
        ));
    }

    // The allowlist is consulted before the transport check so an operator can
    // pin a self-hosted origin. Anything not pinned stays on HTTPS port 443.
    let origin = storage_origin(&url, &host);
    let explicitly_allowed = url
        .port_or_known_default()
        .is_some_and(|port| config.allows_storage_origin(url.scheme(), &host, port));
    if !explicitly_allowed && (url.scheme() != "https" || !matches!(url.port(), None | Some(443))) {
        return Err(invalid_job(format!(
            "signed storage URL origin {origin} must be HTTPS on port 443; add {origin} to COMPILER_ALLOWED_STORAGE_HOSTS to allow this endpoint"
        )));
    }

    match provider {
        CompilationStorageProvider::AzureBlob => {
            let account = config.azure_storage_account.as_deref().ok_or_else(|| {
                CompilerError::Config(
                    "AZURE_STORAGE_ACCOUNT_NAME is required for Azure compilation".to_string(),
                )
            })?;
            validate_azure_label("storage account", account)?;
            if host != format!("{}.blob.core.windows.net", account.to_ascii_lowercase()) {
                return Err(invalid_job(
                    "Azure signed URL host does not match the configured storage account",
                ));
            }
        }
        CompilationStorageProvider::AwsS3 => {
            // S3 Express One Zone signs against the zonal endpoint
            // `<bucket>--<az>--x-s3.s3express-<az>.<region>.amazonaws.com`, which
            // carries no `.s3.` label. It is still an AWS-operated endpoint, so it
            // belongs with the public hosts rather than behind the allowlist that
            // exists for third-party S3-compatible endpoints.
            let is_public_s3 =
                (host.starts_with("s3.") || host.contains(".s3.") || host.contains(".s3express-"))
                    && (host.ends_with(".amazonaws.com") || host.ends_with(".amazonaws.com.cn"));
            if !is_public_s3 && !explicitly_allowed {
                return Err(invalid_job(format!(
                    "signed S3 URL origin {origin} is not an approved endpoint; add {origin} to COMPILER_ALLOWED_STORAGE_HOSTS to use an S3-compatible endpoint such as Cloudflare R2 or MinIO"
                )));
            }
        }
        CompilationStorageProvider::GoogleCloudStorage => {
            let is_public_gcs =
                host == "storage.googleapis.com" || host.ends_with(".storage.googleapis.com");
            if !is_public_gcs && !explicitly_allowed {
                return Err(invalid_job(format!(
                    "signed GCS URL origin {origin} is not an approved endpoint; add {origin} to COMPILER_ALLOWED_STORAGE_HOSTS to use a private GCS-compatible endpoint"
                )));
            }
        }
    }

    let raw_path = url.path().to_ascii_lowercase();
    if ["%2f", "%5c", "%00"]
        .iter()
        .any(|forbidden| raw_path.contains(forbidden))
    {
        return Err(invalid_job(
            "signed storage URL path contains a forbidden encoded separator",
        ));
    }
    let decoded_path = percent_encoding::percent_decode_str(url.path())
        .decode_utf8()
        .map_err(|_| invalid_job("signed storage URL path is not UTF-8"))?;
    let expected_suffix = format!("/{logical_path}");
    if provider == CompilationStorageProvider::AzureBlob {
        let container = azure_container.ok_or_else(|| {
            CompilerError::Config(
                "Azure content and metadata containers are required for compilation".to_string(),
            )
        })?;
        validate_azure_label("storage container", container)?;
        if decoded_path != format!("/{container}/{logical_path}") {
            return Err(invalid_job(
                "Azure signed URL path is outside the expected container/object",
            ));
        }
    } else if !decoded_path.ends_with(&expected_suffix) {
        return Err(invalid_job(
            "signed storage URL path does not match the expected compilation object",
        ));
    }

    let query = unique_query(&url)?;
    match provider {
        CompilationStorageProvider::AzureBlob => validate_azure_sas(&query, operation),
        CompilationStorageProvider::AwsS3 => validate_aws_signature(&query),
        CompilationStorageProvider::GoogleCloudStorage => validate_gcs_signature(&query),
    }
}

fn validate_azure_label(name: &str, value: &str) -> Result<(), CompilerError> {
    let valid = (3..=63).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--");
    if valid {
        Ok(())
    } else {
        Err(CompilerError::Config(format!(
            "configured Azure {name} is invalid"
        )))
    }
}

fn unique_query(url: &Url) -> Result<HashMap<String, String>, CompilerError> {
    let mut query = HashMap::new();
    for (name, value) in url.query_pairs() {
        let name = name.to_ascii_lowercase();
        if name.is_empty() || value.is_empty() || query.insert(name, value.into_owned()).is_some() {
            return Err(invalid_job(
                "signed storage URL contains empty or duplicate query fields",
            ));
        }
    }
    if query.is_empty() {
        return Err(invalid_job("storage URL is not authenticated"));
    }
    Ok(query)
}

fn validate_azure_sas(
    query: &HashMap<String, String>,
    operation: StorageOperation,
) -> Result<(), CompilerError> {
    for required in [
        "sig", "sv", "st", "se", "sr", "sp", "skoid", "sktid", "skt", "ske", "sks", "skv",
    ] {
        if !query.contains_key(required) {
            return Err(invalid_job(
                "Azure URL must use a Microsoft Entra user-delegation SAS",
            ));
        }
    }
    if query.get("sr").map(String::as_str) != Some("b")
        || query.get("sp").map(String::as_str)
            != Some(match operation {
                StorageOperation::Download => "r",
                StorageOperation::Upload => "w",
            })
        || query.get("spr").is_some_and(|protocol| protocol != "https")
    {
        return Err(invalid_job(
            "Azure SAS is not scoped to the expected blob operation",
        ));
    }
    validate_rfc3339_expiry(query.get("se").expect("checked above"), 2 * 60 * 60)
}

fn validate_aws_signature(query: &HashMap<String, String>) -> Result<(), CompilerError> {
    for required in [
        "x-amz-algorithm",
        "x-amz-credential",
        "x-amz-date",
        "x-amz-expires",
        "x-amz-signature",
        "x-amz-signedheaders",
    ] {
        if !query.contains_key(required) {
            return Err(invalid_job(
                "S3 URL does not contain a complete V4 signature",
            ));
        }
    }
    validate_relative_expiry(
        query.get("x-amz-expires").expect("checked above"),
        2 * 60 * 60,
    )
}

fn validate_gcs_signature(query: &HashMap<String, String>) -> Result<(), CompilerError> {
    for required in [
        "x-goog-algorithm",
        "x-goog-credential",
        "x-goog-date",
        "x-goog-expires",
        "x-goog-signature",
        "x-goog-signedheaders",
    ] {
        if !query.contains_key(required) {
            return Err(invalid_job(
                "GCS URL does not contain a complete V4 signature",
            ));
        }
    }
    validate_relative_expiry(
        query.get("x-goog-expires").expect("checked above"),
        2 * 60 * 60,
    )
}

fn validate_relative_expiry(value: &str, maximum_secs: u64) -> Result<(), CompilerError> {
    let seconds = value
        .parse::<u64>()
        .map_err(|_| invalid_job("signed storage URL has an invalid expiry"))?;
    if seconds == 0 || seconds > maximum_secs {
        return Err(invalid_job(
            "signed storage URL expiry exceeds the worker policy",
        ));
    }
    Ok(())
}

fn validate_rfc3339_expiry(value: &str, maximum_secs: i64) -> Result<(), CompilerError> {
    let expiry = chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid_job("Azure SAS has an invalid expiry"))?
        .with_timezone(&chrono::Utc);
    let now = chrono::Utc::now();
    if expiry < now - chrono::Duration::minutes(5)
        || expiry > now + chrono::Duration::seconds(maximum_secs)
    {
        return Err(invalid_job("Azure SAS expiry is outside the worker policy"));
    }
    Ok(())
}

fn invalid_job(message: impl Into<String>) -> CompilerError {
    CompilerError::InvalidJob(message.into())
}

fn is_retryable(error: &CompilerError) -> bool {
    matches!(
        error,
        CompilerError::Download(_)
            | CompilerError::Upload(_)
            | CompilerError::Config(_)
            | CompilerError::Callback(_)
    )
}

/// Envelope rejections are deterministic, so without a terminal callback the
/// package would stay pinned in "compiling" while the message poisons. The
/// callback only goes to a callback URL that passed validation itself, and it
/// carries the verified claim identifiers, never the untrusted job fields.
async fn envelope_failure(
    job: &CompilationJob,
    claims: &CompilerClaims,
    error: CompilerError,
    config: &CompilerConfig,
) -> CompilerError {
    if is_retryable(&error) || validate_callback_url(&claims.callback_url).is_err() {
        return error;
    }

    let result = CompilationResult {
        job_id: claims.job_id.clone(),
        package_id: claims.package_id.clone(),
        version: claims.version.clone(),
        status: CompilationStatus::Failed,
        compiled_platforms: Vec::new(),
        error: Some(bounded_terminal_error(&error)),
        nodes: None,
    };
    match send_callback(&claims.callback_url, &job.compiler_jwt, &result, config).await {
        Ok(()) => error,
        Err(callback_error) => callback_error,
    }
}

fn bounded_terminal_error(error: &CompilerError) -> String {
    let mut message = error.to_string();
    if message.len() > MAX_CALLBACK_ERROR_BYTES {
        // Truncating mid-codepoint panics, which would take down the worker
        // instead of delivering the terminal callback.
        let boundary = (0..=MAX_CALLBACK_ERROR_BYTES)
            .rev()
            .find(|index| message.is_char_boundary(*index))
            .unwrap_or(0);
        message.truncate(boundary);
        message.push('…');
    }
    message
}

async fn compile_inner(
    job: &CompilationJob,
    config: &CompilerConfig,
    storage_client: &Client,
) -> Result<(Vec<String>, Option<serde_json::Value>), CompilerError> {
    let wasm_bytes = download_wasm(
        storage_client,
        &job.wasm_download_url,
        config.max_wasm_bytes,
    )
    .await?;

    let actual_hash = blake3::hash(&wasm_bytes).to_hex().to_string();
    if actual_hash != job.wasm_hash {
        return Err(CompilerError::InvalidJob(
            "downloaded WASM does not match the signed content hash".to_string(),
        ));
    }

    // Extract node definitions by instantiating the WASM with the host engine
    let nodes = match extract_nodes(&wasm_bytes).await {
        Ok(defs) => {
            info!(count = defs.len(), "Extracted node definitions from WASM");
            match serde_json::to_value(&defs) {
                Ok(v) => {
                    info!("Node definitions serialized successfully");
                    Some(v)
                }
                Err(e) => {
                    warn!(error = %e, "Failed to serialize node definitions");
                    None
                }
            }
        }
        Err(e) => {
            warn!(error = %e, "Failed to extract node definitions (non-fatal)");
            None
        }
    };

    info!(target_count = job.targets.len(), "Compiling for targets");

    let wasm_bytes = Arc::new(wasm_bytes.to_vec());
    let mut set = tokio::task::JoinSet::new();
    let mut compiled_platforms = Vec::new();

    let max_parallel = config
        .max_parallel_targets
        .unwrap_or_else(|| num_cpus::get().max(1));

    for target in &job.targets {
        let bytes = wasm_bytes.clone();
        let platform_key = target.platform_key.clone();
        let cross_triple = target.cross_triple.clone();
        let cwasm_url = target.cwasm_upload_url.clone();
        let checksum_url = target.checksum_upload_url.clone();
        let upload_provider = target.upload_provider;
        let storage_client = storage_client.clone();
        let max_artifact_bytes = config.max_artifact_bytes;

        // Wait if we've saturated the parallelism limit
        while set.len() >= max_parallel {
            if let Some(done) = set.join_next().await {
                let platform =
                    done.map_err(|e| CompilerError::Compilation(format!("Task panicked: {e}")))??;
                compiled_platforms.push(platform);
            }
        }

        set.spawn(async move {
            info!(platform = %platform_key, "Compiling target");

            let pk = platform_key.clone();
            let ct = cross_triple.clone();
            let b = bytes.clone();

            let serialized = tokio::task::spawn_blocking(move || {
                let mut wasm_config = WasmConfig::default().without_cache();
                if let Some(triple) = &ct {
                    wasm_config = wasm_config.with_target(triple);
                }
                if pk.starts_with("ios-") {
                    wasm_config = wasm_config.with_ios_memory_layout();
                }
                let engine = WasmEngine::new(wasm_config).map_err(|e| {
                    CompilerError::Compilation(format!("Engine creation failed for {pk}: {e}"))
                })?;
                engine.precompile(&b).map_err(|e| {
                    CompilerError::Compilation(format!("Precompile failed for {pk}: {e}"))
                })
            })
            .await
            .map_err(|e| CompilerError::Compilation(format!("Spawn-blocking panicked: {e}")))??;

            let hash = blake3::hash(&serialized).to_hex().to_string();

            tokio::try_join!(
                upload_artifact(
                    &storage_client,
                    &cwasm_url,
                    upload_provider,
                    serialized,
                    max_artifact_bytes,
                ),
                upload_artifact(
                    &storage_client,
                    &checksum_url,
                    upload_provider,
                    hash.into_bytes(),
                    64,
                ),
            )?;

            info!(platform = %platform_key, "Target compiled and uploaded");
            Ok::<String, CompilerError>(platform_key)
        });
    }

    while let Some(result) = set.join_next().await {
        let platform =
            result.map_err(|e| CompilerError::Compilation(format!("Task panicked: {e}")))??;
        compiled_platforms.push(platform);
    }

    info!(count = compiled_platforms.len(), "All targets compiled");
    Ok((compiled_platforms, nodes))
}

/// Instantiate the WASM module with the host engine to extract node definitions,
/// returning them as `PackageNodeEntry` values ready for storage.
async fn extract_nodes(
    wasm_bytes: &[u8],
) -> Result<Vec<flow_like_wasm::manifest::PackageNodeEntry>, CompilerError> {
    let engine = WasmEngine::new(WasmConfig::default().without_cache())
        .map_err(|e| CompilerError::Compilation(format!("Host engine creation failed: {e}")))?;
    let loaded = engine.load_auto(wasm_bytes).await.map_err(|e| {
        CompilerError::Compilation(format!("Failed to load WASM for node extraction: {e}"))
    })?;
    let security = WasmSecurityConfig::restrictive().for_metadata();
    let mut instance = loaded.instantiate(&engine, security).await.map_err(|e| {
        CompilerError::Compilation(format!(
            "Failed to instantiate WASM for node extraction: {e}"
        ))
    })?;
    let defs = instance
        .call_get_nodes()
        .await
        .map_err(|e| CompilerError::Compilation(format!("Failed to call get_nodes: {e}")))?;
    Ok(defs
        .iter()
        .map(flow_like_wasm::definition_to_package_entry)
        .collect())
}

async fn send_callback(
    url: &str,
    jwt: &str,
    result: &CompilationResult,
    config: &CompilerConfig,
) -> Result<(), CompilerError> {
    // The bearer credential must never follow a redirect to another endpoint.
    // The User-Agent is load-bearing: the AWS edge WAF blocks UA-less
    // requests (CRS `NoUserAgent_HEADER`).
    let client = reqwest::Client::builder()
        .user_agent(concat!("flow-like-compiler/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(STORAGE_CONNECT_TIMEOUT)
        .timeout(config.callback_timeout())
        .build()
        .map_err(|_| CompilerError::Config("failed to build callback client".to_string()))?;

    for attempt in 0..=config.callback_retries {
        let response = client
            .post(url)
            .header("Authorization", format!("Bearer {jwt}"))
            .header("Content-Type", "application/json")
            .json(result)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            Ok(resp) => {
                let status = resp.status();
                warn!(attempt, status = %status, "Compilation callback failed");
            }
            Err(_) => {
                // Request errors can embed a signed callback URL. Keep bearer
                // material and query strings out of logs and result payloads.
                warn!(attempt, "Compilation callback request failed");
            }
        }

        if attempt < config.callback_retries {
            tokio::time::sleep(std::time::Duration::from_millis(200 * (attempt as u64 + 1))).await;
        }
    }

    Err(CompilerError::Callback(format!(
        "Callback failed after {} retries",
        config.callback_retries
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types_contracts::dispatch::{compilation_job_payload_hash, CompilationTarget};

    fn azure_config() -> CompilerConfig {
        CompilerConfig {
            azure_storage_account: Some("flowlikedevdata".to_string()),
            azure_content_container: Some("content".to_string()),
            azure_meta_container: Some("metadata".to_string()),
            ..CompilerConfig::default()
        }
    }

    fn sas_query(permission: &str) -> String {
        let start = chrono::Utc::now() - chrono::Duration::minutes(1);
        let expiry = chrono::Utc::now() + chrono::Duration::minutes(30);
        format!(
            "sv=2025-01-05&sp={permission}&st={}&se={}&sr=b&skoid=object&sktid=tenant&skt={}&ske={}&sks=b&skv=2025-01-05&sig=signature",
            start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            expiry.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            expiry.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        )
    }

    fn azure_job() -> CompilationJob {
        let package_id = "pkg_123";
        let version = "1.2.3";
        let platform = format!("linux-x86_64-wt{WASMTIME_MAJOR_VERSION}");
        CompilationJob {
            job_id: "job_123".to_string(),
            package_id: package_id.to_string(),
            version: version.to_string(),
            wasm_download_url: format!(
                "https://flowlikedevdata.blob.core.windows.net/content/wasm/{package_id}/{version}/node.wasm?{}",
                sas_query("r")
            ),
            wasm_download_provider: CompilationStorageProvider::AzureBlob,
            wasm_hash: "a".repeat(64),
            targets: vec![CompilationTarget {
                platform_key: platform.to_string(),
                cross_triple: None,
                cwasm_upload_url: format!(
                    "https://flowlikedevdata.blob.core.windows.net/metadata/wasm-compiled/{package_id}/{version}/{platform}.cwasm?{}",
                    sas_query("w")
                ),
                checksum_upload_url: format!(
                    "https://flowlikedevdata.blob.core.windows.net/metadata/wasm-compiled/{package_id}/{version}/{platform}.cwasm.b3?{}",
                    sas_query("w")
                ),
                upload_provider: CompilationStorageProvider::AzureBlob,
            }],
            compiler_jwt: "not-part-of-payload-hash".to_string(),
        }
    }

    fn claims_for(job: &CompilationJob) -> CompilerClaims {
        CompilerClaims {
            sub: "user_123".to_string(),
            job_id: job.job_id.clone(),
            package_id: job.package_id.clone(),
            version: job.version.clone(),
            payload_hash: compilation_job_payload_hash(job).unwrap(),
            callback_url: "https://flow-like.example/api/v1/registry/compilation-callback"
                .to_string(),
            token_type: "compiler".to_string(),
            iss: "flow-like".to_string(),
            aud: "flow-like-compiler".to_string(),
            iat: 1,
            nbf: 1,
            exp: 2,
            jti: "token_123".to_string(),
        }
    }

    #[test]
    fn azure_job_requires_exact_account_container_and_object_paths() {
        let job = azure_job();
        assert!(validate_job_envelope(&job, &claims_for(&job), &azure_config()).is_ok());

        let mut wrong_host = job.clone();
        wrong_host.wasm_download_url = wrong_host
            .wasm_download_url
            .replace("flowlikedevdata", "attacker");
        let claims = claims_for(&wrong_host);
        assert!(validate_job_envelope(&wrong_host, &claims, &azure_config()).is_err());

        let mut wrong_path = job.clone();
        wrong_path.targets[0].cwasm_upload_url = wrong_path.targets[0]
            .cwasm_upload_url
            .replace("/metadata/", "/content/");
        let claims = claims_for(&wrong_path);
        assert!(validate_job_envelope(&wrong_path, &claims, &azure_config()).is_err());
    }

    #[test]
    fn compilation_target_version_must_match_worker() {
        let job = azure_job();
        assert!(validate_job_envelope(&job, &claims_for(&job), &azure_config()).is_ok());

        let current = WASMTIME_MAJOR_VERSION.parse::<u32>().unwrap();
        for platform in [
            format!("linux-x86_64-wt{}", current - 1),
            format!("linux-x86_64-wt{}", current + 1),
            "linux-x86_64".to_string(),
        ] {
            let mut mismatched = job.clone();
            let target = &mut mismatched.targets[0];
            target.cwasm_upload_url = target
                .cwasm_upload_url
                .replace(&target.platform_key, &platform);
            target.checksum_upload_url = target
                .checksum_upload_url
                .replace(&target.platform_key, &platform);
            target.platform_key = platform;

            // Bind the changed envelope so rejection tests the compiler version,
            // even when the API has authorized every upload URL in this job.
            let error =
                validate_job_envelope(&mismatched, &claims_for(&mismatched), &azure_config())
                    .unwrap_err();
            assert!(matches!(error, CompilerError::InvalidJob(_)));
            assert!(error.to_string().contains("worker's Wasmtime version"));
        }
    }

    #[test]
    fn payload_binding_detects_url_and_provider_tampering() {
        let job = azure_job();
        let claims = claims_for(&job);

        let mut tampered = job.clone();
        tampered.targets[0].upload_provider = CompilationStorageProvider::AwsS3;
        assert!(validate_job_envelope(&tampered, &claims, &azure_config()).is_err());

        let mut tampered = job.clone();
        tampered.wasm_download_url.push_str("&ignored=changed");
        assert!(validate_job_envelope(&tampered, &claims, &azure_config()).is_err());
    }

    #[test]
    fn bounded_terminal_error_truncates_on_a_character_boundary() {
        let error = CompilerError::Compilation("é".repeat(MAX_CALLBACK_ERROR_BYTES));
        let bounded = bounded_terminal_error(&error);
        assert!(bounded.len() <= MAX_CALLBACK_ERROR_BYTES + "…".len());
        assert!(bounded.ends_with('…'));
    }

    #[test]
    fn deterministic_failures_are_not_retried_without_a_terminal_callback() {
        assert!(!is_retryable(&CompilerError::InvalidJob("bad".to_string())));
        assert!(!is_retryable(&CompilerError::Compilation(
            "bad".to_string()
        )));
        assert!(is_retryable(&CompilerError::Download("flaky".to_string())));
        assert!(is_retryable(&CompilerError::Upload("flaky".to_string())));
    }

    async fn callback_capture_server() -> (String, tokio::sync::mpsc::Receiver<CompilationResult>) {
        let (tx, rx) = tokio::sync::mpsc::channel::<CompilationResult>(1);
        let app = axum::Router::new().route(
            "/api/v1/registry/compilation-callback",
            axum::routing::post(move |axum::Json(result): axum::Json<CompilationResult>| {
                let tx = tx.clone();
                async move {
                    let _ = tx.send(result).await;
                    "ok"
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/api/v1/registry/compilation-callback",
            listener.local_addr().unwrap()
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (url, rx)
    }

    #[tokio::test]
    async fn envelope_failure_sends_a_failed_callback_to_a_validated_url() {
        let (url, mut rx) = callback_capture_server().await;

        let mut job = azure_job();
        job.wasm_hash = "not-hex".to_string();
        let mut claims = claims_for(&job);
        claims.callback_url = url;

        let config = azure_config();
        let error = validate_job_envelope(&job, &claims, &config).unwrap_err();
        let returned = envelope_failure(&job, &claims, error, &config).await;
        assert!(matches!(returned, CompilerError::InvalidJob(_)));

        let delivered = rx.recv().await.unwrap();
        assert_eq!(delivered.status, CompilationStatus::Failed);
        assert_eq!(delivered.job_id, claims.job_id);
        assert_eq!(delivered.package_id, claims.package_id);
        assert!(delivered.compiled_platforms.is_empty());
        assert!(delivered.error.is_some());
    }

    #[tokio::test]
    async fn envelope_failure_with_an_invalid_callback_url_sends_nothing() {
        let mut job = azure_job();
        job.wasm_hash = "not-hex".to_string();
        let mut claims = claims_for(&job);
        // Public cleartext callback hosts never pass validation.
        claims.callback_url =
            "http://callback.example/api/v1/registry/compilation-callback".to_string();

        let config = azure_config();
        let error = validate_job_envelope(&job, &claims, &config).unwrap_err();
        let returned = envelope_failure(&job, &claims, error, &config).await;
        assert!(matches!(returned, CompilerError::InvalidJob(_)));
    }

    #[test]
    fn azure_upload_header_is_provider_aware() {
        assert_eq!(
            required_upload_headers(CompilationStorageProvider::AzureBlob),
            &[("x-ms-blob-type", "BlockBlob")]
        );
        assert!(required_upload_headers(CompilationStorageProvider::AwsS3).is_empty());
        assert!(required_upload_headers(CompilationStorageProvider::GoogleCloudStorage).is_empty());
    }

    #[test]
    fn signed_storage_url_rejects_redirect_shape_and_duplicate_auth_fields() {
        let config = azure_config();
        let path = "wasm/pkg_123/1.2.3/node.wasm";
        let duplicate_signature = format!(
            "https://flowlikedevdata.blob.core.windows.net/content/{path}?{}&sig=second",
            sas_query("r")
        );
        assert!(validate_storage_url(
            &duplicate_signature,
            CompilationStorageProvider::AzureBlob,
            StorageOperation::Download,
            path,
            Some("content"),
            &config,
        )
        .is_err());

        let cleartext = duplicate_signature.replacen("https://", "http://", 1);
        assert!(validate_storage_url(
            &cleartext,
            CompilationStorageProvider::AzureBlob,
            StorageOperation::Download,
            path,
            Some("content"),
            &config,
        )
        .is_err());
    }

    fn aws_query() -> &'static str {
        "X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=key%2F20250101%2Fauto%2Fs3%2Faws4_request&X-Amz-Date=20250101T000000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=deadbeef"
    }

    fn check_s3(url: &str, config: &CompilerConfig) -> Result<(), CompilerError> {
        let path = "wasm/pkg_123/1.2.3/node.wasm";
        validate_storage_url(
            url,
            CompilationStorageProvider::AwsS3,
            StorageOperation::Download,
            path,
            None,
            config,
        )
    }

    #[test]
    fn allowlisted_origins_admit_s3_compatible_endpoints() {
        let path = "wasm/pkg_123/1.2.3/node.wasm";
        let r2 = format!("https://bucket.r2.example.com/{path}?{}", aws_query());
        let minio = format!("http://minio:9000/bucket/{path}?{}", aws_query());

        let default = CompilerConfig::default();
        assert!(check_s3(&r2, &default).is_err());
        assert!(check_s3(&minio, &default).is_err());

        let allowed = CompilerConfig {
            allowed_storage_hosts: crate::config::parse_allowed_storage_origins(
                "bucket.r2.example.com, http://minio:9000",
            ),
            ..CompilerConfig::default()
        };
        assert!(check_s3(&r2, &allowed).is_ok());
        assert!(check_s3(&minio, &allowed).is_ok());
        assert!(allowed.allows_plaintext_storage());

        // A bare host entry only allows HTTPS on 443, never a downgrade.
        let downgraded = r2.replacen("https://", "http://", 1);
        assert!(check_s3(&downgraded, &allowed).is_err());
        let off_port = r2.replacen("bucket.r2.example.com", "bucket.r2.example.com:8443", 1);
        assert!(check_s3(&off_port, &allowed).is_err());
    }

    #[test]
    fn public_aws_endpoints_need_no_allowlist_and_keep_transport_rules() {
        let path = "wasm/pkg_123/1.2.3/node.wasm";
        let public = format!(
            "https://bucket.s3.eu-central-1.amazonaws.com/{path}?{}",
            aws_query()
        );
        assert!(check_s3(&public, &CompilerConfig::default()).is_ok());
        assert!(!CompilerConfig::default().allows_plaintext_storage());

        let cleartext = public.replacen("https://", "http://", 1);
        assert!(check_s3(&cleartext, &CompilerConfig::default()).is_err());

        let ip = format!("https://203.0.113.10/{path}?{}", aws_query());
        let by_ip = CompilerConfig {
            allowed_storage_hosts: crate::config::parse_allowed_storage_origins(
                "https://203.0.113.10",
            ),
            ..CompilerConfig::default()
        };
        assert!(check_s3(&ip, &by_ip).is_err());

        let fragment = format!("{public}#frag");
        assert!(check_s3(&fragment, &CompilerConfig::default()).is_err());
    }

    #[test]
    fn s3_express_zonal_endpoint_needs_no_allowlist() {
        let path = "wasm/pkg_123/1.2.3/node.wasm";
        let express = format!(
            "https://flow-meta-bucket-prod--euw1-az1--x-s3.s3express-euw1-az1.eu-west-1.amazonaws.com/{path}?{}",
            aws_query()
        );
        assert!(check_s3(&express, &CompilerConfig::default()).is_ok());

        // The AWS suffix is what makes it trusted, not the `s3express-` label.
        let lookalike = express.replacen(
            "s3express-euw1-az1.eu-west-1.amazonaws.com",
            "s3express-euw1-az1.example.com",
            1,
        );
        assert!(check_s3(&lookalike, &CompilerConfig::default()).is_err());
    }
}
