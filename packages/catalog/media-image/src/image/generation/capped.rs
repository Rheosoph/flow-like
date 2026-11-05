use flow_like_types::{anyhow, bail, reqwest};

pub const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_ERROR_CHARS: usize = 2048;

/// Reads a provider response without holding more than `limit` bytes, whatever the endpoint sends.
pub(super) async fn read_capped(
    mut response: reqwest::Response,
    limit: usize,
) -> flow_like_types::Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        bail!(
            "The image provider's response is larger than {} MiB",
            limit / (1024 * 1024)
        );
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)?
    {
        if body.len() + chunk.len() > limit {
            return Err(anyhow!(
                "The image provider's response is larger than {} MiB",
                limit / (1024 * 1024)
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// A provider's error body, cut to what a message can carry.
pub(super) fn error_text(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let mut shown: String = text.chars().take(MAX_ERROR_CHARS).collect();
    if text.chars().count() > MAX_ERROR_CHARS {
        shown.push_str("...");
    }
    shown
}

/// Refuses a result whose images or total size pass the caps.
pub(super) fn ensure_within_caps(
    sizes: impl IntoIterator<Item = usize>,
) -> flow_like_types::Result<()> {
    let mut total = 0usize;
    for size in sizes {
        if size > MAX_IMAGE_BYTES {
            bail!(
                "A generated image is larger than {} MiB",
                MAX_IMAGE_BYTES / (1024 * 1024)
            );
        }
        total = total.saturating_add(size);
    }
    if total > MAX_RESPONSE_BYTES {
        bail!(
            "The generated images add up to more than {} MiB",
            MAX_RESPONSE_BYTES / (1024 * 1024)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn serve(body_len: usize, declare: bool) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request).await;
            let head = if declare {
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {body_len}\r\nconnection: close\r\n\r\n"
                )
            } else {
                "HTTP/1.1 200 OK\r\nconnection: close\r\n\r\n".to_string()
            };
            let _ = socket.write_all(head.as_bytes()).await;
            let chunk = vec![b'x'; 8192];
            let mut sent = 0;
            while sent < body_len && socket.write_all(&chunk).await.is_ok() {
                sent += chunk.len();
            }
        });
        format!("http://{address}/")
    }

    #[tokio::test]
    async fn a_response_within_the_limit_is_read_whole() {
        let url = serve(10_000, true).await;
        let response = reqwest::Client::new().get(&url).send().await.unwrap();
        assert_eq!(
            read_capped(response, 64 * 1024).await.unwrap().len(),
            10_000
        );
    }

    #[tokio::test]
    async fn a_declared_oversize_response_is_refused_before_reading() {
        let url = serve(200_000, true).await;
        let response = reqwest::Client::new().get(&url).send().await.unwrap();
        let error = read_capped(response, 64 * 1024).await.unwrap_err();
        assert!(error.to_string().contains("larger than"));
    }

    #[tokio::test]
    async fn an_undeclared_endless_body_stops_at_the_limit() {
        let url = serve(50 * 1024 * 1024, false).await;
        let response = reqwest::Client::new().get(&url).send().await.unwrap();
        assert!(read_capped(response, 64 * 1024).await.is_err());
    }

    #[test]
    fn error_bodies_are_cut_for_messages() {
        let long = vec![b'a'; 10_000];
        let shown = error_text(&long);
        assert_eq!(shown.chars().count(), MAX_ERROR_CHARS + 3);
        assert_eq!(error_text(b"short"), "short");
    }

    #[test]
    fn image_and_total_caps_hold() {
        assert!(ensure_within_caps([MAX_IMAGE_BYTES, MAX_IMAGE_BYTES]).is_ok());
        assert!(ensure_within_caps([MAX_IMAGE_BYTES + 1]).is_err());
        assert!(ensure_within_caps([MAX_IMAGE_BYTES, MAX_IMAGE_BYTES, 1]).is_err());
    }
}
