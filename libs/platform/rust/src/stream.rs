use crate::{Client, Error, Method, RequestOptions, Result};
use bytes::Bytes;
use futures_util::{Stream, StreamExt, stream::BoxStream};
use std::{
    collections::VecDeque,
    pin::Pin,
    task::{Context, Poll},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
    pub id: Option<String>,
    pub retry: Option<u64>,
}

pub struct EventStream(BoxStream<'static, Result<SseEvent>>);
impl Stream for EventStream {
    type Item = Result<SseEvent>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.as_mut().poll_next(cx)
    }
}

struct Decoder {
    input: BoxStream<'static, std::result::Result<Bytes, reqwest::Error>>,
    buffer: Vec<u8>,
    current: SseEvent,
    has_data: bool,
    size: usize,
    ready: VecDeque<SseEvent>,
    ended: bool,
}

impl Decoder {
    fn line(&mut self, bytes: &[u8]) -> Result<()> {
        let line = std::str::from_utf8(bytes).map_err(|_| Error::Stream("Invalid UTF-8".into()))?;
        if line.is_empty() {
            self.flush();
            return Ok(());
        }
        if line.starts_with(':') {
            return Ok(());
        }
        self.size += bytes.len();
        if self.size > 8 * 1024 * 1024 {
            return Err(Error::Stream("An SSE event exceeded 8 MiB".into()));
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.current.event = Some(value.into()),
            "data" => {
                if self.has_data {
                    self.current.data.push('\n');
                }
                self.current.data.push_str(value);
                self.has_data = true;
            }
            "id" if !value.contains('\0') => self.current.id = Some(value.into()),
            "retry" => self.current.retry = value.parse().ok(),
            _ => {}
        }
        Ok(())
    }
    fn flush(&mut self) {
        let id = self.current.id.clone();
        if self.has_data {
            self.ready.push_back(std::mem::take(&mut self.current));
        }
        self.current = SseEvent {
            id,
            ..Default::default()
        };
        self.has_data = false;
        self.size = 0;
    }
}

impl Client {
    pub async fn stream_sse(
        &self,
        method: Method,
        segments: &[&str],
        options: &RequestOptions,
    ) -> Result<EventStream> {
        let mut options = options.clone();
        options.headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("text/event-stream"),
        );
        let response = self.request_raw(method, segments, &options).await?;
        if !response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(';')
                    .next()
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("text/event-stream"))
            })
        {
            return Err(Error::Stream("Expected text/event-stream response".into()));
        }
        let state = Decoder {
            input: response.bytes_stream().boxed(),
            buffer: Vec::new(),
            current: SseEvent::default(),
            has_data: false,
            size: 0,
            ready: VecDeque::new(),
            ended: false,
        };
        Ok(EventStream(
            futures_util::stream::try_unfold(state, |mut state| async move {
                loop {
                    if let Some(event) = state.ready.pop_front() {
                        return Ok(Some((event, state)));
                    }
                    if state.ended {
                        return Ok(None);
                    }
                    match state.input.next().await {
                        Some(bytes) => {
                            state.buffer.extend_from_slice(&bytes?);
                            while let Some(end) = state
                                .buffer
                                .iter()
                                .position(|byte| matches!(byte, b'\n' | b'\r'))
                            {
                                // Keep a trailing CR until the next chunk decides CRLF versus CR.
                                if state.buffer[end] == b'\r' && end + 1 == state.buffer.len() {
                                    break;
                                }
                                let separator = if state.buffer[end] == b'\r'
                                    && state.buffer.get(end + 1) == Some(&b'\n')
                                {
                                    2
                                } else {
                                    1
                                };
                                let mut line =
                                    state.buffer.drain(..end + separator).collect::<Vec<_>>();
                                line.truncate(end);
                                state.line(&line)?;
                            }
                            if state.buffer.len() + state.size > 8 * 1024 * 1024 {
                                return Err(Error::Stream("An SSE event exceeded 8 MiB".into()));
                            }
                        }
                        None => {
                            let mut line = std::mem::take(&mut state.buffer);
                            if line.last() == Some(&b'\r') {
                                line.pop();
                            }
                            if !line.is_empty() {
                                state.line(&line)?;
                            }
                            state.flush();
                            state.ended = true;
                        }
                    }
                }
            })
            .boxed(),
        ))
    }
}
