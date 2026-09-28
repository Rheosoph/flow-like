use crate::{IngestApi, IngestRequest, Result, invalid};
use std::{sync::Arc, time::Duration};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};

const LINE_LIMIT: usize = 65_536;
const COMMAND_LIMIT: usize = 1_000;
const RECIPIENT_LIMIT: usize = 100;
const READ_TIMEOUT: Duration = Duration::from_secs(30);

pub struct LmtpConfig {
    pub hostname: String,
    pub max_bytes: usize,
}

async fn bounded_line<R: AsyncBufRead + Unpin>(reader: &mut R, limit: usize) -> Result<Vec<u8>> {
    tokio::time::timeout(READ_TIMEOUT, async {
        let mut line = Vec::new();
        loop {
            let available = reader.fill_buf().await?;
            if available.is_empty() {
                return Err(invalid("Unexpected connection close"));
            }
            let count = available
                .iter()
                .position(|b| *b == b'\n')
                .map_or(available.len(), |index| index + 1);
            if line.len() + count > limit {
                return Err(invalid("Protocol line too long"));
            }
            line.extend_from_slice(&available[..count]);
            reader.consume(count);
            if line.ends_with(b"\n") {
                if !line.ends_with(b"\r\n") {
                    return Err(invalid("Protocol requires CRLF"));
                }
                return Ok(line);
            }
        }
    })
    .await?
}

fn envelope_path<'a>(command: &'a str, prefix: &str, allow_empty: bool) -> Option<&'a str> {
    if !command.get(..prefix.len())?.eq_ignore_ascii_case(prefix) {
        return None;
    }
    let rest = command[prefix.len()..].trim_start();
    let rest = rest.strip_prefix('<')?;
    let (address, options) = rest.split_once('>')?;
    if (!allow_empty && address.is_empty())
        || address.len() > 320
        || !address.is_ascii()
        || address.bytes().any(|b| b.is_ascii_control())
        || (!address.is_empty() && !address.contains('@'))
    {
        return None;
    }
    // Only extensions advertised by this LMTP server are accepted.
    for option in options.split_whitespace() {
        let upper = option.to_ascii_uppercase();
        if !allow_empty
            || !(upper == "BODY=8BITMIME"
                || upper == "BODY=7BIT"
                || upper
                    .strip_prefix("SIZE=")
                    .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())))
        {
            return None;
        }
    }
    Some(address)
}

fn trusted_received_header(raw: &[u8], hostname: &str) -> bool {
    let text = String::from_utf8_lossy(raw);
    let mut lines = text.split("\r\n");
    let Some(first) = lines.next() else {
        return false;
    };
    if !first
        .get(..9)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Received:"))
    {
        return false;
    }
    let mut header = first.to_string();
    for line in lines {
        if !line.starts_with([' ', '\t']) {
            break;
        }
        header.push(' ');
        header.push_str(line.trim());
    }
    let words: Vec<_> = header.split_whitespace().collect();
    words
        .windows(2)
        .any(|w| w[0] == "by" && w[1].eq_ignore_ascii_case(hostname))
        && words.windows(2).any(|w| {
            w[0] == "id"
                && w[1]
                    .trim_end_matches(';')
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric())
        })
}

