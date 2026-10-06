use crate::error::ApiError;
use flow_like_types::reqwest::Response;

/// Read only the prefix needed for metadata, then drop the upstream response.
pub(crate) async fn read_prefix(mut response: Response, limit: usize) -> Result<Vec<u8>, ApiError> {
    let mut body = Vec::new();
    while body.len() < limit {
        let Some(chunk) = response.chunk().await.map_err(|error| {
            ApiError::bad_gateway(format!(
                "Failed to read upstream response: {}",
                error.without_url()
            ))
        })?
        else {
            break;
        };
        let length = chunk.len().min(limit - body.len());
        body.extend_from_slice(&chunk[..length]);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[tokio::test]
    async fn metadata_prefix_stops_without_requiring_the_remaining_body() {
        let stream = futures::stream::iter([
            Ok::<_, std::io::Error>(Bytes::from_static(b"1234")),
            Ok(Bytes::from_static(b"5678")),
            Err(std::io::Error::other("remaining body must not be read")),
        ]);
        let response = axum::http::Response::builder()
            .header("Content-Length", "1000000")
            .body(reqwest::Body::wrap_stream(stream))
            .unwrap();
        assert_eq!(
            read_prefix(Response::from(response), 7).await.unwrap(),
            b"1234567"
        );
    }
}
