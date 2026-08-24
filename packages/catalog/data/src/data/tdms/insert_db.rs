use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_storage::arrow_array::{
    ArrayRef, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array,
    Int64Array, RecordBatch, UInt8Array, UInt16Array, UInt32Array, UInt64Array,
    builder::StringBuilder,
};
#[cfg(feature = "execute")]
use flow_like_storage::arrow_schema::{DataType, Field, Schema};
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use futures::StreamExt;
#[cfg(feature = "execute")]
use std::io::Write;
#[cfg(feature = "execute")]
use std::path::{Path, PathBuf};
#[cfg(feature = "execute")]
use std::sync::Arc;
#[cfg(feature = "execute")]
use tdms_rs::api::reader::Pod;
#[cfg(feature = "execute")]
use tdms_rs::{DataType as TdmsDataType, TdmsChannel, TdmsFile};

use crate::data::db::vector::NodeDBConnection;
use crate::data::path::FlowPath;

#[crate::register_node]
#[derive(Default)]
pub struct BatchInsertTdmsLocalDatabaseNode {}

impl BatchInsertTdmsLocalDatabaseNode {
    pub fn new() -> Self {
        BatchInsertTdmsLocalDatabaseNode {}
    }
}

#[cfg(feature = "execute")]
const TDMS_MAX_CHUNK_SIZE: usize = 20_000;

#[cfg(feature = "execute")]
enum TdmsSourceFile {
    Direct(PathBuf),
    Temp(tempfile::NamedTempFile),
}

#[cfg(feature = "execute")]
impl TdmsSourceFile {
    fn path(&self) -> &Path {
        match self {
            Self::Direct(path) => path.as_path(),
            Self::Temp(file) => file.path(),
        }
    }
}

#[cfg(feature = "execute")]
async fn resolve_tdms_source_file(
    tdms_path: &FlowPath,
    context: &mut ExecutionContext,
) -> flow_like_types::Result<TdmsSourceFile> {
    let runtime = tdms_path.to_runtime(context).await?;
    let object_path = flow_like_storage::Path::from(runtime.path.as_ref());

    if let flow_like_storage::files::store::FlowLikeStore::Local(local_store) =
        runtime.store.as_ref()
        && let Ok(local_path) = local_store.path_to_filesystem(&object_path)
        && local_path.exists()
    {
        return Ok(TdmsSourceFile::Direct(local_path));
    }

    let mut stream = runtime
        .store
        .as_generic()
        .get(&object_path)
        .await?
        .into_stream();
    let tmp_file = tempfile::NamedTempFile::new()?;
    let mut writer = std::fs::File::create(tmp_file.path())?;

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        writer.write_all(&chunk)?;
    }
    writer.flush()?;

    Ok(TdmsSourceFile::Temp(tmp_file))
}

#[cfg(feature = "execute")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct TdmsRawTimestamp {
    first: u64,
    second: i64,
}

#[cfg(feature = "execute")]
impl Pod for TdmsRawTimestamp {}

#[cfg(feature = "execute")]
struct TdmsValueIter<'a, T>
where
    T: Pod + Copy + Default,
{
    channel: TdmsChannel<'a>,
    cursor: usize,
    buffer: Vec<T>,
    buffer_index: usize,
    read_chunk_size: usize,
}

#[cfg(feature = "execute")]
impl<'a, T> TdmsValueIter<'a, T>
where
    T: Pod + Copy + Default,
{
    fn new(channel: TdmsChannel<'a>, read_chunk_size: usize) -> Self {
        Self {
            channel,
            cursor: 0,
            buffer: Vec::new(),
            buffer_index: 0,
            read_chunk_size: read_chunk_size.max(1),
        }
    }

    fn refill_buffer(&mut self) -> bool {
        if self.cursor >= self.channel.len() {
            return false;
        }

        let end = (self.cursor + self.read_chunk_size).min(self.channel.len());
        let count = end - self.cursor;
        self.buffer.clear();
        self.buffer.resize(count, T::default());

        let read_count = match self.channel.read(self.cursor..end, &mut self.buffer) {
            Ok(read_count) => read_count,
            Err(_) => {
                self.buffer.clear();
                return false;
            }
        };

        self.buffer.truncate(read_count);
        self.cursor += read_count;
        self.buffer_index = 0;
        read_count > 0
    }
}

