#[cfg(feature = "execute")]
use flow_like::flow::execution::{LogLevel, internal_node::InternalNode};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
#[cfg(feature = "execute")]
use flow_like_storage::object_store::buffered::BufReader;
#[cfg(feature = "execute")]
use flow_like_types::json::to_value;
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use futures::StreamExt;

#[crate::register_node]
#[derive(Default)]
pub struct BufferedCsvReaderNode {}

impl BufferedCsvReaderNode {
    pub fn new() -> Self {
        BufferedCsvReaderNode {}
    }
}

#[async_trait]
impl NodeLogic for BufferedCsvReaderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "csv_buffered_reader",
            "Buffered CSV Reader",
            "Stream Read a CSV File",
            "Utils/CSV",
        );

        // node.add_icon("/flow/icons/bool.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("csv", "CSV", "CSV Path", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "chunk_size",
            "Chunk Size",
            "Chunk Size for Buffered Read",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10_000)));

        node.add_input_pin(
            "delimiter",
            "Delimiter",
            "Delimiter for CSV",
            VariableType::String,
        )
        .set_default_value(Some(json!(",")));

        node.add_output_pin(
            "for_chunk",
            "For Chunk",
            "Executes for each chunk",
            VariableType::Execution,
        );

        node.add_output_pin("chunk", "Chunk", "Chunk", VariableType::Struct)
            .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.add_output_pin("exec_done", "Done", "Done", VariableType::Execution);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_done").await?;
        context.activate_exec_pin("for_chunk").await?;
        let exec_item = context.get_pin_by_name("for_chunk").await?;
        let value = context.get_pin_by_name("chunk").await?;

        let delimiter: String = context.evaluate_pin("delimiter").await?;
        let delimiter = delimiter.as_bytes()[0];
        let csv_path: FlowPath = context.evaluate_pin("csv").await?;
        let store = csv_path.to_runtime(context).await?;
        let location = store.path.clone();
        let get_request = store.store.as_generic().get(&location).await?;
        let reader = BufReader::new(store.store.as_generic(), &get_request.meta);

        let mut rdr = csv_async::AsyncReaderBuilder::new()
            .has_headers(true)
            .buffer_capacity(32 * 1024 * 1024)
            .delimiter(delimiter)
            .create_reader(reader);

        let chunk_size: u64 = context.evaluate_pin("chunk_size").await?;
        let headers = rdr.byte_headers().await?.clone();
        let headers = headers
            .iter()
            .map(|h| {
                let lossy_header = String::from_utf8_lossy(h);
                lossy_header.to_string()
            })
            .collect::<Vec<String>>();

        let mut records = rdr.byte_records();
        let mut chunk = Vec::with_capacity(chunk_size as usize);
        let flow = exec_item.get_connected_nodes();
        let mut total_rows: u64 = 0;
        let mut chunk_count: u64 = 0;

        while let Some(element) = records.next().await {
            let record = match element {
                Ok(record) => record,
                Err(e) => {
                    context.log_message(
                        &format!("Error reading CSV record: {:?}", e),
                        LogLevel::Warn,
                    );
                    continue;
                }
            };
            let json_obj =
                headers
                    .iter()
                    .zip(record.iter())
                    .fold(json!({}), |mut acc, (header, value)| {
                        let lossy_value = String::from_utf8_lossy(value);
                        acc[header] = json!(lossy_value.to_string());
                        acc
                    });
            chunk.push(json_obj);
            total_rows += 1;
            if chunk.len() as u64 == chunk_size {
                chunk_count += 1;
                let chunk_value = to_value(&chunk)?;
                context.override_pin_value_if_active(&value.id, &chunk_value);
                value.set_value(chunk_value).await;
                chunk = Vec::with_capacity(chunk_size as usize);
                for node in &flow {
                    let mut sub_context = context.create_sub_context(node).await;
                    let run = InternalNode::trigger(&mut sub_context, &mut None, true).await;
                    sub_context.end_trace();
                    context.push_sub_context(&mut sub_context);

                    if run.is_err() {
                        let error = run.err().unwrap();
                        context.log_message(&format!("Error: {:?}", error), LogLevel::Error);
                    }
                }
            }
        }

        if !chunk.is_empty() {
            chunk_count += 1;
            let chunk_value = to_value(&chunk)?;
            context.override_pin_value_if_active(&value.id, &chunk_value);
            value.set_value(chunk_value).await;
            for node in &flow {
                let mut sub_context = context.create_sub_context(node).await;
                let run = InternalNode::trigger(&mut sub_context, &mut None, true).await;
                sub_context.end_trace();
                context.push_sub_context(&mut sub_context);

                if run.is_err() {
                    let error = run.err().unwrap();
                    context.log_message(&format!("Error: {:?}", error), LogLevel::Error);
                }
            }
        }

        context.log_message(
            &format!(
                "CSV Reader completed: {} total rows, {} chunks, {} connected nodes",
                total_rows,
                chunk_count,
                flow.len()
            ),
            LogLevel::Info,
        );

        context.activate_exec_pin("exec_done").await?;
        context.deactivate_exec_pin("for_chunk").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This feature requires the 'execute' feature"
        ))
    }
}