pub async fn lmtp_session<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    api: &dyn IngestApi,
    config: &LmtpConfig,
) -> Result<()> {
    let (read, mut write) = tokio::io::split(stream);
    let mut read = BufReader::new(read);
    write.write_all(b"220 flow-like LMTP ready\r\n").await?;
    let mut greeted = false;
    let mut sender: Option<String> = None;
    let mut recipients = Vec::new();
    for _ in 0..COMMAND_LIMIT {
        let line = bounded_line(&mut read, 1024).await?;
        let command = std::str::from_utf8(&line[..line.len() - 2])?;
        let verb = command
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        match verb.as_str() {
            "LHLO" if command.split_whitespace().count() == 2 => {
                greeted = true;
                sender = None;
                recipients.clear();
                write
                    .write_all(
                        format!(
                            "250-flow-like\r\n250-SIZE {}\r\n250 8BITMIME\r\n",
                            config.max_bytes
                        )
                        .as_bytes(),
                    )
                    .await?;
            }
            "QUIT" => {
                write.write_all(b"221 Bye\r\n").await?;
                return Ok(());
            }
            "NOOP" => {
                write.write_all(b"250 OK\r\n").await?;
            }
            "RSET" => {
                sender = None;
                recipients.clear();
                write.write_all(b"250 Reset\r\n").await?;
            }
            "MAIL" if greeted && sender.is_none() => {
                if let Some(value) = envelope_path(command, "MAIL FROM:", true) {
                    sender = Some(value.to_string());
                    write.write_all(b"250 Sender accepted\r\n").await?;
                } else {
                    write
                        .write_all(b"501 Invalid sender or options\r\n")
                        .await?;
                }
            }
            "RCPT" if sender.is_some() => {
                if recipients.len() >= RECIPIENT_LIMIT {
                    write.write_all(b"452 Too many recipients\r\n").await?;
                } else if let Some(value) = envelope_path(command, "RCPT TO:", false) {
                    match api.accepts(value).await {
                        Ok(true) => {
                            recipients.push(value.to_string());
                            write.write_all(b"250 Recipient accepted\r\n").await?;
                        }
                        Ok(false) => {
                            write.write_all(b"550 Unknown recipient\r\n").await?;
                        }
                        Err(error) => {
                            tracing::warn!(%error, "Recipient lookup failed");
                            write
                                .write_all(b"451 Recipient lookup unavailable\r\n")
                                .await?;
                        }
                    }
                } else {
                    write.write_all(b"501 Invalid recipient\r\n").await?;
                }
            }
            "DATA" if command.eq_ignore_ascii_case("DATA") && !recipients.is_empty() => {
                write.write_all(b"354 End with <CRLF>.<CRLF>\r\n").await?;
                let mut raw = Vec::new();
                let mut oversized = false;
                loop {
                    let line = bounded_line(&mut read, LINE_LIMIT).await?;
                    if line == b".\r\n" {
                        break;
                    }
                    let content = if line.starts_with(b".") {
                        &line[1..]
                    } else {
                        &line
                    };
                    if raw.len().saturating_add(content.len()) > config.max_bytes {
                        oversized = true;
                    }
                    if !oversized {
                        raw.extend_from_slice(content);
                    }
                }
                let trusted = trusted_received_header(&raw, &config.hostname);
                for recipient in &recipients {
                    let response = if oversized {
                        "552 Message exceeds size limit\r\n"
                    } else if !trusted {
                        "451 Expected trusted Postfix Received header\r\n"
                    } else {
                        let request = IngestRequest::postfix(
                            &raw,
                            sender.as_deref().unwrap_or(""),
                            recipient,
                        );
                        match api.ingest(&request).await {
                            Ok(()) => "250 Message stored\r\n",
                            Err(error) => {
                                tracing::warn!(%error, "Mail ingestion failed; Postfix will retry");
                                "451 Message storage unavailable\r\n"
                            }
                        }
                    };
                    // LMTP returns one delivery result per accepted RCPT, in order.
                    write.write_all(response.as_bytes()).await?;
                }
                sender = None;
                recipients.clear();
            }
            "MAIL" | "RCPT" | "DATA" => {
                write.write_all(b"503 Bad command sequence\r\n").await?;
            }
            _ => {
                write.write_all(b"500 Unsupported command\r\n").await?;
            }
        }
    }
    write
        .write_all(b"421 Session command limit reached\r\n")
        .await?;
    Ok(())
}

async fn socketmap_request<R: AsyncRead + Unpin>(read: &mut R) -> Result<Option<Vec<u8>>> {
    let mut length = 0usize;
    for index in 0..5 {
        let byte = match read.read_u8().await {
            Ok(value) => value,
            Err(error) if index == 0 && error.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        if byte == b':' {
            if index == 0 || length > 512 {
                return Err(invalid("Invalid socketmap length"));
            }
            let mut data = vec![0; length];
            read.read_exact(&mut data).await?;
            if read.read_u8().await? != b',' {
                return Err(invalid("Invalid socketmap terminator"));
            }
            return Ok(Some(data));
        }
        if !byte.is_ascii_digit() {
            return Err(invalid("Invalid socketmap length"));
        }
        length = length * 10 + usize::from(byte - b'0');
    }
    Err(invalid("Socketmap request too large"))
}

pub async fn socketmap_session<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    api: &dyn IngestApi,
) -> Result<()> {
    for _ in 0..COMMAND_LIMIT {
        let Some(request) =
            tokio::time::timeout(READ_TIMEOUT, socketmap_request(&mut stream)).await??
        else {
            return Ok(());
        };
        let query = std::str::from_utf8(&request)?;
        let response = if let Some(recipient) = query.strip_prefix("recipients ") {
            if recipient.is_empty()
                || recipient.len() > 320
                || recipient.bytes().any(|b| b.is_ascii_control())
            {
                "NOTFOUND "
            } else {
                match api.accepts(recipient).await {
                    Ok(true) => "OK 1",
                    Ok(false) => "NOTFOUND ",
                    Err(error) => {
                        tracing::warn!(%error, "Socketmap recipient lookup failed");
                        "TEMP Recipient lookup unavailable"
                    }
                }
            }
        } else {
            "PERM Unknown map"
        };
        stream
            .write_all(format!("{}:{response},", response.len()).as_bytes())
            .await?;
    }
    Ok(())
}

