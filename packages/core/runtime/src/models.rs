use crate::bit::Bit;

pub mod custom_bit_relay;
pub mod device;
pub mod embedding;
pub mod embedding_factory;
mod factory_cache;
pub mod generation;
pub mod image_embedding;
pub mod llm;
pub mod local_utils;
pub mod media;
#[cfg(feature = "local-stt")]
pub mod stt;
pub mod systemone;
#[cfg(feature = "local-tts")]
pub mod tts;

pub trait ModelMeta: Send + Sync {
    fn get_bit(&self) -> Bit;
}

#[cfg(test)]
pub(crate) mod test_http {
    use std::str;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    pub(crate) struct CapturedRequest {
        pub(crate) request_line: String,
        pub(crate) headers: String,
        pub(crate) body: flow_like_types::Value,
    }

    /// Reads one JSON request from `listener` and answers it with HTTP 400.
    pub(crate) async fn capture_one_http_request(listener: TcpListener) -> CapturedRequest {
        let (mut stream, _) = listener.accept().await.expect("accept proxy request");
        let (bytes, header_end) = read_request(&mut stream).await;
        stream
            .write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\nstop",
            )
            .await
            .expect("write mock response");

        let headers = str::from_utf8(&bytes[..header_end])
            .expect("UTF-8 request headers")
            .to_string();
        CapturedRequest {
            request_line: headers.lines().next().expect("request line").to_string(),
            body: flow_like_types::json::from_slice(&bytes[header_end..])
                .expect("JSON request body"),
            headers,
        }
    }

    /// One request up to the end of its declared body, and where its headers end.
    async fn read_request(stream: &mut TcpStream) -> (Vec<u8>, usize) {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            if let Some(header_end) = header_end(&bytes) {
                let end = header_end + content_length(&bytes[..header_end]);
                if bytes.len() >= end {
                    bytes.truncate(end);
                    return (bytes, header_end);
                }
            }
            let read = stream.read(&mut buffer).await.expect("read proxy request");
            assert!(
                read > 0,
                "connection closed before the request was complete"
            );
            bytes.extend_from_slice(&buffer[..read]);
        }
    }

    fn header_end(bytes: &[u8]) -> Option<usize> {
        bytes
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|position| position + 4)
    }

    fn content_length(headers: &[u8]) -> usize {
        str::from_utf8(headers)
            .expect("UTF-8 request headers")
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().expect("numeric content length"))
            })
            .unwrap_or_default()
    }
}