#[cfg(feature = "execute")]
impl<T> Iterator for TdmsValueIter<'_, T>
where
    T: Pod + Copy + Default,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buffer_index >= self.buffer.len() && !self.refill_buffer() {
            return None;
        }

        let value = self.buffer[self.buffer_index];
        self.buffer_index += 1;
        Some(value)
    }
}

#[cfg(feature = "execute")]
struct TdmsStringIter<'a> {
    channel: TdmsChannel<'a>,
    cursor: usize,
    buffer: Vec<String>,
    buffer_index: usize,
    read_chunk_size: usize,
}

#[cfg(feature = "execute")]
impl<'a> TdmsStringIter<'a> {
    fn new(channel: TdmsChannel<'a>, read_chunk_size: usize) -> Self {
        Self {
            channel,
            cursor: 0,
            buffer: Vec::new(),
            buffer_index: 0,
            read_chunk_size: read_chunk_size.max(1),
        }
    }

    fn refill_buffer(&mut self) -> bool {
        if self.cursor >= self.channel.len() {
            return false;
        }

        let end = (self.cursor + self.read_chunk_size).min(self.channel.len());
        self.buffer = match self.channel.read_strings(self.cursor..end) {
            Ok(values) => values,
            Err(_) => {
                self.buffer.clear();
                return false;
            }
        };
        self.cursor = end;
        self.buffer_index = 0;
        !self.buffer.is_empty()
    }
}

#[cfg(feature = "execute")]
impl Iterator for TdmsStringIter<'_> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buffer_index >= self.buffer.len() && !self.refill_buffer() {
            return None;
        }

        let value = std::mem::take(&mut self.buffer[self.buffer_index]);
        self.buffer_index += 1;
        Some(value)
    }
}

#[cfg(feature = "execute")]
enum TdmsScalar {
    F64(f64),
    F32(f32),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    Bool(bool),
    Timestamp(String),
    String(String),
}

#[cfg(feature = "execute")]
enum TdmsColumnBuffer {
    F64(Vec<Option<f64>>),
    F32(Vec<Option<f32>>),
    I8(Vec<Option<i8>>),
    I16(Vec<Option<i16>>),
    I32(Vec<Option<i32>>),
    I64(Vec<Option<i64>>),
    U8(Vec<Option<u8>>),
    U16(Vec<Option<u16>>),
    U32(Vec<Option<u32>>),
    U64(Vec<Option<u64>>),
    Bool(Vec<Option<bool>>),
    Timestamp(Vec<Option<String>>),
    String(Vec<Option<String>>),
}

#[cfg(feature = "execute")]
impl TdmsColumnBuffer {
    fn from_tdms_data_type(data_type: &TdmsDataType, capacity: usize) -> Option<Self> {
        match data_type {
            TdmsDataType::Double => Some(Self::F64(Vec::with_capacity(capacity))),
            TdmsDataType::Float => Some(Self::F32(Vec::with_capacity(capacity))),
            TdmsDataType::I8 => Some(Self::I8(Vec::with_capacity(capacity))),
            TdmsDataType::I16 => Some(Self::I16(Vec::with_capacity(capacity))),
            TdmsDataType::I32 => Some(Self::I32(Vec::with_capacity(capacity))),
            TdmsDataType::I64 => Some(Self::I64(Vec::with_capacity(capacity))),
            TdmsDataType::U8 => Some(Self::U8(Vec::with_capacity(capacity))),
            TdmsDataType::U16 => Some(Self::U16(Vec::with_capacity(capacity))),
            TdmsDataType::U32 => Some(Self::U32(Vec::with_capacity(capacity))),
            TdmsDataType::U64 => Some(Self::U64(Vec::with_capacity(capacity))),
            TdmsDataType::Boolean => Some(Self::Bool(Vec::with_capacity(capacity))),
            TdmsDataType::TimeStamp => Some(Self::Timestamp(Vec::with_capacity(capacity))),
            TdmsDataType::String => Some(Self::String(Vec::with_capacity(capacity))),
        }
    }