pub async fn serve(
    address: &str,
    api: Arc<dyn IngestApi>,
    config: Arc<LmtpConfig>,
    socketmap: bool,
) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(address).await?;
    let permits = Arc::new(tokio::sync::Semaphore::new(64));
    loop {
        let permit = permits.clone().acquire_owned().await?;
        let (stream, _) = listener.accept().await?;
        let api = api.clone();
        let config = config.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let result = tokio::time::timeout(Duration::from_secs(300), async {
                if socketmap {
                    socketmap_session(stream, api.as_ref()).await
                } else {
                    lmtp_session(stream, api.as_ref(), &config).await
                }
            })
            .await;
            if !matches!(result, Ok(Ok(()))) {
                tracing::debug!(?result, "Mail gateway session ended");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeApi {
        messages: Mutex<Vec<(String, Vec<u8>)>>,
    }
    #[async_trait]
    impl IngestApi for FakeApi {
        async fn accepts(&self, recipient: &str) -> Result<bool> {
            if recipient == "lookup-error@example.test" {
                return Err(invalid("offline"));
            }
            Ok(recipient != "missing@example.test")
        }
        async fn ingest(&self, request: &IngestRequest) -> Result<()> {
            if request.recipients[0] == "retry@example.test" {
                return Err(invalid("offline"));
            }
            self.messages.lock().unwrap().push((
                request.recipients[0].clone(),
                STANDARD
                    .decode(request.raw_mime_base64.as_ref().unwrap())
                    .unwrap(),
            ));
            Ok(())
        }
    }

    async fn run_lmtp(input: &[u8], max_bytes: usize) -> (String, Arc<FakeApi>) {
        let (mut client, server) = tokio::io::duplex(65_536);
        let api = Arc::new(FakeApi::default());
        let service = api.clone();
        let task = tokio::spawn(async move {
            lmtp_session(
                server,
                service.as_ref(),
                &LmtpConfig {
                    hostname: "mx.example.test".into(),
                    max_bytes,
                },
            )
            .await
        });
        client.write_all(input).await.unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();
        task.await.unwrap().unwrap();
        (response, api)
    }

    #[tokio::test]
    async fn dot_stuffing_and_per_recipient_retry() {
        let (response, api) = run_lmtp(b"LHLO postfix\r\nMAIL FROM:<sender@example.test>\r\nRCPT TO:<one@example.test>\r\nRCPT TO:<retry@example.test>\r\nDATA\r\nReceived: from source\r\n by mx.example.test (Postfix) with ESMTP id ABC; now\r\nSubject: test\r\n\r\n..one dot\r\n.\r\nQUIT\r\n", 10_000).await;
        assert!(response.contains("250 Message stored\r\n451 Message storage unavailable\r\n"));
        let messages = api.messages.lock().unwrap();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].1.ends_with(b"\r\n.one dot\r\n"));
    }

    #[tokio::test]
    async fn refuses_unknown_recipient_and_data_before_envelope() {
        let (response, api) = run_lmtp(b"DATA\r\nLHLO postfix\r\nMAIL FROM:<>\r\nRCPT TO:<missing@example.test>\r\nRCPT TO:<lookup-error@example.test>\r\nDATA\r\nQUIT\r\n", 10_000).await;
        assert!(response.contains("550 Unknown recipient"));
        assert!(response.contains("451 Recipient lookup unavailable"));
        assert_eq!(response.matches("503 Bad command sequence").count(), 2);
        assert!(api.messages.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn refuses_oversized_data_without_ingesting() {
        let (response, api) = run_lmtp(b"LHLO postfix\r\nMAIL FROM:<>\r\nRCPT TO:<one@example.test>\r\nDATA\r\nReceived: by mx.example.test (Postfix) id ABC; now\r\n\r\nbody\r\n.\r\nQUIT\r\n", 8).await;
        assert!(response.contains("552 Message exceeds size limit"));
        assert!(api.messages.lock().unwrap().is_empty());
    }

    #[test]
    fn sender_header_cannot_replace_the_expected_gateway_header() {
        assert!(!trusted_received_header(
            b"Subject: forged\r\nReceived: by mx.example.test id ABC\r\n",
            "mx.example.test"
        ));
        assert!(!trusted_received_header(
            b"Received: by mx.example.test.evil id ABC\r\n",
            "mx.example.test"
        ));
        assert!(!trusted_received_header(
            b"Received: by mx.example.test\r\n",
            "mx.example.test"
        ));
    }

    #[tokio::test]
    async fn socketmap_distinguishes_absent_recipient_from_outage() {
        let (mut client, server) = tokio::io::duplex(4096);
        let task =
            tokio::spawn(async move { socketmap_session(server, &FakeApi::default()).await });
        for recipient in [
            "one@example.test",
            "missing@example.test",
            "lookup-error@example.test",
        ] {
            let query = format!("recipients {recipient}");
            client
                .write_all(format!("{}:{query},", query.len()).as_bytes())
                .await
                .unwrap();
        }
        client.shutdown().await.unwrap();
        let mut output = String::new();
        client.read_to_string(&mut output).await.unwrap();
        task.await.unwrap().unwrap();
        assert_eq!(
            output,
            "4:OK 1,9:NOTFOUND ,33:TEMP Recipient lookup unavailable,"
        );
    }

    #[tokio::test]
    async fn bounded_protocol_reads_reject_oversized_or_incomplete_frames() {
        let mut malformed = &b"9999:unbounded"[..];
        assert!(socketmap_request(&mut malformed).await.is_err());
        let mut line = BufReader::new(&b"0123456789\r\n"[..]);
        assert!(bounded_line(&mut line, 8).await.is_err());
    }
}
