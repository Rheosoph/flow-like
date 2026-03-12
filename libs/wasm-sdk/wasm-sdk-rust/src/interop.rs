use serde::{Deserialize, Serialize};

use crate::Context;

// =============================================================================
// FlowPath — handle to a file in an object store, resolved host-side
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowPath {
    pub path: String,
    pub store_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_store_ref: Option<String>,
}

impl FlowPath {
    pub fn new(path: String, store_ref: String, cache_store_ref: Option<String>) -> Self {
        Self {
            path,
            store_ref,
            cache_store_ref,
        }
    }

    pub fn read(&self, ctx: &Context) -> Option<Vec<u8>> {
        ctx.storage_read_typed(self)
    }

    pub fn write(&self, ctx: &Context, data: &[u8]) -> bool {
        ctx.storage_write_typed(self, data)
    }

    pub fn list(&self, ctx: &Context) -> Option<Vec<FlowPath>> {
        ctx.storage_list_typed(self)
    }

    pub fn schema() -> String {
        crate::host::get_type_schema("FlowPath").unwrap_or_default()
    }
}

// =============================================================================
// NodeImage — handle to an in-memory image, resolved host-side
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeImage {
    pub image_ref: String,
}

impl NodeImage {
    pub fn from_bytes(ctx: &Context, data: &[u8], format: &str) -> Option<Self> {
        ctx.image_from_bytes(data, format)
    }

    pub fn to_bytes(&self, ctx: &Context, format: &str) -> Option<Vec<u8>> {
        ctx.image_to_bytes(self, format)
    }

    pub fn schema() -> String {
        crate::host::get_type_schema("NodeImage").unwrap_or_default()
    }
}

