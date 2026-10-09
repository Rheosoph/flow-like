use crate::{CompilationJob, CompilerError};
use flow_like_types_contracts::dispatch::{split_claim_check_url, CompilationJobRef};
use std::time::Duration;

/// Resolve the queue-worker format with its existing 64 MiB and 60 second limits.
pub async fn resolve_job(reference: CompilationJobRef) -> Result<CompilationJob, CompilerError> {
    match reference {
        CompilationJobRef::Inline(job) => Ok(job),
        CompilationJobRef::Remote { remote_url } => {
            let body = fetch_remote(
                &remote_url,
                Some(64 * 1024 * 1024),
                Some(Duration::from_secs(60)),
            )
            .await?;
            serde_json::from_slice(&body)
                .map_err(|_| CompilerError::InvalidJob("Invalid staged job".into()))
        }
    }
}

/// AWS ECS also accepts remotely staged job arrays. Its existing timeout covers
/// response headers; it does not impose a remote-body size or read deadline.
pub async fn resolve_jobs(
    reference: CompilationJobRef,
) -> Result<Vec<CompilationJob>, CompilerError> {
    match reference {
        CompilationJobRef::Inline(job) => Ok(vec![job]),
        CompilationJobRef::Remote { remote_url } => {
            let body = fetch_remote(&remote_url, None, None).await?;
            parse_jobs(&body)
        }
    }
}

fn parse_jobs(body: &[u8]) -> Result<Vec<CompilationJob>, CompilerError> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Jobs {
        One(CompilationJob),
        Many(Vec<CompilationJob>),
    }
    match serde_json::from_slice(body) {
        Ok(Jobs::One(job)) => Ok(vec![job]),
        Ok(Jobs::Many(jobs)) => Ok(jobs),
        Err(_) => Err(CompilerError::InvalidJob("Invalid staged job".into())),
    }
}

async fn fetch_remote(
    remote_url: &str,
    max_bytes: Option<u64>,
    total_timeout: Option<Duration>,
) -> Result<Vec<u8>, CompilerError> {
    let (url, proof) = split_claim_check_url(remote_url)
        .ok_or_else(|| CompilerError::InvalidJob("Unsigned claim-check reference".into()))?;
    let claims = crate::jwt::verify_claim_check(proof).await?;
    let binding = claims.claim_check;
    if !binding.matches_url(url) {
        return Err(CompilerError::InvalidJob(
            "Invalid claim-check URL binding".into(),
        ));
    }
    let mut client = reqwest::Client::builder();
    if let Some(timeout) = total_timeout {
        client = client
            .user_agent(concat!("flow-like-executor/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(timeout);
    }
    let client = client
        .build()
        .map_err(|_| CompilerError::Config("Claim-check client failed".into()))?;
    let request = client.get(url).send();
    let response = if total_timeout.is_some() {
        request.await
    } else {
        tokio::time::timeout(Duration::from_secs(30), request)
            .await
            .map_err(|_| CompilerError::Download("Claim-check fetch timed out".into()))?
    };
    let mut response =
        response.map_err(|_| CompilerError::Download("Claim-check fetch failed".into()))?;
    if !response.status().is_success() {
        return Err(CompilerError::Download(
            "Claim-check store unavailable".into(),
        ));
    }
    if max_bytes.is_some_and(|limit| {
        response
            .content_length()
            .is_some_and(|length| length > limit)
    }) {
        return Err(CompilerError::Download(
            "Claim-check response exceeds byte limit".into(),
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| CompilerError::Download("Claim-check read failed".into()))?
    {
        if max_bytes
            .is_some_and(|limit| (body.len() as u64).saturating_add(chunk.len() as u64) > limit)
        {
            return Err(CompilerError::Download(
                "Claim-check response exceeds byte limit".into(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    if !binding.matches_body(&body) {
        return Err(CompilerError::InvalidJob(
            "Claim-check content hash mismatch".into(),
        ));
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aws_remote_bodies_keep_supporting_single_jobs_and_arrays() {
        let job = serde_json::json!({
            "job_id":"job-1", "package_id":"package-1", "version":"1.0.0",
            "wasm_download_url":"https://store.example/file", "wasm_hash":"hash",
            "wasm_download_provider":"aws_s3",
            "targets":[], "compiler_jwt":"jwt"
        });
        assert_eq!(
            parse_jobs(&serde_json::to_vec(&job).unwrap())
                .unwrap()
                .len(),
            1
        );
        let jobs = parse_jobs(&serde_json::to_vec(&vec![job.clone(), job]).unwrap()).unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].job_id, "job-1");
        assert!(parse_jobs(b"[]").unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_old_http_reader_omits_the_new_proof_fragment() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/object?signature=original",
            listener.local_addr().unwrap()
        );
        let signed =
            flow_like_types_contracts::dispatch::signed_claim_check_url(&url, "signed.proof.value");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let size = stream.read(&mut request).await.unwrap();
            let request = std::str::from_utf8(&request[..size]).unwrap();
            assert!(request.starts_with("GET /object?signature=original HTTP/1.1\r\n"));
            assert!(!request.contains("signed.proof.value"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nstaged",
                )
                .await
                .unwrap();
        });
        assert_eq!(
            reqwest::get(&signed).await.unwrap().text().await.unwrap(),
            "staged"
        );
        server.await.unwrap();
    }
}