// async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
//     context.deactivate_exec_pin("exec_done").await?;
//     context.activate_exec_pin("for_chunk").await?;

//     let csv_path: FlowPath = context.evaluate_pin("csv").await?;
//     let store = csv_path.to_runtime(context).await?;
//     let location = store.path.clone();
//     let csv_bytes = store
//     .store
//     .as_generic()
//     .get(&location)
//     .await?
//     .bytes().await?;

//     let cursor = std::io::Cursor::new(csv_bytes);
//     let mut rdr = csv::ReaderBuilder::new()
//         .has_headers(true)
//         .from_reader(cursor);

//     let chunk_size: u64 = context.evaluate_pin("chunk_size").await?;;
//     let headers = rdr.headers().iter().map(|h| {
//         let bytes = h.as_byte_record();
//         let lossy_header = String::from_utf8_lossy(bytes.as_slice());
//         lossy_header.to_string()
//     }).collect::<Vec<String>>();

//     let records = rdr.byte_records();
//     let mut chunk = Vec::with_capacity(chunk_size as usize);
//     for element in records{
//         let record = match element {
//             Ok(record) => record,
//             Err(e) => {
//                 continue;
//             }
//         };
//         let json_obj= headers.iter().zip(record.iter()).fold(json!({}), |mut acc, (header, value)| {
//             let lossy_value = String::from_utf8_lossy(value);
//             acc[header] = json!(lossy_value.to_string());
//             acc
//         });
//         chunk.push(json_obj);
//         if chunk.len() as u64 == chunk_size {
//             println!("Chunk: {:?}", chunk.len());
//             chunk = Vec::with_capacity(chunk_size as usize);
//         }
//     }

//     return Ok(());
// }
#[cfg(all(test, feature = "execute"))]
mod tests {
    use flow_like_types::json::json;
    use futures::StreamExt;
    use std::io::Cursor;

