use flow_like_model_provider::history::{
    Content, ContentType, History, HistoryMessage, ImageUrl, MessageContent, ResponseFormat,
};
use flow_like_types::utils::data_url::optimize_data_url;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LocalModelHistoryMessage {
    pub role: String,
    pub content: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LocalModelHistory {
    model: String,
    messages: Vec<HistoryMessage>,
    stream: Option<bool>,
    max_completion_tokens: Option<u32>,
    top_p: Option<f32>,
    temperature: Option<f32>,
    seed: Option<u32>,
    presence_penalty: Option<f32>,
    user: Option<String>,
    stop: Option<Vec<String>>,
    response_format: Option<ResponseFormat>,
}

impl LocalModelHistory {
    pub async fn from_history(history: &History) -> Self {
        LocalModelHistory {
            model: history.model.clone(),
            messages: parse_messages(history).await,
            stream: history.stream,
            max_completion_tokens: history.max_completion_tokens,
            top_p: history.top_p,
            temperature: history.temperature,
            seed: history.seed,
            presence_penalty: history.presence_penalty,
            user: history.user.clone(),
            stop: history.stop.clone(),
            response_format: history.response_format.clone(),
        }
    }
}

async fn parse_messages(history: &History) -> Vec<HistoryMessage> {
    let mut messages = Vec::with_capacity(history.messages.len());
    for message in &history.messages {
        let content: Vec<Content> = match message.content {
            MessageContent::String(ref text) => vec![Content::Text {
                content_type: ContentType::Text,
                text: text.clone(),
            }],
            MessageContent::Contents(ref contents) => contents.clone(),
        };

        let mut new_content = Vec::with_capacity(content.len());

        for content in &content {
            match content {
                Content::Text { text, .. } => {
                    new_content.push(Content::Text {
                        content_type: ContentType::Text,
                        text: text.clone(),
                    });
                }
                Content::Image { image_url, .. } => {
                    let optimized_url = optimize_data_url(&image_url.url).await;

                    match optimized_url {
                        Ok(url) => {
                            new_content.push(Content::Image {
                                content_type: ContentType::ImageUrl,
                                image_url: ImageUrl {
                                    url,
                                    detail: image_url.detail.clone(),
                                    media_type: image_url.media_type.clone(),
                                    additional_params: image_url.additional_params.clone(),
                                },
                            });
                        }
                        Err(_) => {
                            eprintln!("Failed to optimize image URL");
                            new_content.push(Content::Text {
                                content_type: ContentType::Text,
                                text: "The user tried to send an image, but it could not be optimized.".to_string(),
                            });
                        }
                    }
                }
                other => new_content.push(other.clone()),
            }
        }

        messages.push(HistoryMessage {
            role: message.role.clone(),
            content: MessageContent::Contents(new_content),
            name: None,
            tool_call_id: None,
            tool_calls: None,
            annotations: None,
        });
    }
    messages
}
