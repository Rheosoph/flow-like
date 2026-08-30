use super::provider::{GOOGLE_PROVIDER_ID, GoogleProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

// =============================================================================
// YouTube Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct YouTubeVideo {
    pub video_id: String,
    pub title: String,
    pub description: Option<String>,
    pub channel_id: String,
    pub channel_title: String,
    pub published_at: Option<String>,
    pub thumbnail_url: Option<String>,
    pub view_count: Option<i64>,
    pub like_count: Option<i64>,
    pub duration: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct YouTubeChannel {
    pub channel_id: String,
    pub title: String,
    pub description: Option<String>,
    pub custom_url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub subscriber_count: Option<i64>,
    pub video_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct YouTubePlaylist {
    pub playlist_id: String,
    pub title: String,
    pub description: Option<String>,
    pub channel_id: String,
    pub item_count: Option<i64>,
    pub thumbnail_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct YouTubePlaylistItem {
    pub item_id: String,
    pub video_id: String,
    pub title: String,
    pub description: Option<String>,
    pub position: i64,
    pub thumbnail_url: Option<String>,
}

fn parse_video_from_search(item: &Value) -> Option<YouTubeVideo> {
    let snippet = &item["snippet"];
    let video_id = item["id"]["videoId"]
        .as_str()
        .or_else(|| item["id"].as_str())?;

    Some(YouTubeVideo {
        video_id: video_id.to_string(),
        title: snippet["title"].as_str().unwrap_or("").to_string(),
        description: snippet["description"].as_str().map(String::from),
        channel_id: snippet["channelId"].as_str().unwrap_or("").to_string(),
        channel_title: snippet["channelTitle"].as_str().unwrap_or("").to_string(),
        published_at: snippet["publishedAt"].as_str().map(String::from),
        thumbnail_url: snippet["thumbnails"]["high"]["url"]
            .as_str()
            .or_else(|| snippet["thumbnails"]["default"]["url"].as_str())
            .map(String::from),
        view_count: None,
        like_count: None,
        duration: None,
    })
}

fn parse_video_details(item: &Value) -> Option<YouTubeVideo> {
    let snippet = &item["snippet"];
    let statistics = &item["statistics"];
    let content_details = &item["contentDetails"];

    Some(YouTubeVideo {
        video_id: item["id"].as_str()?.to_string(),
        title: snippet["title"].as_str().unwrap_or("").to_string(),
        description: snippet["description"].as_str().map(String::from),
        channel_id: snippet["channelId"].as_str().unwrap_or("").to_string(),
        channel_title: snippet["channelTitle"].as_str().unwrap_or("").to_string(),
        published_at: snippet["publishedAt"].as_str().map(String::from),
        thumbnail_url: snippet["thumbnails"]["high"]["url"]
            .as_str()
            .or_else(|| snippet["thumbnails"]["default"]["url"].as_str())
            .map(String::from),
        view_count: statistics["viewCount"]
            .as_str()
            .and_then(|s| s.parse().ok()),
        like_count: statistics["likeCount"]
            .as_str()
            .and_then(|s| s.parse().ok()),
        duration: content_details["duration"].as_str().map(String::from),
    })
}

// =============================================================================
// Search Videos Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct SearchYouTubeVideosNode {}

impl SearchYouTubeVideosNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SearchYouTubeVideosNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_youtube_search",
            "Search Videos",
            "Search for YouTube videos",
            "Data/Google/YouTube",
        );
        node.set_flowscript_name("google.youtube", "search");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("query", "Query", "Search query", VariableType::String);
        node.add_input_pin(
            "max_results",
            "Max Results",
            "Maximum number of results (1-50)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));
        node.add_input_pin("order", "Order", "Sort order", VariableType::String)
            .set_default_value(Some(json!("relevance")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "relevance".to_string(),
                        "date".to_string(),
                        "viewCount".to_string(),
                        "rating".to_string(),
                    ])
                    .build(),
            );
        node.add_input_pin(
            "page_token",
            "Page Token",
            "Token for pagination",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("videos", "Videos", "List of videos", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<YouTubeVideo>();
        node.add_output_pin(
            "next_page_token",
            "Next Page Token",
            "",
            VariableType::String,
        );
        node.add_output_pin(
            "total_results",
            "Total Results",
            "Estimated total results",
            VariableType::Integer,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/youtube.readonly"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let query: String = context.evaluate_pin("query").await?;
        let max_results: i64 = context
            .evaluate_pin("max_results")
            .await
            .unwrap_or(10)
            .min(50);
        let order: String = context
            .evaluate_pin("order")
            .await
            .unwrap_or_else(|_| "relevance".to_string());
        let page_token: String = context.evaluate_pin("page_token").await.unwrap_or_default();

        let mut query_params = vec![
            ("part", "snippet".to_string()),
            ("type", "video".to_string()),
            ("q", query),
            ("maxResults", max_results.to_string()),
            ("order", order),
        ];

        if !page_token.is_empty() {
            query_params.push(("pageToken", page_token));
        }

        let client = reqwest::Client::new();
        let response = client
            .get("https://www.googleapis.com/youtube/v3/search")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&query_params)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let videos: Vec<YouTubeVideo> = body["items"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_video_from_search).collect())
                    .unwrap_or_default();
                let next_page_token = body["nextPageToken"].as_str().unwrap_or("").to_string();
                let total_results = body["pageInfo"]["totalResults"].as_i64().unwrap_or(0);

                context.set_pin_value("videos", json!(videos)).await?;
                context
                    .set_pin_value("next_page_token", json!(next_page_token))
                    .await?;
                context
                    .set_pin_value("total_results", json!(total_results))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Get Video Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetYouTubeVideoNode {}

impl GetYouTubeVideoNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetYouTubeVideoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_youtube_get_video",
            "Get Video",
            "Get YouTube video details by ID",
            "Data/Google/YouTube",
        );
        node.set_flowscript_name("google.youtube", "getVideo");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "video_id",
            "Video ID",
            "YouTube video ID",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("video", "Video", "Video details", VariableType::Struct)
            .set_schema::<YouTubeVideo>();
        node.add_output_pin("raw", "Raw", "Raw API response", VariableType::Generic);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/youtube.readonly"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let video_id: String = context.evaluate_pin("video_id").await?;

        let client = reqwest::Client::new();
        let response = client
            .get("https://www.googleapis.com/youtube/v3/videos")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[
                ("part", "snippet,statistics,contentDetails"),
                ("id", &video_id),
            ])
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(item) = body["items"].as_array().and_then(|arr| arr.first()) {
                    if let Some(video) = parse_video_details(item) {
                        context.set_pin_value("video", json!(video)).await?;
                        context.set_pin_value("raw", body).await?;
                        context.activate_exec_pin("exec_out").await?;
                    } else {
                        context
                            .set_pin_value("error_message", json!("Failed to parse video"))
                            .await?;
                        context.activate_exec_pin("error").await?;
                    }
                } else {
                    context
                        .set_pin_value("error_message", json!("Video not found"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// List My Videos Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListMyYouTubeVideosNode {}

impl ListMyYouTubeVideosNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListMyYouTubeVideosNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_youtube_list_my_videos",
            "List My Videos",
            "List videos from the authenticated user's channel",
            "Data/Google/YouTube",
        );
        node.set_flowscript_name("google.youtube", "listMyVideos");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "max_results",
            "Max Results",
            "Maximum results (1-50)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));
        node.add_input_pin(
            "page_token",
            "Page Token",
            "Token for pagination",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("videos", "Videos", "List of videos", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<YouTubeVideo>();
        node.add_output_pin(
            "next_page_token",
            "Next Page Token",
            "",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/youtube.readonly"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let max_results: i64 = context
            .evaluate_pin("max_results")
            .await
            .unwrap_or(10)
            .min(50);
        let page_token: String = context.evaluate_pin("page_token").await.unwrap_or_default();

        let mut query_params = vec![
            ("part", "snippet".to_string()),
            ("forMine", "true".to_string()),
            ("type", "video".to_string()),
            ("maxResults", max_results.to_string()),
        ];

        if !page_token.is_empty() {
            query_params.push(("pageToken", page_token));
        }

        let client = reqwest::Client::new();
        let response = client
            .get("https://www.googleapis.com/youtube/v3/search")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&query_params)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let videos: Vec<YouTubeVideo> = body["items"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_video_from_search).collect())
                    .unwrap_or_default();
                let next_page_token = body["nextPageToken"].as_str().unwrap_or("").to_string();

                context.set_pin_value("videos", json!(videos)).await?;
                context
                    .set_pin_value("next_page_token", json!(next_page_token))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Get Channel Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetYouTubeChannelNode {}

impl GetYouTubeChannelNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetYouTubeChannelNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_youtube_get_channel",
            "Get Channel",
            "Get YouTube channel details",
            "Data/Google/YouTube",
        );
        node.set_flowscript_name("google.youtube", "getChannel");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "channel_id",
            "Channel ID",
            "YouTube channel ID (leave empty for own channel)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "channel",
            "Channel",
            "Channel details",
            VariableType::Struct,
        )
        .set_schema::<YouTubeChannel>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/youtube.readonly"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let channel_id: String = context.evaluate_pin("channel_id").await.unwrap_or_default();

        let mut query_params = vec![("part".to_string(), "snippet,statistics".to_string())];

        if channel_id.is_empty() {
            query_params.push(("mine".to_string(), "true".to_string()));
        } else {
            query_params.push(("id".to_string(), channel_id));
        }

        let client = reqwest::Client::new();
        let response = client
            .get("https://www.googleapis.com/youtube/v3/channels")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&query_params)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(item) = body["items"].as_array().and_then(|arr| arr.first()) {
                    let snippet = &item["snippet"];
                    let statistics = &item["statistics"];
                    let channel = YouTubeChannel {
                        channel_id: item["id"].as_str().unwrap_or("").to_string(),
                        title: snippet["title"].as_str().unwrap_or("").to_string(),
                        description: snippet["description"].as_str().map(String::from),
                        custom_url: snippet["customUrl"].as_str().map(String::from),
                        thumbnail_url: snippet["thumbnails"]["high"]["url"]
                            .as_str()
                            .map(String::from),
                        subscriber_count: statistics["subscriberCount"]
                            .as_str()
                            .and_then(|s| s.parse().ok()),
                        video_count: statistics["videoCount"]
                            .as_str()
                            .and_then(|s| s.parse().ok()),
                    };
                    context.set_pin_value("channel", json!(channel)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Channel not found"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// List Playlists Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListYouTubePlaylistsNode {}

impl ListYouTubePlaylistsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListYouTubePlaylistsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_youtube_list_playlists",
            "List Playlists",
            "List YouTube playlists",
            "Data/Google/YouTube",
        );
        node.set_flowscript_name("google.youtube", "listPlaylists");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "channel_id",
            "Channel ID",
            "Channel ID (empty for own playlists)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "max_results",
            "Max Results",
            "Maximum results (1-50)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(25)));
        node.add_input_pin(
            "page_token",
            "Page Token",
            "Token for pagination",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "playlists",
            "Playlists",
            "List of playlists",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<YouTubePlaylist>();
        node.add_output_pin(
            "next_page_token",
            "Next Page Token",
            "",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/youtube.readonly"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let channel_id: String = context.evaluate_pin("channel_id").await.unwrap_or_default();
        let max_results: i64 = context
            .evaluate_pin("max_results")
            .await
            .unwrap_or(25)
            .min(50);
        let page_token: String = context.evaluate_pin("page_token").await.unwrap_or_default();

        let mut query_params = vec![
            ("part".to_string(), "snippet,contentDetails".to_string()),
            ("maxResults".to_string(), max_results.to_string()),
        ];

        if channel_id.is_empty() {
            query_params.push(("mine".to_string(), "true".to_string()));
        } else {
            query_params.push(("channelId".to_string(), channel_id));
        }

        if !page_token.is_empty() {
            query_params.push(("pageToken".to_string(), page_token));
        }

        let client = reqwest::Client::new();
        let response = client
            .get("https://www.googleapis.com/youtube/v3/playlists")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&query_params)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let playlists: Vec<YouTubePlaylist> = body["items"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|item| {
                                let snippet = &item["snippet"];
                                Some(YouTubePlaylist {
                                    playlist_id: item["id"].as_str()?.to_string(),
                                    title: snippet["title"].as_str().unwrap_or("").to_string(),
                                    description: snippet["description"].as_str().map(String::from),
                                    channel_id: snippet["channelId"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                    item_count: item["contentDetails"]["itemCount"].as_i64(),
                                    thumbnail_url: snippet["thumbnails"]["high"]["url"]
                                        .as_str()
                                        .map(String::from),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let next_page_token = body["nextPageToken"].as_str().unwrap_or("").to_string();

                context.set_pin_value("playlists", json!(playlists)).await?;
                context
                    .set_pin_value("next_page_token", json!(next_page_token))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Get Playlist Items Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetYouTubePlaylistItemsNode {}

impl GetYouTubePlaylistItemsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetYouTubePlaylistItemsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_youtube_get_playlist_items",
            "Get Playlist Items",
            "Get videos in a YouTube playlist",
            "Data/Google/YouTube",
        );
        node.set_flowscript_name("google.youtube", "getPlaylistItems");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "playlist_id",
            "Playlist ID",
            "YouTube playlist ID",
            VariableType::String,
        );
        node.add_input_pin(
            "max_results",
            "Max Results",
            "Maximum results (1-50)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(25)));
        node.add_input_pin(
            "page_token",
            "Page Token",
            "Token for pagination",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("items", "Items", "Playlist items", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<YouTubePlaylistItem>();
        node.add_output_pin(
            "next_page_token",
            "Next Page Token",
            "",
            VariableType::String,
        );
        node.add_output_pin(
            "total_items",
            "Total Items",
            "Total items in playlist",
            VariableType::Integer,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/youtube.readonly"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let playlist_id: String = context.evaluate_pin("playlist_id").await?;
        let max_results: i64 = context
            .evaluate_pin("max_results")
            .await
            .unwrap_or(25)
            .min(50);
        let page_token: String = context.evaluate_pin("page_token").await.unwrap_or_default();

        let max_results_str = max_results.to_string();
        let mut query_params = vec![
            ("part", "snippet,contentDetails"),
            ("playlistId", &playlist_id),
            ("maxResults", &max_results_str),
        ];

        if !page_token.is_empty() {
            query_params.push(("pageToken", &page_token));
        }

        let client = reqwest::Client::new();
        let response = client
            .get("https://www.googleapis.com/youtube/v3/playlistItems")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&query_params)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let items: Vec<YouTubePlaylistItem> = body["items"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|item| {
                                let snippet = &item["snippet"];
                                Some(YouTubePlaylistItem {
                                    item_id: item["id"].as_str()?.to_string(),
                                    video_id: snippet["resourceId"]["videoId"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                    title: snippet["title"].as_str().unwrap_or("").to_string(),
                                    description: snippet["description"].as_str().map(String::from),
                                    position: snippet["position"].as_i64().unwrap_or(0),
                                    thumbnail_url: snippet["thumbnails"]["high"]["url"]
                                        .as_str()
                                        .map(String::from),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let next_page_token = body["nextPageToken"].as_str().unwrap_or("").to_string();
                let total_items = body["pageInfo"]["totalResults"].as_i64().unwrap_or(0);

                context.set_pin_value("items", json!(items)).await?;
                context
                    .set_pin_value("next_page_token", json!(next_page_token))
                    .await?;
                context
                    .set_pin_value("total_items", json!(total_items))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Add to Playlist Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct AddToYouTubePlaylistNode {}

impl AddToYouTubePlaylistNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AddToYouTubePlaylistNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_youtube_add_to_playlist",
            "Add to Playlist",
            "Add a video to a YouTube playlist",
            "Data/Google/YouTube",
        );
        node.set_flowscript_name("google.youtube", "addToPlaylist");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "playlist_id",
            "Playlist ID",
            "YouTube playlist ID",
            VariableType::String,
        );
        node.add_input_pin(
            "video_id",
            "Video ID",
            "YouTube video ID to add",
            VariableType::String,
        );
        node.add_input_pin(
            "position",
            "Position",
            "Position in playlist (optional, -1 for end)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "item_id",
            "Item ID",
            "ID of the playlist item",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/youtube"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let playlist_id: String = context.evaluate_pin("playlist_id").await?;
        let video_id: String = context.evaluate_pin("video_id").await?;
        let position: i64 = context.evaluate_pin("position").await.unwrap_or(-1);

        let mut snippet = json!({
            "playlistId": playlist_id,
            "resourceId": {
                "kind": "youtube#video",
                "videoId": video_id
            }
        });

        if position >= 0 {
            snippet["position"] = json!(position);
        }

        let request_body = json!({
            "snippet": snippet
        });

        let client = reqwest::Client::new();
        let response = client
            .post("https://www.googleapis.com/youtube/v3/playlistItems")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .query(&[("part", "snippet")])
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let item_id = body["id"].as_str().unwrap_or("").to_string();
                context.set_pin_value("item_id", json!(item_id)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Remove from Playlist Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct RemoveFromYouTubePlaylistNode {}

impl RemoveFromYouTubePlaylistNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RemoveFromYouTubePlaylistNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_youtube_remove_from_playlist",
            "Remove from Playlist",
            "Remove a video from a YouTube playlist",
            "Data/Google/YouTube",
        );
        node.set_flowscript_name("google.youtube", "removeFromPlaylist");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "item_id",
            "Item ID",
            "Playlist item ID (not video ID)",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/youtube"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let item_id: String = context.evaluate_pin("item_id").await?;

        let client = reqwest::Client::new();
        let response = client
            .delete("https://www.googleapis.com/youtube/v3/playlistItems")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[("id", &item_id)])
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 204 => {
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}