    /// Test that CSV parsing with comma delimiter works correctly
    #[tokio::test]
    async fn test_csv_parsing_with_comma_delimiter() {
        let csv_data = b"name,age,city\nAlice,30,Berlin\nBob,25,Munich";
        let cursor = Cursor::new(csv_data.as_slice());

        let mut rdr = csv_async::AsyncReaderBuilder::new()
            .has_headers(true)
            .delimiter(b',')
            .create_reader(cursor);

        let headers = rdr.byte_headers().await.unwrap().clone();
        let headers: Vec<String> = headers
            .iter()
            .map(|h| String::from_utf8_lossy(h).to_string())
            .collect();

        assert_eq!(headers, vec!["name", "age", "city"]);

        let mut records = rdr.byte_records();
        let mut results = Vec::new();

        while let Some(element) = records.next().await {
            let record = element.unwrap();
            let json_obj =
                headers
                    .iter()
                    .zip(record.iter())
                    .fold(json!({}), |mut acc, (header, value)| {
                        let lossy_value = String::from_utf8_lossy(value);
                        acc[header] = json!(lossy_value.to_string());
                        acc
                    });
            results.push(json_obj);
        }

        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["name"], "Alice");
        assert_eq!(results[0]["age"], "30");
        assert_eq!(results[0]["city"], "Berlin");
        assert_eq!(results[1]["name"], "Bob");
        assert_eq!(results[1]["age"], "25");
        assert_eq!(results[1]["city"], "Munich");
    }

    /// Test that CSV parsing with semicolon delimiter works correctly
    #[tokio::test]
    async fn test_csv_parsing_with_semicolon_delimiter() {
        let csv_data = b"name;age;city\nAlice;30;Berlin\nBob;25;Munich";
        let cursor = Cursor::new(csv_data.as_slice());

        let mut rdr = csv_async::AsyncReaderBuilder::new()
            .has_headers(true)
            .delimiter(b';')
            .create_reader(cursor);

        let headers = rdr.byte_headers().await.unwrap().clone();
        let headers: Vec<String> = headers
            .iter()
            .map(|h| String::from_utf8_lossy(h).to_string())
            .collect();

        assert_eq!(headers, vec!["name", "age", "city"]);

        let mut records = rdr.byte_records();
        let record = records.next().await.unwrap().unwrap();
        let name = String::from_utf8_lossy(record.get(0).unwrap());
        assert_eq!(name, "Alice");
    }

    /// Test chunking logic
    #[tokio::test]
    async fn test_chunking_logic() {
        let csv_data = b"id,value\n1,a\n2,b\n3,c\n4,d\n5,e";
        let cursor = Cursor::new(csv_data.as_slice());

        let mut rdr = csv_async::AsyncReaderBuilder::new()
            .has_headers(true)
            .delimiter(b',')
            .create_reader(cursor);

        let headers = rdr.byte_headers().await.unwrap().clone();
        let headers: Vec<String> = headers
            .iter()
            .map(|h| String::from_utf8_lossy(h).to_string())
            .collect();

        let chunk_size: u64 = 2;
        let mut records = rdr.byte_records();
        let mut chunk = Vec::with_capacity(chunk_size as usize);
        let mut chunks_processed = 0;

        while let Some(element) = records.next().await {
            let record = element.unwrap();
            let json_obj =
                headers
                    .iter()
                    .zip(record.iter())
                    .fold(json!({}), |mut acc, (header, value)| {
                        let lossy_value = String::from_utf8_lossy(value);
                        acc[header] = json!(lossy_value.to_string());
                        acc
                    });
            chunk.push(json_obj);
            if chunk.len() as u64 == chunk_size {
                chunks_processed += 1;
                chunk = Vec::with_capacity(chunk_size as usize);
            }
        }

        // Should have processed 2 full chunks (4 records) and have 1 remaining
        assert_eq!(chunks_processed, 2);
        assert_eq!(chunk.len(), 1);
    }

    /// Test UTF-8 handling with special characters
    #[tokio::test]
    async fn test_utf8_handling() {
        let csv_data = "name,city\nHäns,München\nJosé,São Paulo".as_bytes();
        let cursor = Cursor::new(csv_data);

        let mut rdr = csv_async::AsyncReaderBuilder::new()
            .has_headers(true)
            .delimiter(b',')
            .create_reader(cursor);

        let headers = rdr.byte_headers().await.unwrap().clone();
        let headers: Vec<String> = headers
            .iter()
            .map(|h| String::from_utf8_lossy(h).to_string())
            .collect();

        let mut records = rdr.byte_records();
        let mut results = Vec::new();

        while let Some(element) = records.next().await {
            let record = element.unwrap();
            let json_obj =
                headers
                    .iter()
                    .zip(record.iter())
                    .fold(json!({}), |mut acc, (header, value)| {
                        let lossy_value = String::from_utf8_lossy(value);
                        acc[header] = json!(lossy_value.to_string());
                        acc
                    });
            results.push(json_obj);
        }

        assert_eq!(results[0]["name"], "Häns");
        assert_eq!(results[0]["city"], "München");
        assert_eq!(results[1]["name"], "José");
        assert_eq!(results[1]["city"], "São Paulo");
    }

    /// Test empty CSV (only headers)
    #[tokio::test]
    async fn test_empty_csv() {
        let csv_data = b"name,age,city";
        let cursor = Cursor::new(csv_data.as_slice());

        let mut rdr = csv_async::AsyncReaderBuilder::new()
            .has_headers(true)
            .delimiter(b',')
            .create_reader(cursor);

        let headers = rdr.byte_headers().await.unwrap().clone();
        let headers: Vec<String> = headers
            .iter()
            .map(|h| String::from_utf8_lossy(h).to_string())
            .collect();

        assert_eq!(headers, vec!["name", "age", "city"]);

        let mut records = rdr.byte_records();
        let record = records.next().await;
        assert!(record.is_none());
    }

    /// Test CSV with quoted fields containing delimiters
    #[tokio::test]
    async fn test_quoted_fields_with_delimiter() {
        let csv_data = b"name,address\n\"Smith, John\",\"123 Main St, Apt 4\"";
        let cursor = Cursor::new(csv_data.as_slice());

        let mut rdr = csv_async::AsyncReaderBuilder::new()
            .has_headers(true)
            .delimiter(b',')
            .create_reader(cursor);

        let headers = rdr.byte_headers().await.unwrap().clone();
        let _headers: Vec<String> = headers
            .iter()
            .map(|h| String::from_utf8_lossy(h).to_string())
            .collect();

        let mut records = rdr.byte_records();
        let record = records.next().await.unwrap().unwrap();

        let name = String::from_utf8_lossy(record.get(0).unwrap());
        let address = String::from_utf8_lossy(record.get(1).unwrap());

        assert_eq!(name, "Smith, John");
        assert_eq!(address, "123 Main St, Apt 4");
    }
}