    fn push(&mut self, value: TdmsScalar) {
        match (self, value) {
            (Self::F64(values), TdmsScalar::F64(value)) => values.push(Some(value)),
            (Self::F32(values), TdmsScalar::F32(value)) => values.push(Some(value)),
            (Self::I8(values), TdmsScalar::I8(value)) => values.push(Some(value)),
            (Self::I16(values), TdmsScalar::I16(value)) => values.push(Some(value)),
            (Self::I32(values), TdmsScalar::I32(value)) => values.push(Some(value)),
            (Self::I64(values), TdmsScalar::I64(value)) => values.push(Some(value)),
            (Self::U8(values), TdmsScalar::U8(value)) => values.push(Some(value)),
            (Self::U16(values), TdmsScalar::U16(value)) => values.push(Some(value)),
            (Self::U32(values), TdmsScalar::U32(value)) => values.push(Some(value)),
            (Self::U64(values), TdmsScalar::U64(value)) => values.push(Some(value)),
            (Self::Bool(values), TdmsScalar::Bool(value)) => values.push(Some(value)),
            (Self::Timestamp(values), TdmsScalar::Timestamp(value)) => values.push(Some(value)),
            (Self::String(values), TdmsScalar::String(value)) => values.push(Some(value)),
            _ => {
                debug_assert!(
                    false,
                    "TDMS scalar type did not match Arrow column buffer type"
                );
            }
        }
    }

    fn push_null(&mut self) {
        match self {
            Self::F64(values) => values.push(None),
            Self::F32(values) => values.push(None),
            Self::I8(values) => values.push(None),
            Self::I16(values) => values.push(None),
            Self::I32(values) => values.push(None),
            Self::I64(values) => values.push(None),
            Self::U8(values) => values.push(None),
            Self::U16(values) => values.push(None),
            Self::U32(values) => values.push(None),
            Self::U64(values) => values.push(None),
            Self::Bool(values) => values.push(None),
            Self::Timestamp(values) => values.push(None),
            Self::String(values) => values.push(None),
        }
    }

    fn pop_last(&mut self) {
        match self {
            Self::F64(values) => {
                values.pop();
            }
            Self::F32(values) => {
                values.pop();
            }
            Self::I8(values) => {
                values.pop();
            }
            Self::I16(values) => {
                values.pop();
            }
            Self::I32(values) => {
                values.pop();
            }
            Self::I64(values) => {
                values.pop();
            }
            Self::U8(values) => {
                values.pop();
            }
            Self::U16(values) => {
                values.pop();
            }
            Self::U32(values) => {
                values.pop();
            }
            Self::U64(values) => {
                values.pop();
            }
            Self::Bool(values) => {
                values.pop();
            }
            Self::Timestamp(values) => {
                values.pop();
            }
            Self::String(values) => {
                values.pop();
            }
        }
    }

    fn data_type(&self) -> DataType {
        match self {
            Self::F64(_) => DataType::Float64,
            Self::F32(_) => DataType::Float32,
            Self::I8(_) => DataType::Int8,
            Self::I16(_) => DataType::Int16,
            Self::I32(_) => DataType::Int32,
            Self::I64(_) => DataType::Int64,
            Self::U8(_) => DataType::UInt8,
            Self::U16(_) => DataType::UInt16,
            Self::U32(_) => DataType::UInt32,
            Self::U64(_) => DataType::UInt64,
            Self::Bool(_) => DataType::Boolean,
            Self::Timestamp(_) => DataType::Utf8,
            Self::String(_) => DataType::Utf8,
        }
    }

    fn take_array(&mut self) -> ArrayRef {
        match self {
            Self::F64(values) => Arc::new(Float64Array::from(std::mem::take(values))) as ArrayRef,
            Self::F32(values) => Arc::new(Float32Array::from(std::mem::take(values))) as ArrayRef,
            Self::I8(values) => Arc::new(Int8Array::from(std::mem::take(values))) as ArrayRef,
            Self::I16(values) => Arc::new(Int16Array::from(std::mem::take(values))) as ArrayRef,
            Self::I32(values) => Arc::new(Int32Array::from(std::mem::take(values))) as ArrayRef,
            Self::I64(values) => Arc::new(Int64Array::from(std::mem::take(values))) as ArrayRef,
            Self::U8(values) => Arc::new(UInt8Array::from(std::mem::take(values))) as ArrayRef,
            Self::U16(values) => Arc::new(UInt16Array::from(std::mem::take(values))) as ArrayRef,
            Self::U32(values) => Arc::new(UInt32Array::from(std::mem::take(values))) as ArrayRef,
            Self::U64(values) => Arc::new(UInt64Array::from(std::mem::take(values))) as ArrayRef,
            Self::Bool(values) => Arc::new(BooleanArray::from(std::mem::take(values))) as ArrayRef,
            Self::Timestamp(values) => {
                let mut builder = StringBuilder::new();
                for value in std::mem::take(values) {
                    if let Some(value) = value {
                        builder.append_value(value);
                    } else {
                        builder.append_null();
                    }
                }
                Arc::new(builder.finish()) as ArrayRef
            }
            Self::String(values) => {
                let mut builder = StringBuilder::new();
                for value in std::mem::take(values) {
                    if let Some(value) = value {
                        builder.append_value(value);
                    } else {
                        builder.append_null();
                    }
                }
                Arc::new(builder.finish()) as ArrayRef
            }
        }
    }
}

