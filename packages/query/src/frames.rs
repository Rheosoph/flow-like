//! Response framing: `kind: u8 | length: u32 big-endian | payload`. Arrow frames are chunks of
//! one Arrow IPC stream, so rows are typed and LZ4-compressed instead of JSON text.

use crate::protocol::{Trailer, WireError};
use arrow_ipc::reader::StreamReader;
use bytes::{Buf, Bytes, BytesMut};
use flow_like_storage::arrow_array::RecordBatch;
use flow_like_storage::arrow_schema::SchemaRef;
use flow_like_storage::arrow_utils::record_batch_to_value;
use flow_like_storage::databases::workbench::{SqlColumn, SqlQueryResult};
use flow_like_types::Value;

/// Larger frames are refused rather than buffered.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
const HEADER_BYTES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameKind {
    Arrow = 1,
    Json = 2,
    Trailer = 3,
    Error = 4,
}

impl TryFrom<u8> for FrameKind {
    type Error = FrameError;

    fn try_from(value: u8) -> Result<Self, FrameError> {
        Ok(match value {
            1 => Self::Arrow,
            2 => Self::Json,
            3 => Self::Trailer,
            4 => Self::Error,
            other => return Err(FrameError::Malformed(format!("unknown frame kind {other}"))),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("Malformed query service response: {0}")]
    Malformed(String),
    #[error("{0}")]
    Remote(WireError),
    #[error("The query service response ended before its trailer")]
    Truncated,
}

pub fn encode_frame(kind: FrameKind, payload: &[u8]) -> Bytes {
    let mut frame = BytesMut::with_capacity(HEADER_BYTES + payload.len());
    frame.extend_from_slice(&[kind as u8]);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    frame.freeze()
}

pub fn encode_json_frame<T: serde::Serialize>(kind: FrameKind, value: &T) -> Bytes {
    match serde_json::to_vec(value) {
        Ok(payload) => encode_frame(kind, &payload),
        Err(error) => encode_frame(
            FrameKind::Error,
            format!(
                r#"{{"code":"internal","message":"Encoding a response frame failed: {}"}}"#,
                error.to_string().replace('"', "'")
            )
            .as_bytes(),
        ),
    }
}

/// Splits a byte stream into frames, whatever the chunk boundaries.
#[derive(Default)]
pub struct FrameDecoder {
    buffer: BytesMut,
}

impl FrameDecoder {
    pub fn push(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
    }

    pub fn next_frame(&mut self) -> Result<Option<(FrameKind, Bytes)>, FrameError> {
        if self.buffer.len() < HEADER_BYTES {
            return Ok(None);
        }
        let kind = FrameKind::try_from(self.buffer[0])?;
        let length = u32::from_be_bytes([
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
            self.buffer[4],
        ]) as usize;
        if length > MAX_FRAME_BYTES {
            return Err(FrameError::Malformed(format!(
                "frame of {length} bytes exceeds the {MAX_FRAME_BYTES} byte limit"
            )));
        }
        if self.buffer.len() < HEADER_BYTES + length {
            return Ok(None);
        }
        self.buffer.advance(HEADER_BYTES);
        Ok(Some((kind, self.buffer.split_to(length).freeze())))
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

/// A complete, decoded response.
#[derive(Debug, Default)]
pub struct QueryResponse {
    pub schema: Option<SchemaRef>,
    pub batches: Vec<RecordBatch>,
    pub json: Option<Value>,
    pub trailer: Trailer,
}

/// Collects frames into a [`QueryResponse`]; an error frame ends it with that error.
#[derive(Default)]
pub struct ResponseAssembler {
    decoder: FrameDecoder,
    arrow: Vec<u8>,
    json: Option<Value>,
    trailer: Option<Trailer>,
}

impl ResponseAssembler {
    pub fn push(&mut self, chunk: &[u8]) -> Result<(), FrameError> {
        self.decoder.push(chunk);
        while let Some((kind, payload)) = self.decoder.next_frame()? {
            if self.trailer.is_some() {
                return Err(FrameError::Malformed("a frame follows the trailer".into()));
            }
            match kind {
                FrameKind::Arrow => self.arrow.extend_from_slice(&payload),
                FrameKind::Json => {
                    self.json = Some(serde_json::from_slice(&payload).map_err(|error| {
                        FrameError::Malformed(format!("invalid JSON frame: {error}"))
                    })?);
                }
                FrameKind::Trailer => {
                    self.trailer = Some(serde_json::from_slice(&payload).map_err(|error| {
                        FrameError::Malformed(format!("invalid trailer: {error}"))
                    })?);
                }
                FrameKind::Error => {
                    let error: WireError = serde_json::from_slice(&payload).map_err(|error| {
                        FrameError::Malformed(format!("invalid error frame: {error}"))
                    })?;
                    return Err(FrameError::Remote(error));
                }
            }
        }
        Ok(())
    }

    pub fn finish(self) -> Result<QueryResponse, FrameError> {
        if !self.decoder.is_empty() {
            return Err(FrameError::Truncated);
        }
        let trailer = self.trailer.ok_or(FrameError::Truncated)?;
        let (schema, batches) = if self.arrow.is_empty() {
            (None, Vec::new())
        } else {
            let reader = StreamReader::try_new(std::io::Cursor::new(self.arrow), None)
                .map_err(|error| FrameError::Malformed(format!("invalid Arrow stream: {error}")))?;
            let schema = reader.schema();
            let batches = reader
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| FrameError::Malformed(format!("invalid Arrow batch: {error}")))?;
            (Some(schema), batches)
        };
        Ok(QueryResponse {
            schema,
            batches,
            json: self.json,
            trailer,
        })
    }
}

impl QueryResponse {
    pub fn columns(&self) -> Vec<SqlColumn> {
        self.schema
            .as_ref()
            .map(|schema| {
                schema
                    .fields()
                    .iter()
                    .enumerate()
                    .map(|(position, field)| SqlColumn {
                        name: field.name().to_string(),
                        type_name: field.data_type().to_string(),
                        position,
                        metadata: field.metadata().clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The JSON shape the workbench routes have always returned.
    pub fn to_sql_result(&self) -> flow_like_types::Result<SqlQueryResult> {
        let mut rows = Vec::with_capacity(self.trailer.row_count);
        for batch in &self.batches {
            rows.extend(record_batch_to_value(batch)?);
        }
        Ok(SqlQueryResult {
            columns: self.columns(),
            row_count: rows.len(),
            truncated: self.trailer.truncated,
            rows,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ErrorCode;

    #[test]
    fn frames_survive_any_chunking() {
        let mut stream = Vec::new();
        stream.extend_from_slice(&encode_frame(FrameKind::Json, br#"{"a":1}"#));
        stream.extend_from_slice(&encode_json_frame(
            FrameKind::Trailer,
            &Trailer {
                row_count: 0,
                elapsed_ms: 3,
                ..Trailer::default()
            },
        ));
        for chunk_size in [1, 2, 3, 7, stream.len()] {
            let mut assembler = ResponseAssembler::default();
            for chunk in stream.chunks(chunk_size) {
                assembler.push(chunk).unwrap();
            }
            let response = assembler.finish().unwrap();
            assert_eq!(response.json, Some(serde_json::json!({ "a": 1 })));
            assert_eq!(response.trailer.elapsed_ms, 3);
        }
    }

    #[test]
    fn error_frames_and_missing_trailers_fail() {
        let mut assembler = ResponseAssembler::default();
        let error = assembler
            .push(&encode_json_frame(
                FrameKind::Error,
                &WireError::new(ErrorCode::Timeout, "slow"),
            ))
            .unwrap_err();
        assert!(matches!(
            error,
            FrameError::Remote(WireError {
                code: ErrorCode::Timeout,
                ..
            })
        ));

        let mut unfinished = ResponseAssembler::default();
        unfinished
            .push(&encode_frame(FrameKind::Json, b"{}")[..4])
            .unwrap();
        assert!(matches!(unfinished.finish(), Err(FrameError::Truncated)));
    }

    #[test]
    fn oversized_and_unknown_frames_are_refused() {
        let mut decoder = FrameDecoder::default();
        decoder.push(&[9, 0, 0, 0, 0]);
        assert!(decoder.next_frame().is_err());
        let mut decoder = FrameDecoder::default();
        decoder.push(&[1, 0xff, 0xff, 0xff, 0xff]);
        assert!(decoder.next_frame().is_err());
    }
}
