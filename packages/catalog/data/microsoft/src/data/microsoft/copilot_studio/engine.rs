//! Direct-to-Engine: `POST …/conversations[/{id}]` answered by an SSE stream of `activity` events,
//! as implemented by the Microsoft 365 Agents SDK `CopilotStudioClient`.

use super::{
    agent::CopilotStudioAgent,
    http_failure,
    provider::CopilotStudioProvider,
    turn::{StreamEmitter, TurnCollector},
};
use eventsource_stream::Eventsource;
use flow_like::flow::execution::context::ExecutionContext;
use flow_like_types::{
    Result, Value, anyhow,
    futures::StreamExt,
    json::{self, json},
    reqwest::{self, Url},
    tokio::time::timeout,
};
use std::time::Duration;

const CONVERSATION_ID_HEADER: &str = "x-ms-conversationid";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Agent flows may run for minutes before the next activity arrives.
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

pub(crate) enum EngineRequest {
    Start {
        run_greeting: bool,
        locale: String,
    },
    Execute {
        conversation_id: String,
        activity: Value,
    },
}

impl EngineRequest {
    fn operation(&self) -> &'static str {
        match self {
            Self::Start { .. } => "Starting the Copilot Studio conversation",
            Self::Execute { .. } => "Sending the activity to Copilot Studio",
        }
    }

    fn url_and_body(&self, agent: &CopilotStudioAgent) -> Result<(Url, Value)> {
        match self {
            Self::Start {
                run_greeting,
                locale,
            } => Ok((
                agent.conversation_url(None)?,
                json!({ "emitStartConversationEvent": run_greeting, "locale": locale }),
            )),
            Self::Execute {
                conversation_id,
                activity,
            } => {
                let mut activity = activity.clone();
                activity["conversation"] = json!({ "id": conversation_id });
                Ok((
                    agent.conversation_url(Some(conversation_id.as_str()))?,
                    json!({ "activity": activity }),
                ))
            }
        }
    }
}

/// Runs one Direct-to-Engine call and streams every activity into `collector`.
pub(crate) async fn run_engine_request(
    context: &mut ExecutionContext,
    provider: &CopilotStudioProvider,
    agent: &CopilotStudioAgent,
    request: EngineRequest,
    collector: &mut TurnCollector,
    emitter: &mut StreamEmitter,
) -> Result<()> {
    let operation = request.operation();
    let (url, body) = request.url_and_body(agent)?;
    let response = send(provider, url, &body, operation).await?;

    if let Some(conversation_id) = header(&response, CONVERSATION_ID_HEADER) {
        collector.set_conversation_id(&conversation_id);
    }
    let is_json = header(&response, reqwest::header::CONTENT_TYPE.as_str())
        .is_some_and(|value| value.starts_with("application/json"));
    if !is_json {
        return stream_activities(context, response, collector, emitter, operation).await;
    }
    for activity in json_activities(response, collector, operation).await? {
        let updates = collector.ingest(activity);
        emitter.emit_all(context, updates).await?;
    }
    Ok(())
}

async fn send(
    provider: &CopilotStudioProvider,
    url: Url,
    body: &Value,
    operation: &str,
) -> Result<reqwest::Response> {
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()?;
    let response = client
        .post(url)
        .bearer_auth(&provider.access_token)
        .header("Accept", "text/event-stream")
        .json(body)
        .send()
        .await
        .map_err(|error| anyhow!("{operation} failed: {error}"))?;
    if !response.status().is_success() {
        return Err(anyhow!(http_failure(operation, response).await));
    }
    Ok(response)
}

fn header(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

async fn stream_activities(
    context: &mut ExecutionContext,
    response: reqwest::Response,
    collector: &mut TurnCollector,
    emitter: &mut StreamEmitter,
    operation: &str,
) -> Result<()> {
    let mut events = response.bytes_stream().eventsource();
    loop {
        let next = timeout(IDLE_TIMEOUT, events.next()).await.map_err(|_| {
            anyhow!(
                "{operation}: no activity from the agent for {} seconds",
                IDLE_TIMEOUT.as_secs()
            )
        })?;
        let Some(event) = next else { return Ok(()) };
        let event = event.map_err(|error| anyhow!("{operation}: broken event stream: {error}"))?;
        if event.event == "end" {
            return Ok(());
        }
        if let Some(activity) = parse_activity_event(&event.event, &event.data, operation)? {
            let updates = collector.ingest(activity);
            emitter.emit_all(context, updates).await?;
        }
    }
}

/// `activity` frames carry one Bot Framework activity; a frame without event name defaults to
/// `message` in SSE and is accepted when it holds an activity object.
fn parse_activity_event(event: &str, data: &str, operation: &str) -> Result<Option<Value>> {
    if event != "activity" && event != "message" {
        return Ok(None);
    }
    let activity: Value = json::from_str(data)
        .map_err(|error| anyhow!("{operation}: activity is not valid JSON ({error})"))?;
    Ok(activity.is_object().then_some(activity))
}

/// Non-streaming fallback the .NET client accepts: `{"conversationId": …, "activities": […]}`.
async fn json_activities(
    response: reqwest::Response,
    collector: &mut TurnCollector,
    operation: &str,
) -> Result<Vec<Value>> {
    let payload: Value = response
        .json()
        .await
        .map_err(|error| anyhow!("{operation} returned invalid JSON: {error}"))?;
    if let Some(conversation_id) = payload["conversationId"].as_str() {
        collector.set_conversation_id(conversation_id);
    }
    Ok(match payload {
        Value::Object(mut map) => match map.remove("activities") {
            Some(Value::Array(activities)) => activities,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    #[test]
    fn only_activity_frames_with_objects_are_ingested() {
        let parsed = parse_activity_event("activity", r#"{"type":"message"}"#, "op").unwrap();
        assert_eq!(parsed.unwrap()["type"], "message");
        assert!(parse_activity_event("state", "{}", "op").unwrap().is_none());
        assert!(
            parse_activity_event("activity", "[]", "op")
                .unwrap()
                .is_none()
        );
        assert!(parse_activity_event("activity", "not json", "op").is_err());
    }

    #[tokio::test]
    async fn split_sse_frames_reassemble_into_activities() {
        let chunks: Vec<Result<&[u8], std::io::Error>> = vec![
            Ok(b"event: activity\r\ndata: {\"type\":\"typing\",\"te"),
            Ok(b"xt\":\"Hi\"}\r\n\r\nevent: activity\ndata: {\"type\":\"message\",\"text\":\"Hi!\"}\n\n"),
            Ok(b"event: end\ndata: end\n\n"),
        ];
        let events: Vec<_> = stream::iter(chunks).eventsource().collect().await;
        let names: Vec<String> = events
            .iter()
            .map(|event| event.as_ref().unwrap().event.clone())
            .collect();
        assert_eq!(names, vec!["activity", "activity", "end"]);
        let first = events[0].as_ref().unwrap();
        let activity = parse_activity_event(&first.event, &first.data, "op")
            .unwrap()
            .unwrap();
        assert_eq!(activity["text"], "Hi");
    }
}