// =============================================================================
// Bit — model descriptor for LLM / VLM interactions
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bit {
    pub id: String,
    #[serde(rename = "type", default)]
    pub bit_type: String,
    #[serde(default)]
    pub hub: String,
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub parameters: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Bit {
    /// Send a completion prompt to the LLM/VLM referenced by this Bit.
    /// Returns the model's response text.
    pub fn prompt(&self, ctx: &Context, messages: &[ChatMessage]) -> Option<String> {
        ctx.llm_prompt(self, messages)
    }

    /// Stream a completion prompt — text chunks are streamed via the streaming interface
    /// and the full response is returned.
    pub fn prompt_stream(&self, ctx: &Context, messages: &[ChatMessage]) -> Option<String> {
        ctx.llm_prompt_stream(self, messages)
    }

    pub fn schema() -> String {
        crate::host::get_type_schema("Bit").unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(flatten)]
    pub content: ChatContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallData>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChatContent {
    Text { content: String },
    Parts { parts: Vec<ContentPart> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    Image { image: ImageData },
    Audio { audio: AudioData },
    Video { video: VideoData },
    Document { document: DocumentData },
    ToolCall { tool_call: ToolCallData },
    ToolResult { tool_result: ToolResultData },
    Reasoning { reasoning: ReasoningData },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageData {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioData {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoData {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentData {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallData {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultData {
    pub id: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub text: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: ChatContent::Text {
                content: content.into(),
            },
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: ChatContent::Text {
                content: content.into(),
            },
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user_multimodal(parts: Vec<ContentPart>) -> Self {
        Self {
            role: "user".into(),
            content: ChatContent::Parts { parts },
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: ChatContent::Text {
                content: content.into(),
            },
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant_with_tool_calls(
        content: impl Into<String>,
        tool_calls: Vec<ToolCallData>,
    ) -> Self {
        Self {
            role: "assistant".into(),
            content: ChatContent::Text {
                content: content.into(),
            },
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: ChatContent::Text {
                content: content.into(),
            },
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }

    pub fn text_content(&self) -> String {
        match &self.content {
            ChatContent::Text { content } => content.clone(),
            ChatContent::Parts { parts } => parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

impl ContentPart {
    pub fn text(text: impl Into<String>) -> Self {
        ContentPart::Text { text: text.into() }
    }

    pub fn image_url(url: impl Into<String>) -> Self {
        ContentPart::Image {
            image: ImageData {
                url: url.into(),
                media_type: None,
                detail: None,
            },
        }
    }

    pub fn image(url: impl Into<String>, media_type: impl Into<String>) -> Self {
        ContentPart::Image {
            image: ImageData {
                url: url.into(),
                media_type: Some(media_type.into()),
                detail: None,
            },
        }
    }

    pub fn audio_url(url: impl Into<String>) -> Self {
        ContentPart::Audio {
            audio: AudioData {
                url: url.into(),
                media_type: None,
            },
        }
    }

    pub fn audio(url: impl Into<String>, media_type: impl Into<String>) -> Self {
        ContentPart::Audio {
            audio: AudioData {
                url: url.into(),
                media_type: Some(media_type.into()),
            },
        }
    }

    pub fn video_url(url: impl Into<String>) -> Self {
        ContentPart::Video {
            video: VideoData {
                url: url.into(),
                media_type: None,
            },
        }
    }

    pub fn video(url: impl Into<String>, media_type: impl Into<String>) -> Self {
        ContentPart::Video {
            video: VideoData {
                url: url.into(),
                media_type: Some(media_type.into()),
            },
        }
    }

    pub fn document_url(url: impl Into<String>) -> Self {
        ContentPart::Document {
            document: DocumentData {
                url: url.into(),
                media_type: None,
            },
        }
    }

    pub fn document(url: impl Into<String>, media_type: impl Into<String>) -> Self {
        ContentPart::Document {
            document: DocumentData {
                url: url.into(),
                media_type: Some(media_type.into()),
            },
        }
    }

    pub fn tool_call(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        ContentPart::ToolCall {
            tool_call: ToolCallData {
                id: id.into(),
                name: name.into(),
                arguments,
            },
        }
    }

    pub fn tool_result(id: impl Into<String>, content: impl Into<String>) -> Self {
        ContentPart::ToolResult {
            tool_result: ToolResultData {
                id: id.into(),
                content: content.into(),
            },
        }
    }

    pub fn reasoning(text: Vec<String>) -> Self {
        ContentPart::Reasoning {
            reasoning: ReasoningData {
                id: None,
                text,
                signature: None,
            },
        }
    }
}

// =============================================================================
// CachedEmbeddingModel — handle to a text/image embedding model
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedEmbeddingModel {
    pub cache_key: String,
    pub model_type: String,
}

impl CachedEmbeddingModel {
    /// Embed texts for query (optimised for retrieval queries).
    pub fn embed_query(&self, ctx: &Context, texts: &[String]) -> Option<Vec<Vec<f32>>> {
        ctx.embed_text_query(self, texts)
    }

    /// Embed texts for document indexing.
    pub fn embed_document(&self, ctx: &Context, texts: &[String]) -> Option<Vec<Vec<f32>>> {
        ctx.embed_text_document(self, texts)
    }

    /// Embed an image.
    pub fn embed_image(&self, ctx: &Context, image: &NodeImage) -> Option<Vec<f32>> {
        ctx.embed_image(self, image)
    }

    pub fn schema() -> String {
        crate::host::get_type_schema("CachedEmbeddingModel").unwrap_or_default()
    }
}

// =============================================================================
// NodeDBConnection — handle to a vector database
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeDBConnection {
    pub cache_key: String,
}

impl NodeDBConnection {
    pub fn vector_search(
        &self,
        ctx: &Context,
        query: &VectorSearchQuery,
    ) -> Option<Vec<serde_json::Value>> {
        ctx.db_vector_search(self, query)
    }

    pub fn fts_search(
        &self,
        ctx: &Context,
        query: &FtsSearchQuery,
    ) -> Option<Vec<serde_json::Value>> {
        ctx.db_fts_search(self, query)
    }

    pub fn hybrid_search(
        &self,
        ctx: &Context,
        query: &HybridSearchQuery,
    ) -> Option<Vec<serde_json::Value>> {
        ctx.db_hybrid_search(self, query)
    }

    pub fn insert(&self, ctx: &Context, items: &[serde_json::Value]) -> bool {
        ctx.db_insert(self, items)
    }

    pub fn upsert(&self, ctx: &Context, items: &[serde_json::Value], id_field: &str) -> bool {
        ctx.db_upsert(self, items, id_field)
    }

    pub fn delete(&self, ctx: &Context, filter: &str) -> bool {
        ctx.db_delete(self, filter)
    }

    pub fn list(
        &self,
        ctx: &Context,
        select: Option<&[String]>,
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> Option<Vec<serde_json::Value>> {
        ctx.db_list(self, select, limit, offset)
    }

    pub fn count(&self, ctx: &Context, filter: Option<&str>) -> Option<u64> {
        ctx.db_count(self, filter)
    }

    pub fn schema() -> String {
        crate::host::get_type_schema("NodeDBConnection").unwrap_or_default()
    }
}

// =============================================================================
// Query types for vector DB operations
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorSearchQuery {
    pub vector: Vec<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Vec<String>>,
    #[serde(default)]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FtsSearchQuery {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
    #[serde(default)]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSearchQuery {
    pub vector: Vec<f32>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
    #[serde(default)]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub rerank: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // =========================================================================
    // FlowPath
    // =========================================================================

    #[test]
    fn test_flow_path_new() {
        let fp = FlowPath::new("a/b.txt".into(), "store1".into(), None);
        assert_eq!(fp.path, "a/b.txt");
        assert_eq!(fp.store_ref, "store1");
        assert!(fp.cache_store_ref.is_none());
    }

    #[test]
    fn test_flow_path_new_with_cache() {
        let fp = FlowPath::new("a/b.txt".into(), "store1".into(), Some("cache1".into()));
        assert_eq!(fp.cache_store_ref, Some("cache1".to_string()));
    }

    #[test]
    fn test_flow_path_serde_roundtrip() {
        let fp = FlowPath::new("dir/file.bin".into(), "s3".into(), Some("redis".into()));
        let json = serde_json::to_string(&fp).unwrap();
        let fp2: FlowPath = serde_json::from_str(&json).unwrap();
        assert_eq!(fp.path, fp2.path);
        assert_eq!(fp.store_ref, fp2.store_ref);
        assert_eq!(fp.cache_store_ref, fp2.cache_store_ref);
    }

    #[test]
    fn test_flow_path_omits_none_cache() {
        let fp = FlowPath::new("x".into(), "y".into(), None);
        let val: serde_json::Value = serde_json::to_value(&fp).unwrap();
        assert!(val.get("cache_store_ref").is_none());
    }

    #[test]
    fn test_flow_path_deserialize_without_cache() {
        let val = json!({"path": "a", "store_ref": "b"});
        let fp: FlowPath = serde_json::from_value(val).unwrap();
        assert_eq!(fp.path, "a");
        assert!(fp.cache_store_ref.is_none());
    }

    // =========================================================================
    // NodeImage
    // =========================================================================

    #[test]
    fn test_node_image_serde_roundtrip() {
        let img = NodeImage {
            image_ref: "ref-abc-123".into(),
        };
        let json = serde_json::to_string(&img).unwrap();
        let img2: NodeImage = serde_json::from_str(&json).unwrap();
        assert_eq!(img.image_ref, img2.image_ref);
    }

    // =========================================================================
    // Bit
    // =========================================================================

    #[test]
    fn test_bit_serde_roundtrip() {
        let bit = Bit {
            id: "model-1".into(),
            bit_type: "llm".into(),
            hub: "huggingface".into(),
            hash: "abc123".into(),
            parameters: json!({"temperature": 0.7}),
            file_name: Some("model.bin".into()),
            version: Some("1.0".into()),
            license: None,
            extra: serde_json::Map::new(),
        };
        let json_str = serde_json::to_string(&bit).unwrap();
        let bit2: Bit = serde_json::from_str(&json_str).unwrap();
        assert_eq!(bit.id, bit2.id);
        assert_eq!(bit.bit_type, bit2.bit_type);
        assert_eq!(bit.hub, bit2.hub);
        assert_eq!(bit.parameters, bit2.parameters);
        assert_eq!(bit.file_name, bit2.file_name);
        assert_eq!(bit.version, bit2.version);
    }

    #[test]
    fn test_bit_type_rename() {
        let bit = Bit {
            id: "x".into(),
            bit_type: "vlm".into(),
            hub: String::new(),
            hash: String::new(),
            parameters: json!(null),
            file_name: None,
            version: None,
            license: None,
            extra: serde_json::Map::new(),
        };
        let val: serde_json::Value = serde_json::to_value(&bit).unwrap();
        assert_eq!(val.get("type").unwrap(), "vlm");
        assert!(val.get("bit_type").is_none());
    }

    #[test]
    fn test_bit_omits_none_fields() {
        let bit = Bit {
            id: "x".into(),
            bit_type: String::new(),
            hub: String::new(),
            hash: String::new(),
            parameters: json!(null),
            file_name: None,
            version: None,
            license: None,
            extra: serde_json::Map::new(),
        };
        let val: serde_json::Value = serde_json::to_value(&bit).unwrap();
        assert!(val.get("file_name").is_none());
        assert!(val.get("version").is_none());
        assert!(val.get("license").is_none());
    }

    #[test]
    fn test_bit_flatten_extra_fields() {
        let mut extra = serde_json::Map::new();
        extra.insert("custom_key".into(), json!("custom_value"));
        let bit = Bit {
            id: "x".into(),
            bit_type: String::new(),
            hub: String::new(),
            hash: String::new(),
            parameters: json!(null),
            file_name: None,
            version: None,
            license: None,
            extra,
        };
        let val: serde_json::Value = serde_json::to_value(&bit).unwrap();
        assert_eq!(val.get("custom_key").unwrap(), "custom_value");

        let bit2: Bit = serde_json::from_value(val).unwrap();
        assert_eq!(
            bit2.extra.get("custom_key").unwrap(),
            &json!("custom_value")
        );
    }

    #[test]
    fn test_bit_deserialize_minimal() {
        let val = json!({"id": "test"});
        let bit: Bit = serde_json::from_value(val).unwrap();
        assert_eq!(bit.id, "test");
        assert_eq!(bit.bit_type, "");
        assert_eq!(bit.hub, "");
    }

    // =========================================================================
    // ChatMessage
    // =========================================================================

    #[test]
    fn test_chat_message_system() {
        let msg = ChatMessage::system("You are helpful.");
        assert_eq!(msg.role, "system");
        assert_eq!(msg.text_content(), "You are helpful.");
    }

    #[test]
    fn test_chat_message_user() {
        let msg = ChatMessage::user("Hello!");
        assert_eq!(msg.role, "user");
        assert_eq!(msg.text_content(), "Hello!");
    }

    #[test]
    fn test_chat_message_assistant() {
        let msg = ChatMessage::assistant("Hi there.");
        assert_eq!(msg.role, "assistant");
        assert_eq!(msg.text_content(), "Hi there.");
    }

    #[test]
    fn test_chat_message_serde_roundtrip() {
        let msg = ChatMessage::user("test");
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg.role, msg2.role);
        assert_eq!(msg.text_content(), msg2.text_content());
    }

    #[test]
    fn test_chat_message_array_serialization() {
        let messages = vec![
            ChatMessage::system("sys"),
            ChatMessage::user("usr"),
            ChatMessage::assistant("ast"),
        ];
        let json = serde_json::to_string(&messages).unwrap();
        let messages2: Vec<ChatMessage> = serde_json::from_str(&json).unwrap();
        assert_eq!(messages2.len(), 3);
        assert_eq!(messages2[0].role, "system");
        assert_eq!(messages2[1].role, "user");
        assert_eq!(messages2[2].role, "assistant");
    }

    // =========================================================================
    // CachedEmbeddingModel
    // =========================================================================

    #[test]
    fn test_cached_embedding_model_serde_roundtrip() {
        let model = CachedEmbeddingModel {
            cache_key: "embed-model-abc".into(),
            model_type: "text".into(),
        };
        let json = serde_json::to_string(&model).unwrap();
        let model2: CachedEmbeddingModel = serde_json::from_str(&json).unwrap();
        assert_eq!(model.cache_key, model2.cache_key);
        assert_eq!(model.model_type, model2.model_type);
    }

    // =========================================================================
    // NodeDBConnection
    // =========================================================================

    #[test]
    fn test_node_db_connection_default() {
        let conn = NodeDBConnection::default();
        assert_eq!(conn.cache_key, "");
    }

    #[test]
    fn test_node_db_connection_serde_roundtrip() {
        let conn = NodeDBConnection {
            cache_key: "db-conn-xyz".into(),
        };
        let json = serde_json::to_string(&conn).unwrap();
        let conn2: NodeDBConnection = serde_json::from_str(&json).unwrap();
        assert_eq!(conn.cache_key, conn2.cache_key);
    }

    // =========================================================================
    // VectorSearchQuery
    // =========================================================================

    #[test]
    fn test_vector_search_query_serde_roundtrip() {
        let q = VectorSearchQuery {
            vector: vec![0.1, 0.2, 0.3],
            filter: Some("category = 'doc'".into()),
            select: Some(vec!["id".into(), "text".into()]),
            limit: 10,
            offset: 0,
        };
        let json = serde_json::to_string(&q).unwrap();
        let q2: VectorSearchQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(q.vector, q2.vector);
        assert_eq!(q.filter, q2.filter);
        assert_eq!(q.select, q2.select);
        assert_eq!(q.limit, q2.limit);
    }

    #[test]
    fn test_vector_search_query_omits_none() {
        let q = VectorSearchQuery {
            vector: vec![1.0],
            filter: None,
            select: None,
            limit: 5,
            offset: 0,
        };
        let val: serde_json::Value = serde_json::to_value(&q).unwrap();
        assert!(val.get("filter").is_none());
        assert!(val.get("select").is_none());
    }

    // =========================================================================
    // FtsSearchQuery
    // =========================================================================

    #[test]
    fn test_fts_search_query_serde_roundtrip() {
        let q = FtsSearchQuery {
            text: "search term".into(),
            filter: None,
            select: None,
            fields: Some(vec!["title".into(), "body".into()]),
            limit: 20,
            offset: 5,
        };
        let json = serde_json::to_string(&q).unwrap();
        let q2: FtsSearchQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(q.text, q2.text);
        assert_eq!(q.fields, q2.fields);
        assert_eq!(q.limit, q2.limit);
        assert_eq!(q.offset, q2.offset);
    }

    #[test]
    fn test_fts_search_query_omits_none() {
        let q = FtsSearchQuery {
            text: "x".into(),
            filter: None,
            select: None,
            fields: None,
            limit: 0,
            offset: 0,
        };
        let val: serde_json::Value = serde_json::to_value(&q).unwrap();
        assert!(val.get("filter").is_none());
        assert!(val.get("select").is_none());
        assert!(val.get("fields").is_none());
    }

    // =========================================================================
    // HybridSearchQuery
    // =========================================================================

    #[test]
    fn test_hybrid_search_query_serde_roundtrip() {
        let q = HybridSearchQuery {
            vector: vec![0.5, 0.6],
            text: "hybrid".into(),
            filter: Some("active = true".into()),
            select: None,
            fields: None,
            limit: 10,
            offset: 0,
            rerank: true,
        };
        let json = serde_json::to_string(&q).unwrap();
        let q2: HybridSearchQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(q.vector, q2.vector);
        assert_eq!(q.text, q2.text);
        assert_eq!(q.filter, q2.filter);
        assert!(q2.rerank);
    }

    #[test]
    fn test_hybrid_search_query_rerank_defaults_false() {
        let val = json!({"vector": [1.0], "text": "x", "limit": 1, "offset": 0});
        let q: HybridSearchQuery = serde_json::from_value(val).unwrap();
        assert!(!q.rerank);
    }

    // =========================================================================
    // Context payload construction (verify JSON shapes for host calls)
    // =========================================================================

    #[test]
    fn test_db_upsert_payload_shape() {
        let items = vec![json!({"id": 1, "text": "hello"})];
        let payload = json!({ "items": items, "id_field": "id" });
        let s = serde_json::to_string(&payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["id_field"], "id");
        assert!(parsed["items"].is_array());
    }

    #[test]
    fn test_db_delete_payload_shape() {
        let payload = json!({ "filter": "id > 5" });
        let s = serde_json::to_string(&payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["filter"], "id > 5");
    }

    #[test]
    fn test_db_list_payload_shape() {
        let select: Option<&[String]> = Some(&["id".to_string(), "name".to_string()]);
        let limit: Option<u64> = Some(50);
        let offset: Option<u64> = Some(10);
        let payload = json!({
            "select": select,
            "limit": limit,
            "offset": offset,
        });
        let s = serde_json::to_string(&payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["limit"], 50);
        assert_eq!(parsed["offset"], 10);
        assert!(parsed["select"].is_array());
    }

    #[test]
    fn test_db_list_payload_null_optionals() {
        let select: Option<&[String]> = None;
        let limit: Option<u64> = None;
        let offset: Option<u64> = None;
        let payload = json!({
            "select": select,
            "limit": limit,
            "offset": offset,
        });
        assert!(payload["select"].is_null());
        assert!(payload["limit"].is_null());
    }

    #[test]
    fn test_db_count_payload_shape() {
        let filter: Option<&str> = Some("status = 'active'");
        let payload = json!({ "filter": filter });
        let s = serde_json::to_string(&payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["filter"], "status = 'active'");
    }

    #[test]
    fn test_db_count_payload_null_filter() {
        let filter: Option<&str> = None;
        let payload = json!({ "filter": filter });
        assert!(payload["filter"].is_null());
    }

    // =========================================================================
    // Multimodal ChatMessage
    // =========================================================================

    #[test]
    fn test_chat_message_multimodal_text_content() {
        let msg = ChatMessage::user_multimodal(vec![
            ContentPart::text("Describe this image"),
            ContentPart::image_url("https://example.com/photo.png"),
        ]);
        assert_eq!(msg.role, "user");
        assert_eq!(msg.text_content(), "Describe this image");
    }

    #[test]
    fn test_chat_message_multimodal_serde_roundtrip() {
        let msg = ChatMessage::user_multimodal(vec![
            ContentPart::text("hello"),
            ContentPart::image("data:image/png;base64,abc", "image/png"),
            ContentPart::audio_url("https://example.com/audio.mp3"),
            ContentPart::video_url("https://example.com/video.mp4"),
            ContentPart::document_url("https://example.com/doc.pdf"),
        ]);
        let json_str = serde_json::to_string(&msg).unwrap();
        let msg2: ChatMessage = serde_json::from_str(&json_str).unwrap();
        assert_eq!(msg2.role, "user");
        assert_eq!(msg2.text_content(), "hello");
        if let ChatContent::Parts { parts } = &msg2.content {
            assert_eq!(parts.len(), 5);
        } else {
            panic!("Expected Parts variant");
        }
    }

    #[test]
    fn test_content_part_constructors() {
        let text = ContentPart::text("hi");
        assert!(matches!(text, ContentPart::Text { .. }));

        let img = ContentPart::image_url("https://example.com/img.jpg");
        assert!(matches!(img, ContentPart::Image { .. }));

        let aud = ContentPart::audio("https://example.com/a.wav", "audio/wav");
        assert!(matches!(aud, ContentPart::Audio { .. }));

        let vid = ContentPart::video("https://example.com/v.mp4", "video/mp4");
        assert!(matches!(vid, ContentPart::Video { .. }));

        let doc = ContentPart::document("https://example.com/d.pdf", "application/pdf");
        assert!(matches!(doc, ContentPart::Document { .. }));
    }
}