#[cfg(feature = "execute")]
struct TdmsChannelState<'a> {
    name: String,
    values: Box<dyn Iterator<Item = TdmsScalar> + Send + 'a>,
    buffer: TdmsColumnBuffer,
    exhausted: bool,
}

#[cfg(feature = "execute")]
impl TdmsChannelState<'_> {
    fn field(&self) -> Field {
        Field::new(self.name.clone(), self.buffer.data_type(), true)
    }
}

#[async_trait]
impl NodeLogic for BatchInsertTdmsLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "tdms_insert_local_db",
            "Batch Insert (TDMS)",
            "Reads a LabVIEW TDMS file and batch-inserts its channel data as rows into a vector database.",
            "Data/Database/Insert",
        );
        node.set_flowscript_name("db", "insertTdms");
        node.set_receiver("database");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "database",
            "Database",
            "Database Connection Reference",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "tdms_path",
            "TDMS File",
            "Path to the TDMS file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "chunk_size",
            "Chunk Size",
            "Chunk Size for buffered Arrow inserts",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10_000)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Finished inserting TDMS data",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let database = database.load(context).await?.db.clone();
        let chunk_size_input: u64 = context.evaluate_pin("chunk_size").await?;
        let chunk_size = chunk_size_input.clamp(1, TDMS_MAX_CHUNK_SIZE as u64) as usize;
        if chunk_size_input > TDMS_MAX_CHUNK_SIZE as u64 {
            context.log_message(
                &format!(
                    "TDMS chunk_size={} is above safe limit; clamped to {}",
                    chunk_size_input, TDMS_MAX_CHUNK_SIZE
                ),
                LogLevel::Warn,
            );
        }

        let tdms_path: FlowPath = context.evaluate_pin("tdms_path").await?;
        let source_file = resolve_tdms_source_file(&tdms_path, context).await?;
        let file = TdmsFile::open(source_file.path())
            .map_err(|e| flow_like_types::anyhow!("Failed to parse TDMS file: {:?}", e))?;

        let mut total_rows_inserted: u64 = 0;

        for group in file.groups() {
            let group_name = group.name().to_string();
            let channels: Vec<_> = group.channels().collect();
            let total_channels = channels.len();
            let mut channel_states: Vec<TdmsChannelState<'_>> = channels
                .into_iter()
                .filter_map(|channel| extract_channel_state(channel, chunk_size))
                .collect();

            if channel_states.len() < total_channels {
                context.log_message(
                    &format!(
                        "Skipping {} unsupported TDMS channel(s) in group '{}'",
                        total_channels - channel_states.len(),
                        group_name
                    ),
                    LogLevel::Warn,
                );
            }

            if channel_states.is_empty() {
                continue;
            }

            loop {
                if channel_states.iter().all(|state| state.exhausted) {
                    break;
                }

                let mut rows_in_chunk = 0usize;
                while rows_in_chunk < chunk_size {
                    if channel_states.iter().all(|state| state.exhausted) {
                        break;
                    }

                    let mut row_has_value = false;

                    for state in &mut channel_states {
                        if state.exhausted {
                            state.buffer.push_null();
                            continue;
                        }

                        if let Some(value) = state.values.next() {
                            state.buffer.push(value);
                            row_has_value = true;
                        } else {
                            state.exhausted = true;
                            state.buffer.push_null();
                        }
                    }

                    if !row_has_value {
                        for state in &mut channel_states {
                            state.buffer.pop_last();
                        }
                        break;
                    }

                    rows_in_chunk += 1;
                }

                if rows_in_chunk == 0 {
                    break;
                }

                let mut fields = Vec::with_capacity(channel_states.len() + 1);
                let mut columns = Vec::with_capacity(channel_states.len() + 1);

                fields.push(Field::new("_group", DataType::Utf8, false));
                let mut group_builder = StringBuilder::new();
                for _ in 0..rows_in_chunk {
                    group_builder.append_value(group_name.as_str());
                }
                columns.push(Arc::new(group_builder.finish()) as ArrayRef);

                for state in &mut channel_states {
                    fields.push(state.field());
                    columns.push(state.buffer.take_array());
                }

                let schema = Arc::new(Schema::new(fields));
                let batch = RecordBatch::try_new(schema, columns)?;
                total_rows_inserted += rows_in_chunk as u64;

                let mut database = database.write().await;
                if let Err(e) = database.inner_mut().insert_record_batch(batch).await {
                    context.log_message(
                        &format!("Error inserting TDMS Arrow chunk: {:?}", e),
                        LogLevel::Error,
                    );
                }

                context.log_message(
                    &format!(
                        "TDMS insert progress: {} rows inserted (group: {})",
                        total_rows_inserted, group_name
                    ),
                    LogLevel::Debug,
                );
            }
        }

        context.log_message(
            &format!("TDMS insert complete: {} total rows", total_rows_inserted),
            LogLevel::Info,
        );
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "TDMS processing requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn extract_channel_state<'a>(
    channel: TdmsChannel<'a>,
    chunk_size: usize,
) -> Option<TdmsChannelState<'a>> {
    let name = channel.name().to_string();
    let read_chunk_size = chunk_size.clamp(256, 16_384);
    let dtype = channel.dtype();
    let buffer = TdmsColumnBuffer::from_tdms_data_type(&dtype, chunk_size)?;

    let values: Box<dyn Iterator<Item = TdmsScalar> + Send + 'a> = match dtype {
        TdmsDataType::Double => {
            Box::new(TdmsValueIter::<f64>::new(channel, read_chunk_size).map(TdmsScalar::F64))
        }
        TdmsDataType::Float => {
            Box::new(TdmsValueIter::<f32>::new(channel, read_chunk_size).map(TdmsScalar::F32))
        }
        TdmsDataType::I8 => {
            Box::new(TdmsValueIter::<i8>::new(channel, read_chunk_size).map(TdmsScalar::I8))
        }
        TdmsDataType::I16 => {
            Box::new(TdmsValueIter::<i16>::new(channel, read_chunk_size).map(TdmsScalar::I16))
        }
        TdmsDataType::I32 => {
            Box::new(TdmsValueIter::<i32>::new(channel, read_chunk_size).map(TdmsScalar::I32))
        }
        TdmsDataType::I64 => {
            Box::new(TdmsValueIter::<i64>::new(channel, read_chunk_size).map(TdmsScalar::I64))
        }
        TdmsDataType::U8 => {
            Box::new(TdmsValueIter::<u8>::new(channel, read_chunk_size).map(TdmsScalar::U8))
        }
        TdmsDataType::U16 => {
            Box::new(TdmsValueIter::<u16>::new(channel, read_chunk_size).map(TdmsScalar::U16))
        }
        TdmsDataType::U32 => {
            Box::new(TdmsValueIter::<u32>::new(channel, read_chunk_size).map(TdmsScalar::U32))
        }
        TdmsDataType::U64 => {
            Box::new(TdmsValueIter::<u64>::new(channel, read_chunk_size).map(TdmsScalar::U64))
        }
        TdmsDataType::Boolean => {
            Box::new(TdmsValueIter::<bool>::new(channel, read_chunk_size).map(TdmsScalar::Bool))
        }
        TdmsDataType::TimeStamp => Box::new(
            TdmsValueIter::<TdmsRawTimestamp>::new(channel, read_chunk_size)
                .map(|value| TdmsScalar::Timestamp(format!("({}, {})", value.second, value.first))),
        ),
        TdmsDataType::String => {
            Box::new(TdmsStringIter::new(channel, read_chunk_size).map(TdmsScalar::String))
        }
    };

    Some(TdmsChannelState {
        name,
        values,
        buffer,
        exhausted: false,
    })
}
