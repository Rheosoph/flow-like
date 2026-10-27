use rmcp::transport::{
    StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
};

fn client() -> Result<reqwest_mcp::Client, reqwest_mcp::Error> {
    reqwest_mcp::Client::builder()
        .pool_max_idle_per_host(0)
        // Registration headers can contain credentials. RMCP 1.7's default client
        // forwards these headers to cross-origin redirect targets.
        .redirect(reqwest_mcp::redirect::Policy::none())
        .build()
}

pub(super) fn transport(
    config: StreamableHttpClientTransportConfig,
) -> Result<StreamableHttpClientTransport<reqwest_mcp::Client>, reqwest_mcp::Error> {
    Ok(StreamableHttpClientTransport::with_client(
        client()?,
        config,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_mcp::header::{HeaderName, HeaderValue};
    use rmcp::{
        model::{ClientJsonRpcMessage, ClientRequest, PingRequest, RequestId},
        transport::streamable_http_client::StreamableHttpClient,
    };
    use std::{collections::HashMap, sync::Arc, time::Duration};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[tokio::test]
    async fn custom_auth_headers_do_not_follow_redirects() {
        for status in [307, 308] {
            let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let destination_addr = destination.local_addr().unwrap();
            let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin_addr = origin.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = origin.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let mut chunk = [0; 1024];
                    let length = socket.read(&mut chunk).await.unwrap();
                    assert!(length > 0 && request.len() + length <= 8192);
                    request.extend_from_slice(&chunk[..length]);
                }
                let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
                assert!(request.contains("x-api-key: test-registration-token"));
                socket.write_all(format!(
                    "HTTP/1.1 {status} Redirect\r\nLocation: http://{destination_addr}/capture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                ).as_bytes()).await.unwrap();
            });
            let message = ClientJsonRpcMessage::request(
                ClientRequest::PingRequest(PingRequest::default()),
                RequestId::Number(1),
            );
            let headers = HashMap::from([(
                HeaderName::from_static("x-api-key"),
                HeaderValue::from_static("test-registration-token"),
            )]);
            let result = tokio::time::timeout(
                Duration::from_secs(2),
                client().unwrap().post_message(
                    Arc::from(format!("http://{origin_addr}/mcp")),
                    message,
                    None,
                    None,
                    headers,
                ),
            )
            .await
            .expect("the redirect response must finish without contacting its target");
            assert!(result.is_err());
            server.await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(50), destination.accept())
                    .await
                    .is_err()
            );
        }
    }
}
