use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use flow_like_types::Value;
use flow_like_types::json::{self as serde_json, json};
use flow_like_types::reqwest;
use rig::{
    OneOrMany,
    client::{ClientBuilderError, CompletionClient},
    completion::{self, CompletionError, CompletionRequest, GetTokenUsage, Usage},
    message::{self, MimeType},
    streaming,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, convert::TryFrom};

#[derive(Clone, Debug, Default)]
struct ToolArgumentSpec {
    ordered_parameter_names: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct LlamaCppClient {
    base_url: String,
    http_client: reqwest::Client,
}

impl LlamaCppClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            http_client: reqwest::Client::new(),
        }
    }

    fn post(&self, path: &str) -> Result<reqwest::RequestBuilder, ClientBuilderError> {
        let url = format!("{}/{}", self.base_url, path);
        Ok(self.http_client.post(url))
    }

    pub fn completion_model(&self, model: &str) -> CompletionModel {
        CompletionModel::new(self.clone(), model)
    }
}

impl CompletionClient for LlamaCppClient {
    type CompletionModel = CompletionModel;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: ApiUsage,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: ResponseMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub r#type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

impl TryFrom<CompletionResponse> for completion::CompletionResponse<CompletionResponse> {
    type Error = CompletionError;

    fn try_from(resp: CompletionResponse) -> Result<Self, Self::Error> {
        let first_choice = resp
            .choices
            .first()
            .ok_or_else(|| CompletionError::ResponseError("No choices in response".to_string()))?;

        let mut assistant_contents = Vec::new();

        if let Some(content) = &first_choice.message.content
            && !content.is_empty()
        {
            assistant_contents.push(completion::AssistantContent::text(content));
        }

        for tc in &first_choice.message.tool_calls {
            let args_value: Value =
                serde_json::from_str(&tc.function.arguments).unwrap_or_else(|_| json!({}));
            assistant_contents.push(completion::AssistantContent::tool_call(
                tc.id.clone(),
                tc.function.name.clone(),
                args_value,
            ));
        }

        let choice = OneOrMany::many(assistant_contents)
            .map_err(|_| CompletionError::ResponseError("No content provided".to_owned()))?;

        Ok(completion::CompletionResponse {
            choice,
            message_id: None,
            usage: Usage {
                input_tokens: resp.usage.prompt_tokens,
                output_tokens: resp.usage.completion_tokens,
                total_tokens: resp.usage.total_tokens,
                cache_creation_input_tokens: 0,
                cached_input_tokens: 0,
                tool_use_prompt_tokens: 0,
                reasoning_tokens: 0,
            },
            raw_response: resp,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StreamingFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StreamingToolCall {
    pub index: usize,
    pub id: Option<String>,
    pub function: StreamingFunction,
}

#[derive(Debug, Default)]
struct AccumulatedToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StreamingDelta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<StreamingToolCall>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StreamingChoice {
    pub delta: StreamingDelta,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StreamingChunk {
    pub choices: Vec<StreamingChoice>,
    pub usage: Option<ApiUsage>,
}

#[derive(Clone)]
pub struct CompletionModel {
    client: LlamaCppClient,
    pub model: String,
}

impl CompletionModel {
    pub fn new(client: LlamaCppClient, model: &str) -> Self {
        Self {
            client,
            model: model.to_owned(),
        }
    }

    fn tool_choice_value(tool_choice: &message::ToolChoice) -> Value {
        match tool_choice {
            message::ToolChoice::Auto => json!("auto"),
            message::ToolChoice::None => json!("none"),
            message::ToolChoice::Required => json!("required"),
            message::ToolChoice::Specific { function_names } => {
                let function_name = function_names.first().cloned().unwrap_or_default();
                json!({
                    "type": "function",
                    "function": {
                        "name": function_name,
                    }
                })
            }
        }
    }

    fn enable_stream_usage(request_payload: &mut Value) {
        let Some(obj) = request_payload.as_object_mut() else {
            return;
        };

        match obj.get_mut("stream_options") {
            Some(stream_options) if stream_options.is_object() => {
                stream_options["include_usage"] = json!(true);
            }
            _ => {
                obj.insert(
                    "stream_options".to_string(),
                    json!({ "include_usage": true }),
                );
            }
        }
    }

    fn fallback_tool_calls_from_text(
        content: &str,
        tool_argument_specs: &HashMap<String, ToolArgumentSpec>,
    ) -> Vec<ToolCall> {
        if tool_argument_specs.is_empty() {
            return Vec::new();
        }

        let normalized = Self::strip_code_fences(content);
        let allowed_tool_names = tool_argument_specs.keys().cloned().collect::<Vec<_>>();

        let mut tool_calls = Self::fallback_tool_calls_from_parenthesized_text(
            &normalized,
            &allowed_tool_names,
            tool_argument_specs,
        );

        if tool_calls.is_empty() {
            tool_calls = Self::fallback_tool_calls_from_bare_lines(
                &normalized,
                &allowed_tool_names,
                tool_argument_specs,
            );
        }

        tool_calls
    }

    fn fallback_tool_calls_from_parenthesized_text(
        content: &str,
        allowed_tool_names: &[String],
        tool_argument_specs: &HashMap<String, ToolArgumentSpec>,
    ) -> Vec<ToolCall> {
        let mut tool_calls = Vec::new();
        let mut search_from = 0usize;

        while let Some((tool_name, open_paren)) =
            Self::find_next_tool_invocation(content, allowed_tool_names, search_from)
        {
            let Some((args_str, next_index)) =
                Self::extract_parenthesized_arguments(content, open_paren)
            else {
                break;
            };

            let tool_argument_spec = tool_argument_specs.get(&tool_name);
            if let Some(arguments) = Self::parse_tool_arguments(&args_str, tool_argument_spec) {
                let arguments =
                    serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".to_string());
                tool_calls.push(ToolCall {
                    id: format!("fallback_tool_call_{}", tool_calls.len()),
                    r#type: "function".to_string(),
                    function: FunctionCall {
                        name: tool_name,
                        arguments,
                    },
                });
            }

            search_from = next_index;
        }

        tool_calls
    }

    fn fallback_tool_calls_from_bare_lines(
        content: &str,
        allowed_tool_names: &[String],
        tool_argument_specs: &HashMap<String, ToolArgumentSpec>,
    ) -> Vec<ToolCall> {
        let mut tool_calls = Vec::new();
        let lines = content.lines().collect::<Vec<_>>();
        let mut index = 0usize;

        while index < lines.len() {
            let line = lines[index].trim();
            if line.is_empty() || line.starts_with("```") {
                index += 1;
                continue;
            }

            let Some((tool_name, inline_arguments)) =
                Self::match_bare_tool_invocation(line, allowed_tool_names)
            else {
                index += 1;
                continue;
            };

            let mut next_index = index + 1;
            let arguments = if inline_arguments.is_empty() {
                let mut continuation = Vec::new();

                while next_index < lines.len() {
                    let next_line = lines[next_index].trim();
                    if next_line.is_empty() || next_line.starts_with("```") {
                        break;
                    }

                    if Self::match_bare_tool_invocation(next_line, allowed_tool_names).is_some() {
                        break;
                    }

                    continuation.push(next_line);
                    next_index += 1;
                }

                continuation.join(" ")
            } else {
                inline_arguments
            };

            let tool_argument_spec = tool_argument_specs.get(&tool_name);
            if let Some(parsed_arguments) =
                Self::parse_tool_arguments(&arguments, tool_argument_spec)
            {
                let arguments =
                    serde_json::to_string(&parsed_arguments).unwrap_or_else(|_| "{}".to_string());
                tool_calls.push(ToolCall {
                    id: format!("fallback_tool_call_{}", tool_calls.len()),
                    r#type: "function".to_string(),
                    function: FunctionCall {
                        name: tool_name,
                        arguments,
                    },
                });
            }

            index = next_index.max(index + 1);
        }

        tool_calls
    }

    fn match_bare_tool_invocation(
        line: &str,
        allowed_tool_names: &[String],
    ) -> Option<(String, String)> {
        for tool_name in allowed_tool_names {
            let Some(rest) = line.strip_prefix(tool_name) else {
                continue;
            };

            if rest
                .chars()
                .next()
                .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
            {
                continue;
            }

            let arguments = rest.trim_start();
            if arguments.starts_with('(') {
                continue;
            }

            return Some((tool_name.clone(), arguments.to_string()));
        }

        None
    }

    fn strip_code_fences(content: &str) -> String {
        let trimmed = content.trim();
        if let Some(rest) = trimmed.strip_prefix("```")
            && let Some(end) = rest.rfind("```")
        {
            let inner = &rest[..end];
            let inner = inner
                .strip_prefix("python\n")
                .or_else(|| inner.strip_prefix("json\n"))
                .or_else(|| inner.strip_prefix("tool_call\n"))
                .unwrap_or(inner);
            return inner.trim().to_string();
        }

        trimmed.to_string()
    }

    fn find_next_tool_invocation(
        content: &str,
        allowed_tool_names: &[String],
        start_at: usize,
    ) -> Option<(String, usize)> {
        let mut best_match: Option<(usize, String, usize)> = None;

        for tool_name in allowed_tool_names {
            let mut search_index = start_at;

            while search_index < content.len() {
                let Some(relative_index) = content[search_index..].find(tool_name) else {
                    break;
                };

                let match_index = search_index + relative_index;
                let before = content[..match_index].chars().next_back();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    search_index = match_index + tool_name.len();
                    continue;
                }

                let after_name = match_index + tool_name.len();
                let mut open_paren_index = after_name;
                while let Some(ch) = content[open_paren_index..].chars().next() {
                    if ch.is_whitespace() {
                        open_paren_index += ch.len_utf8();
                        continue;
                    }
                    break;
                }

                if content[open_paren_index..].starts_with('(') {
                    match &best_match {
                        Some((best_index, _, _)) if *best_index <= match_index => {}
                        _ => {
                            best_match = Some((match_index, tool_name.clone(), open_paren_index));
                        }
                    }
                    break;
                }

                search_index = match_index + tool_name.len();
            }
        }

        best_match.map(|(_, tool_name, open_paren)| (tool_name, open_paren))
    }

    fn extract_parenthesized_arguments(
        content: &str,
        open_paren_index: usize,
    ) -> Option<(String, usize)> {
        let bytes = content.as_bytes();
        if bytes.get(open_paren_index) != Some(&b'(') {
            return None;
        }

        let mut depth = 0usize;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        for index in open_paren_index..bytes.len() {
            let ch = bytes[index] as char;

            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if in_single || in_double => {
                    escaped = true;
                }
                '\'' if !in_double => {
                    in_single = !in_single;
                }
                '"' if !in_single => {
                    in_double = !in_double;
                }
                '(' if !in_single && !in_double => {
                    depth += 1;
                }
                ')' if !in_single && !in_double => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some((
                            content[open_paren_index + 1..index].trim().to_string(),
                            index + 1,
                        ));
                    }
                }
                _ => {}
            }
        }

        None
    }

    fn parse_tool_arguments(
        arguments: &str,
        tool_argument_spec: Option<&ToolArgumentSpec>,
    ) -> Option<Value> {
        let trimmed = arguments.trim();
        if trimmed.is_empty() {
            return Some(json!({}));
        }

        if trimmed.starts_with('{') && trimmed.ends_with('}') {
            return Self::parse_jsonish_value(trimmed);
        }

        let parts = Self::split_tool_arguments(trimmed);
        let mut object = serde_json::Map::new();
        let ordered_parameter_names = tool_argument_spec
            .map(|spec| spec.ordered_parameter_names.as_slice())
            .unwrap_or(&[]);
        let mut positional_index = 0usize;

        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            let (key, value_str) =
                if let Some(separator_index) = Self::find_top_level_separator(part) {
                    (
                        part[..separator_index]
                            .trim()
                            .trim_matches('"')
                            .trim_matches('\'')
                            .to_string(),
                        part[separator_index + 1..].trim().to_string(),
                    )
                } else {
                    let Some(parameter_name) = ordered_parameter_names.get(positional_index) else {
                        return None;
                    };
                    positional_index += 1;
                    (parameter_name.clone(), part.to_string())
                };

            let value = Self::parse_jsonish_value(value_str.as_str())?;

            if let Some(parameter_position) = ordered_parameter_names
                .iter()
                .position(|parameter_name| parameter_name == &key)
            {
                positional_index = positional_index.max(parameter_position + 1);
            }

            object.insert(key, value);
        }

        Some(Value::Object(object))
    }

    fn tool_argument_specs(
        tools: &[completion::ToolDefinition],
    ) -> HashMap<String, ToolArgumentSpec> {
        tools
            .iter()
            .map(|tool| {
                (
                    tool.name.clone(),
                    ToolArgumentSpec {
                        ordered_parameter_names: Self::ordered_parameter_names(&tool.parameters),
                    },
                )
            })
            .collect()
    }

    fn ordered_parameter_names(parameters: &Value) -> Vec<String> {
        let mut ordered_parameter_names = Vec::new();

        if let Some(required_parameters) = parameters
            .get("required")
            .and_then(|required| required.as_array())
        {
            for required_parameter in required_parameters {
                if let Some(required_parameter) = required_parameter.as_str()
                    && !ordered_parameter_names
                        .iter()
                        .any(|name| name == required_parameter)
                {
                    ordered_parameter_names.push(required_parameter.to_string());
                }
            }
        }

        if let Some(properties) = parameters
            .get("properties")
            .and_then(|properties| properties.as_object())
        {
            for property_name in properties.keys() {
                if !ordered_parameter_names
                    .iter()
                    .any(|name| name == property_name)
                {
                    ordered_parameter_names.push(property_name.clone());
                }
            }
        }

        ordered_parameter_names
    }

    fn find_top_level_separator(input: &str) -> Option<usize> {
        let bytes = input.as_bytes();
        let mut depth = 0usize;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        for (index, byte) in bytes.iter().enumerate() {
            let ch = *byte as char;

            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if in_single || in_double => escaped = true,
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '{' | '[' | '(' if !in_single && !in_double => depth += 1,
                '}' | ']' | ')' if !in_single && !in_double => depth = depth.saturating_sub(1),
                '=' | ':' if !in_single && !in_double && depth == 0 => return Some(index),
                _ => {}
            }
        }

        None
    }

    fn split_tool_arguments(input: &str) -> Vec<String> {
        let bytes = input.as_bytes();
        let mut parts = Vec::new();
        let mut start = 0usize;
        let mut index = 0usize;
        let mut depth = 0usize;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        while index < bytes.len() {
            let ch = bytes[index] as char;

            if escaped {
                escaped = false;
                index += 1;
                continue;
            }

            match ch {
                '\\' if in_single || in_double => {
                    escaped = true;
                    index += 1;
                    continue;
                }
                '\'' if !in_double => {
                    in_single = !in_single;
                    index += 1;
                    continue;
                }
                '"' if !in_single => {
                    in_double = !in_double;
                    index += 1;
                    continue;
                }
                '{' | '[' | '(' if !in_single && !in_double => {
                    depth += 1;
                    index += 1;
                    continue;
                }
                '}' | ']' | ')' if !in_single && !in_double => {
                    depth = depth.saturating_sub(1);
                    index += 1;
                    continue;
                }
                ',' if !in_single && !in_double && depth == 0 => {
                    parts.push(input[start..index].trim().to_string());
                    start = index + 1;
                    index += 1;
                    continue;
                }
                _ => {}
            }

            if ch.is_whitespace() && !in_single && !in_double && depth == 0 {
                let mut next_index = index;
                while next_index < bytes.len() && (bytes[next_index] as char).is_whitespace() {
                    next_index += 1;
                }

                if next_index < bytes.len() && Self::starts_with_argument_key(&input[next_index..])
                {
                    parts.push(input[start..index].trim().to_string());
                    start = next_index;
                    index = next_index;
                    continue;
                }
            }

            index += 1;
        }

        if start <= input.len() {
            parts.push(input[start..].trim().to_string());
        }

        parts.into_iter().filter(|part| !part.is_empty()).collect()
    }

    fn starts_with_argument_key(input: &str) -> bool {
        let trimmed = input.trim_start();
        if trimmed.is_empty() {
            return false;
        }

        let mut chars = trimmed.char_indices();
        let Some((_, first_char)) = chars.next() else {
            return false;
        };

        let key_end = if first_char == '\'' || first_char == '"' {
            let quote = first_char;
            let mut escaped = false;
            let mut end = None;

            for (index, ch) in trimmed.char_indices().skip(1) {
                if escaped {
                    escaped = false;
                    continue;
                }

                if ch == '\\' {
                    escaped = true;
                    continue;
                }

                if ch == quote {
                    end = Some(index + ch.len_utf8());
                    break;
                }
            }

            let Some(end) = end else {
                return false;
            };
            end
        } else {
            if !first_char.is_ascii_alphabetic() && first_char != '_' {
                return false;
            }

            let mut end = first_char.len_utf8();
            for (index, ch) in trimmed.char_indices().skip(1) {
                if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                    end = index + ch.len_utf8();
                    continue;
                }
                break;
            }
            end
        };

        trimmed[key_end..].trim_start().starts_with(['=', ':'])
    }

    fn parse_jsonish_value(input: &str) -> Option<Value> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Some(Value::Null);
        }

        match trimmed {
            "true" | "True" => return Some(json!(true)),
            "false" | "False" => return Some(json!(false)),
            "null" | "None" => return Some(Value::Null),
            _ => {}
        }

        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            return Some(value);
        }

        if trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2 {
            let inner = &trimmed[1..trimmed.len() - 1];
            return Some(json!(inner.replace("\\'", "'").replace("\\\"", "\"")));
        }

        if let Ok(value) = trimmed.parse::<i64>() {
            return Some(json!(value));
        }

        if let Ok(value) = trimmed.parse::<f64>() {
            return Some(json!(value));
        }

        if (trimmed.starts_with('{') && trimmed.ends_with('}'))
            || (trimmed.starts_with('[') && trimmed.ends_with(']'))
        {
            let normalized = Self::normalize_jsonish(trimmed);
            if let Ok(value) = serde_json::from_str::<Value>(&normalized) {
                return Some(value);
            }
        }

        Some(json!(trimmed))
    }

    fn normalize_jsonish(input: &str) -> String {
        let mut output = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        while let Some(ch) = chars.next() {
            if escaped {
                if in_single && ch == '"' {
                    output.push('\\');
                }
                output.push(ch);
                escaped = false;
                continue;
            }

            if in_single {
                match ch {
                    '\\' => {
                        output.push('\\');
                        escaped = true;
                    }
                    '\'' => {
                        output.push('"');
                        in_single = false;
                    }
                    '"' => {
                        output.push('\\');
                        output.push('"');
                    }
                    _ => output.push(ch),
                }
                continue;
            }

            if in_double {
                match ch {
                    '\\' => {
                        output.push('\\');
                        escaped = true;
                    }
                    '"' => {
                        output.push('"');
                        in_double = false;
                    }
                    _ => output.push(ch),
                }
                continue;
            }

            match ch {
                '\'' => {
                    output.push('"');
                    in_single = true;
                }
                '"' => {
                    output.push('"');
                    in_double = true;
                }
                c if c.is_ascii_alphabetic() => {
                    let mut identifier = String::from(c);
                    while let Some(next) = chars.peek() {
                        if next.is_ascii_alphanumeric() || *next == '_' {
                            identifier.push(*next);
                            chars.next();
                        } else {
                            break;
                        }
                    }

                    match identifier.as_str() {
                        "True" => output.push_str("true"),
                        "False" => output.push_str("false"),
                        "None" => output.push_str("null"),
                        _ => output.push_str(&identifier),
                    }
                }
                _ => output.push(ch),
            }
        }

        output
    }

    fn normalize_reasoning_preview(content: &str) -> String {
        content.to_string()
    }

    fn should_preserve_canonical_messages(messages: &[Value]) -> bool {
        messages.iter().any(|message| {
            message.get("role").and_then(|r| r.as_str()) == Some("tool")
                || message
                    .get("tool_calls")
                    .and_then(|tool_calls| tool_calls.as_array())
                    .is_some_and(|tool_calls| !tool_calls.is_empty())
        })
    }

    fn merge_system_into_first_user(messages: Vec<Value>) -> Vec<Value> {
        let mut system_parts: Vec<String> = Vec::new();
        let mut non_system: Vec<Value> = Vec::new();

        for message in &messages {
            if let Some(role) = message.get("role").and_then(|r| r.as_str()) {
                if role == "system" {
                    if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
                        system_parts.push(content.to_string());
                    }
                } else {
                    non_system.push(message.clone());
                }
            } else {
                non_system.push(message.clone());
            }
        }

        if !system_parts.is_empty() {
            let system_text = system_parts.join("\n\n");
            if let Some(first_user) = non_system
                .iter_mut()
                .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
            {
                let content = first_user.get("content").cloned().unwrap_or(json!(""));
                if content.is_array() {
                    let mut parts = vec![json!({"type": "text", "text": system_text})];
                    parts.extend(content.as_array().unwrap().iter().cloned());
                    first_user["content"] = json!(parts);
                } else {
                    let existing = content.as_str().unwrap_or_default();
                    first_user["content"] = json!(format!("{system_text}\n\n{existing}"));
                }
            } else {
                non_system.insert(0, json!({ "role": "user", "content": system_text }));
            }
        }

        non_system
    }

    fn append_content_parts(parts: &mut Vec<Value>, content: &Value) {
        match content {
            Value::Null => {}
            Value::String(text) => parts.push(json!({
                "type": "text",
                "text": text,
            })),
            Value::Array(values) => {
                for value in values {
                    if let Some(text) = value.as_str() {
                        parts.push(json!({
                            "type": "text",
                            "text": text,
                        }));
                    } else {
                        parts.push(value.clone());
                    }
                }
            }
            other => parts.push(other.clone()),
        }
    }

    fn merge_message_content(left: &Value, right: &Value) -> Value {
        match (left, right) {
            (Value::Null, other) => other.clone(),
            (other, Value::Null) => other.clone(),
            (Value::String(left_text), Value::String(right_text)) => {
                if left_text.is_empty() {
                    json!(right_text)
                } else if right_text.is_empty() {
                    json!(left_text)
                } else {
                    json!(format!("{left_text}\n\n{right_text}"))
                }
            }
            _ => {
                let mut parts = Vec::new();
                Self::append_content_parts(&mut parts, left);
                Self::append_content_parts(&mut parts, right);

                if parts.is_empty() {
                    Value::Null
                } else if parts.len() == 1 {
                    parts.into_iter().next().unwrap()
                } else {
                    Value::Array(parts)
                }
            }
        }
    }

    fn merge_message_into(target: &mut Value, source: Value) {
        let merged_content = Self::merge_message_content(
            target.get("content").unwrap_or(&Value::Null),
            source.get("content").unwrap_or(&Value::Null),
        );
        target["content"] = merged_content;

        let mut merged_tool_calls = target
            .get("tool_calls")
            .and_then(|tool_calls| tool_calls.as_array())
            .cloned()
            .unwrap_or_default();

        if let Some(tool_calls) = source
            .get("tool_calls")
            .and_then(|tool_calls| tool_calls.as_array())
        {
            merged_tool_calls.extend(tool_calls.iter().cloned());
        }

        if merged_tool_calls.is_empty() {
            if let Some(target_obj) = target.as_object_mut() {
                target_obj.remove("tool_calls");
            }
        } else {
            target["tool_calls"] = Value::Array(merged_tool_calls);
        }
    }

    fn merge_adjacent_messages(messages: Vec<Value>) -> Vec<Value> {
        let mut merged: Vec<Value> = Vec::with_capacity(messages.len());

        for message in messages {
            let role = message
                .get("role")
                .and_then(|role| role.as_str())
                .map(str::to_owned);

            if let (Some(prev), Some(role)) = (merged.last_mut(), role)
                && prev.get("role").and_then(|prev_role| prev_role.as_str()) == Some(role.as_str())
            {
                Self::merge_message_into(prev, message);
                continue;
            }

            merged.push(message);
        }

        merged
    }

    fn summarize_request_messages(request: &Value) -> String {
        request
            .get("messages")
            .and_then(|messages| messages.as_array())
            .map(|messages| {
                messages
                    .iter()
                    .map(|message| {
                        let role = message
                            .get("role")
                            .and_then(|role| role.as_str())
                            .unwrap_or("unknown");
                        let tool_count = message
                            .get("tool_calls")
                            .and_then(|tool_calls| tool_calls.as_array())
                            .map(|tool_calls| tool_calls.len())
                            .unwrap_or(0);
                        let content_kind = match message.get("content") {
                            Some(Value::String(_)) => Some("text"),
                            Some(Value::Array(parts)) => Some(if parts.is_empty() {
                                "empty-parts"
                            } else {
                                "parts"
                            }),
                            Some(Value::Null) => Some("null"),
                            Some(_) => Some("other"),
                            None => None,
                        };

                        match (tool_count, content_kind) {
                            (count, Some(kind)) if count > 0 => {
                                format!("{role}[{kind},tools={count}]")
                            }
                            (0, Some(kind)) => format!("{role}[{kind}]"),
                            _ => role.to_string(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" -> ")
            })
            .unwrap_or_default()
    }

    fn normalize_non_tool_messages(messages: Vec<Value>) -> Vec<Value> {
        // Many local models (e.g. Gemma 3 via LM Studio) reject system messages
        // entirely. Merge all system content into the first user message and
        // guarantee strict user/assistant alternation.
        let non_system = Self::merge_system_into_first_user(messages);

        let mut normalized_messages: Vec<Value> = Vec::new();
        let mut last_role: Option<String> = None;

        for message in &non_system {
            if let Some(role) = message.get("role").and_then(|r| r.as_str()) {
                if let Some(ref last) = last_role
                    && last == role
                {
                    let placeholder_role = if role == "user" { "assistant" } else { "user" };
                    normalized_messages.push(json!({
                        "role": placeholder_role,
                        "content": "[Placeholder message for proper alternation]",
                    }));
                }
                normalized_messages.push(message.clone());
                last_role = Some(role.to_string());
            } else {
                normalized_messages.push(message.clone());
            }
        }

        if normalized_messages
            .first()
            .and_then(|m| m.get("role"))
            .and_then(|r| r.as_str())
            == Some("assistant")
        {
            normalized_messages.insert(
                0,
                json!({
                    "role": "user",
                    "content": "[Start of conversation]",
                }),
            );
        }

        normalized_messages
    }

    fn create_completion_request(
        &self,
        completion_request: CompletionRequest,
    ) -> Result<Value, CompletionError> {
        let mut messages = Vec::new();

        if let Some(preamble) = &completion_request.preamble {
            messages.push(json!({
                "role": "system",
                "content": preamble,
            }));
        }

        if !completion_request.documents.is_empty() {
            let doc_content = completion_request
                .documents
                .iter()
                .map(|d| d.text.clone())
                .collect::<Vec<_>>()
                .join("\n\n");
            messages.push(json!({
                "role": "system",
                "content": format!("Context documents:\n{}", doc_content),
            }));
        }

        for msg in completion_request.chat_history.iter() {
            let converted = self.convert_message(msg.clone())?;
            if let Some(msgs) = converted.as_array() {
                messages.extend(msgs.iter().cloned());
            } else {
                messages.push(converted);
            }
        }
        let messages = if Self::should_preserve_canonical_messages(&messages) {
            Self::merge_adjacent_messages(Self::merge_system_into_first_user(messages))
        } else {
            Self::normalize_non_tool_messages(messages)
        };
        let temperature = completion_request.temperature.unwrap_or(0.7);

        let mut request_payload = json!({
            "model": self.model,
            "messages": messages,
            "temperature": temperature,
            "stream": false,
        });

        if let Some(max_tokens) = completion_request.max_tokens {
            request_payload["max_tokens"] = json!(max_tokens);
        }

        if !completion_request.tools.is_empty() {
            request_payload["tools"] = json!(
                completion_request
                    .tools
                    .into_iter()
                    .map(|tool| json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        }
                    }))
                    .collect::<Vec<_>>()
            );

            if let Some(tool_choice) = completion_request.tool_choice.as_ref() {
                request_payload["tool_choice"] = Self::tool_choice_value(tool_choice);
            }
        }

        if let Some(extra) = completion_request.additional_params
            && let Some(obj) = request_payload.as_object_mut()
            && let Some(extra_obj) = extra.as_object()
        {
            for (k, v) in extra_obj {
                obj.insert(k.clone(), v.clone());
            }
        }

        Ok(request_payload)
    }

    fn merge_media_additional_params(target: &mut Value, additional_params: Option<&Value>) {
        let Some(additional_params) = additional_params else {
            return;
        };
        let Some(target) = target.as_object_mut() else {
            return;
        };

        if let Some(additional_params) = additional_params.as_object() {
            for (key, value) in additional_params {
                target.entry(key.clone()).or_insert_with(|| value.clone());
            }
        } else {
            target
                .entry("additional_params".to_string())
                .or_insert_with(|| additional_params.clone());
        }
    }

    fn invalid_media_source(message: impl Into<String>) -> CompletionError {
        CompletionError::RequestError(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            message.into(),
        )))
    }

    fn image_url_payload(image: &message::Image) -> Result<Value, CompletionError> {
        let mime = image
            .media_type
            .as_ref()
            .map(|media_type| media_type.to_mime_type());
        let url = match &image.data {
            message::DocumentSourceKind::Url(url) => url.clone(),
            message::DocumentSourceKind::Base64(data) => mime
                .map(|mime| format!("data:{mime};base64,{data}"))
                .unwrap_or_else(|| data.clone()),
            message::DocumentSourceKind::Raw(bytes) => {
                let data = BASE64_STANDARD.encode(bytes);
                mime.map(|mime| format!("data:{mime};base64,{data}"))
                    .unwrap_or(data)
            }
            message::DocumentSourceKind::String(_) => {
                return Err(Self::invalid_media_source(
                    "llama.cpp image input cannot use a literal string source; use Url for a URL/data URI or Base64/Raw for image bytes",
                ));
            }
            message::DocumentSourceKind::FileId(_) => {
                return Err(Self::invalid_media_source(
                    "llama.cpp image input does not accept provider file IDs; resolve the file ID to a URL or bytes first",
                ));
            }
            message::DocumentSourceKind::Unknown => {
                return Err(Self::invalid_media_source(
                    "llama.cpp image input has no source data",
                ));
            }
            _ => {
                return Err(Self::invalid_media_source(
                    "llama.cpp image input uses an unsupported source kind",
                ));
            }
        };

        let mut payload = json!({});
        Self::merge_media_additional_params(&mut payload, image.additional_params.as_ref());
        payload["url"] = json!(url);
        payload["detail"] = json!(
            image
                .detail
                .as_ref()
                .map(|detail| format!("{detail:?}").to_lowercase())
                .unwrap_or_else(|| "auto".to_string())
        );
        Ok(payload)
    }

    fn audio_input_payload(audio: &message::Audio) -> Result<Value, CompletionError> {
        let mut payload = json!({});
        Self::merge_media_additional_params(&mut payload, audio.additional_params.as_ref());

        match &audio.data {
            message::DocumentSourceKind::Url(url) => payload["url"] = json!(url),
            message::DocumentSourceKind::Base64(data) => payload["data"] = json!(data),
            message::DocumentSourceKind::Raw(bytes) => {
                payload["data"] = json!(BASE64_STANDARD.encode(bytes));
            }
            message::DocumentSourceKind::String(_) => {
                return Err(Self::invalid_media_source(
                    "llama.cpp audio input cannot use a literal string source; use Url for a URL/file path or Base64/Raw for audio bytes",
                ));
            }
            message::DocumentSourceKind::FileId(_) => {
                return Err(Self::invalid_media_source(
                    "llama.cpp audio input does not accept provider file IDs; resolve the file ID to a URL or bytes first",
                ));
            }
            message::DocumentSourceKind::Unknown => {
                return Err(Self::invalid_media_source(
                    "llama.cpp audio input has no source data",
                ));
            }
            _ => {
                return Err(Self::invalid_media_source(
                    "llama.cpp audio input uses an unsupported source kind",
                ));
            }
        }

        if let Some(media_type) = audio.media_type.as_ref() {
            let format = media_type
                .to_mime_type()
                .split_once('/')
                .map(|(_, format)| format)
                .unwrap_or_else(|| media_type.to_mime_type());
            payload["format"] = json!(format);
        }

        Ok(payload)
    }

    fn video_input_payload(video: &message::Video) -> Result<Value, CompletionError> {
        let mut payload = json!({});
        Self::merge_media_additional_params(&mut payload, video.additional_params.as_ref());

        match &video.data {
            message::DocumentSourceKind::Url(url) => payload["url"] = json!(url),
            message::DocumentSourceKind::Base64(data) => payload["data"] = json!(data),
            message::DocumentSourceKind::Raw(bytes) => {
                payload["data"] = json!(BASE64_STANDARD.encode(bytes));
            }
            message::DocumentSourceKind::String(_) => {
                return Err(Self::invalid_media_source(
                    "llama.cpp video input cannot use a literal string source; use Url for a URL/file path or Base64/Raw for video bytes",
                ));
            }
            message::DocumentSourceKind::FileId(_) => {
                return Err(Self::invalid_media_source(
                    "llama.cpp video input does not accept provider file IDs; resolve the file ID to a URL or bytes first",
                ));
            }
            message::DocumentSourceKind::Unknown => {
                return Err(Self::invalid_media_source(
                    "llama.cpp video input has no source data",
                ));
            }
            _ => {
                return Err(Self::invalid_media_source(
                    "llama.cpp video input uses an unsupported source kind",
                ));
            }
        }

        Ok(payload)
    }

    fn document_text_part(document: &message::Document) -> Result<Value, CompletionError> {
        let message::DocumentSourceKind::String(text) = &document.data else {
            return Err(Self::invalid_media_source(
                "llama.cpp chat completions do not accept document files, URLs, bytes, or file IDs; convert the document to literal text before invoking the model",
            ));
        };
        if matches!(
            document.media_type.as_ref(),
            Some(message::DocumentMediaType::PDF)
        ) {
            return Err(Self::invalid_media_source(
                "llama.cpp cannot losslessly send a PDF string as chat content; extract the PDF text first",
            ));
        }

        let mut part = json!({});
        Self::merge_media_additional_params(&mut part, document.additional_params.as_ref());
        part["type"] = json!("text");
        part["text"] = json!(text);
        Ok(part)
    }

    fn process_user_content(
        &self,
        content: &[&message::UserContent],
    ) -> Result<(Vec<Value>, Vec<Value>, bool), CompletionError> {
        let mut content_parts = Vec::new();
        let mut tool_results = Vec::new();
        let mut has_multimodal = false;

        for c in content.iter() {
            match c {
                message::UserContent::Text(t) => {
                    if has_multimodal || content.len() > 1 || t.additional_params.is_some() {
                        let mut text_part = json!({
                            "type": "text",
                            "text": t.text
                        });
                        Self::merge_media_additional_params(
                            &mut text_part,
                            t.additional_params.as_ref(),
                        );
                        content_parts.push(text_part);
                    } else {
                        content_parts.push(json!(t.text.clone()));
                    }
                }
                message::UserContent::Image(img) => {
                    has_multimodal = true;
                    content_parts.push(json!({
                        "type": "image_url",
                        "image_url": Self::image_url_payload(img)?,
                    }));
                }
                message::UserContent::Audio(audio) => {
                    has_multimodal = true;
                    content_parts.push(json!({
                        "type": "input_audio",
                        "input_audio": Self::audio_input_payload(audio)?,
                    }));
                }
                message::UserContent::Video(video) => {
                    has_multimodal = true;
                    content_parts.push(json!({
                        "type": "input_video",
                        "input_video": Self::video_input_payload(video)?,
                    }));
                }
                message::UserContent::Document(doc) => {
                    content_parts.push(Self::document_text_part(doc)?);
                }
                message::UserContent::ToolResult(tr) => {
                    let result_texts: Vec<String> = tr
                        .content
                        .iter()
                        .filter_map(|item| match item {
                            message::ToolResultContent::Text(t) => Some(t.text.clone()),
                            _ => None,
                        })
                        .collect();

                    let tool_call_id = tr.call_id.as_deref().unwrap_or(&tr.id);
                    let tool_result_text = if result_texts.is_empty() {
                        format!("Tool result for call {tool_call_id}: [no content]")
                    } else {
                        format!(
                            "Tool result for call {tool_call_id}:\n{}",
                            result_texts.join("\n")
                        )
                    };

                    if has_multimodal || content.len() > 1 {
                        tool_results.push(json!({
                            "type": "text",
                            "text": tool_result_text,
                        }));
                    } else {
                        tool_results.push(json!(tool_result_text));
                    }
                }
            }
        }

        Ok((content_parts, tool_results, has_multimodal))
    }

    fn build_user_message(
        &self,
        mut content_parts: Vec<Value>,
        tool_results: Vec<Value>,
        has_multimodal: bool,
    ) -> Result<Value, CompletionError> {
        if !tool_results.is_empty() {
            content_parts.extend(tool_results);
        }

        if has_multimodal {
            let mut normalized_parts = Vec::new();
            for part in content_parts {
                if let Some(text) = part.as_str() {
                    normalized_parts.push(json!({
                        "type": "text",
                        "text": text
                    }));
                } else {
                    normalized_parts.push(part);
                }
            }
            content_parts = normalized_parts;
        }

        let content_value = if content_parts.is_empty() {
            json!("[No content]")
        } else if content_parts.len() == 1 && !has_multimodal && content_parts[0].as_str().is_some()
        {
            content_parts.into_iter().next().unwrap()
        } else if !has_multimodal && content_parts.iter().all(|part| part.as_str().is_some()) {
            json!(
                content_parts
                    .iter()
                    .filter_map(|part| part.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n")
            )
        } else {
            json!(content_parts)
        };

        Ok(json!({
            "role": "user",
            "content": content_value,
        }))
    }

    fn convert_message(&self, msg: message::Message) -> Result<Value, CompletionError> {
        match msg {
            message::Message::User { content, .. } => {
                let (content_parts, tool_results, has_multimodal) =
                    self.process_user_content(content.iter().collect::<Vec<_>>().as_slice())?;
                self.build_user_message(content_parts, tool_results, has_multimodal)
            }
            message::Message::System { content, .. } => Ok(json!({
                "role": "system",
                "content": content,
            })),
            message::Message::Assistant { content, .. } => {
                let mut text_parts = Vec::new();
                let mut tool_calls = Vec::new();

                for c in content.iter() {
                    match c {
                        completion::AssistantContent::Text(t) => {
                            text_parts.push(t.text.clone());
                        }
                        completion::AssistantContent::ToolCall(tc) => {
                            tool_calls.push(json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.function.name,
                                    "arguments": serde_json::to_string(&tc.function.arguments).unwrap_or_default()
                                }
                            }));
                        }
                        _ => {}
                    }
                }

                let text = text_parts.join(" ");
                let mut message = json!({
                    "role": "assistant",
                });

                message["content"] = if text.is_empty() {
                    json!(null)
                } else {
                    json!(text)
                };

                if !tool_calls.is_empty() {
                    message["tool_calls"] = json!(tool_calls);
                }

                Ok(message)
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct StreamingCompletionResponse {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

impl GetTokenUsage for StreamingCompletionResponse {
    fn token_usage(&self) -> Option<Usage> {
        Some(Usage {
            input_tokens: self.prompt_tokens,
            output_tokens: self.completion_tokens,
            total_tokens: self.total_tokens,
            cache_creation_input_tokens: 0,
            cached_input_tokens: 0,
            tool_use_prompt_tokens: 0,
            reasoning_tokens: 0,
        })
    }
}

impl completion::CompletionModel for CompletionModel {
    type Response = CompletionResponse;
    type StreamingResponse = StreamingCompletionResponse;
    type Client = LlamaCppClient;

    fn make(client: &Self::Client, model: impl Into<String>) -> Self {
        Self::new(client.clone(), &model.into())
    }

    async fn completion(
        &self,
        completion_request: CompletionRequest,
    ) -> Result<completion::CompletionResponse<Self::Response>, CompletionError> {
        let tool_argument_specs = Self::tool_argument_specs(&completion_request.tools);
        let request = self.create_completion_request(completion_request)?;

        let response = self
            .client
            .post("v1/chat/completions")
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?
            .json(&request)
            .send()
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            eprintln!(
                "[LLAMACPP ERROR] completion failed: status={} sequence={} body={}",
                status,
                Self::summarize_request_messages(&request),
                body
            );
            return Err(CompletionError::ProviderError(body));
        }

        let bytes = response.bytes().await.map_err(|e| {
            CompletionError::ProviderError(format!("Failed to read response: {}", e))
        })?;

        let mut response_data: CompletionResponse = serde_json::from_slice(&bytes)
            .map_err(|e| CompletionError::ResponseError(e.to_string()))?;

        if !tool_argument_specs.is_empty()
            && let Some(choice) = response_data.choices.first_mut()
            && choice.message.tool_calls.is_empty()
            && let Some(content) = choice.message.content.as_deref()
        {
            let fallback_tool_calls =
                Self::fallback_tool_calls_from_text(content, &tool_argument_specs);
            if !fallback_tool_calls.is_empty() {
                choice.message.tool_calls = fallback_tool_calls;
                choice.message.content = None;
            }
        }

        response_data.try_into()
    }

    async fn stream(
        &self,
        completion_request: CompletionRequest,
    ) -> Result<streaming::StreamingCompletionResponse<Self::StreamingResponse>, CompletionError>
    {
        use flow_like_types::async_stream::stream;
        use flow_like_types::futures::StreamExt;
        use flow_like_types::reqwest_eventsource::{Event, RequestBuilderExt};
        use std::collections::BTreeMap;

        let tool_argument_specs = Self::tool_argument_specs(&completion_request.tools);

        let mut request = self.create_completion_request(completion_request)?;
        request["stream"] = json!(true);
        Self::enable_stream_usage(&mut request);
        let request_summary = Self::summarize_request_messages(&request);

        let builder = self
            .client
            .post("v1/chat/completions")
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?
            .json(&request);

        let mut event_source = builder.eventsource().map_err(|e| {
            eprintln!(
                "[LLAMACPP ERROR] stream failed: sequence={} error={}",
                request_summary, e
            );
            CompletionError::ProviderError(format!("Failed to create event source: {}", e))
        })?;

        let stream = Box::pin(stream! {
            let mut tool_calls: BTreeMap<usize, AccumulatedToolCall> = BTreeMap::new();
            let mut final_usage: Option<ApiUsage> = None;
            let mut streamed_text = String::new();
            let mut streamed_reasoning_preview = String::new();
            let mut buffered_text_chunks = Vec::new();
            let should_buffer_text = !tool_argument_specs.is_empty();

            while let Some(event_result) = event_source.next().await {
                match event_result {
                    Ok(Event::Open) => {
                        continue;
                    }
                    Ok(Event::Message(message)) => {
                        if message.data.trim().is_empty() || message.data == "[DONE]" {
                            continue;
                        }

                        let chunk: Result<StreamingChunk, _> = serde_json::from_str(&message.data);
                        let Ok(chunk) = chunk else {
                            continue;
                        };

                        if let Some(choice) = chunk.choices.first() {
                            let delta = &choice.delta;

                            if let Some(content) = &delta.content
                                && !content.is_empty() {
                                    streamed_text.push_str(content);
                                    if should_buffer_text {
                                        buffered_text_chunks.push(content.clone());
                                        let normalized_preview =
                                            Self::normalize_reasoning_preview(&streamed_text);

                                        if normalized_preview
                                            .starts_with(&streamed_reasoning_preview)
                                        {
                                            let reasoning_delta = normalized_preview
                                                [streamed_reasoning_preview.len()..]
                                                .to_string();
                                            streamed_reasoning_preview = normalized_preview;

                                            if !reasoning_delta.is_empty() {
                                                yield Ok(streaming::RawStreamingChoice::ReasoningDelta {
                                                    id: None,
                                                    reasoning: reasoning_delta,
                                                });
                                            }
                                        } else {
                                            streamed_reasoning_preview = normalized_preview;
                                        }
                                    } else {
                                        yield Ok(streaming::RawStreamingChoice::Message(content.clone()));
                                    }
                                }

                            if !delta.tool_calls.is_empty() {
                                for tool_call in &delta.tool_calls {
                                    let function = &tool_call.function;

                                    let entry = tool_calls.entry(tool_call.index).or_default();

                                    if let Some(id) = &tool_call.id
                                        && !id.is_empty() {
                                            entry.id = id.clone();
                                        }

                                    if let Some(name) = &function.name
                                        && !name.is_empty() {
                                            entry.name.push_str(name);
                                        }

                                    if !function.arguments.is_empty() {
                                        entry.arguments.push_str(&function.arguments);
                                    }
                                }
                            }
                        }

                        if let Some(usage) = chunk.usage {
                            final_usage = Some(usage);
                        }
                    }
                    Err(e) => {
                        let error_str = e.to_string();
                        if error_str.contains("Stream ended") {
                            break;
                        }

                        yield Err(CompletionError::ProviderError(format!("Stream error: {}", e)));
                        break;
                    }
                }
            }

            let mut emitted_native_tool_call = false;

            for (_, tool_call) in tool_calls {
                if tool_call.name.is_empty() {
                    continue;
                }

                emitted_native_tool_call = true;

                let arguments = if tool_call.arguments.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&tool_call.arguments).unwrap_or_else(|_| json!({}))
                };

                yield Ok(streaming::RawStreamingChoice::ToolCall(
                    streaming::RawStreamingToolCall::new(tool_call.id, tool_call.name, arguments)
                ));
            }

            if !emitted_native_tool_call {
                let fallback_tool_calls =
                    Self::fallback_tool_calls_from_text(&streamed_text, &tool_argument_specs);
                if fallback_tool_calls.is_empty() {
                    for content in buffered_text_chunks {
                        yield Ok(streaming::RawStreamingChoice::Message(content));
                    }
                } else {
                    for tool_call in fallback_tool_calls {
                        let arguments = serde_json::from_str(&tool_call.function.arguments)
                            .unwrap_or_else(|_| json!({}));
                        yield Ok(streaming::RawStreamingChoice::ToolCall(
                            streaming::RawStreamingToolCall::new(
                                tool_call.id,
                                tool_call.function.name,
                                arguments,
                            )
                        ));
                    }
                }
            }

            if let Some(usage) = final_usage {
                yield Ok(streaming::RawStreamingChoice::FinalResponse(
                    StreamingCompletionResponse {
                        prompt_tokens: usage.prompt_tokens,
                        completion_tokens: usage.completion_tokens,
                        total_tokens: usage.total_tokens,
                    }
                ));
            }

            event_source.close();
        });

        Ok(streaming::StreamingCompletionResponse::stream(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::tokio;
    use rig::client::CompletionClient;
    use rig::completion::{Chat, CompletionModel as _, Message};
    use rig::message::ToolChoice;
    use rig::streaming::StreamingChat;

    const DEFAULT_BASE_URL: &str = "http://localhost:8080";

    fn test_base_url() -> String {
        std::env::var("LLAMACPP_TEST_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
    }

    fn test_model() -> String {
        std::env::var("LLAMACPP_TEST_MODEL").unwrap_or_else(|_| "test".to_string())
    }

    #[test]
    fn test_tool_choice_value_matches_openai_payload() {
        assert_eq!(
            CompletionModel::tool_choice_value(&ToolChoice::Auto),
            json!("auto")
        );
        assert_eq!(
            CompletionModel::tool_choice_value(&ToolChoice::None),
            json!("none")
        );
        assert_eq!(
            CompletionModel::tool_choice_value(&ToolChoice::Required),
            json!("required")
        );
        assert_eq!(
            CompletionModel::tool_choice_value(&ToolChoice::Specific {
                function_names: vec!["lookup_weather".to_string()],
            }),
            json!({
                "type": "function",
                "function": {
                    "name": "lookup_weather",
                }
            })
        );
    }

    #[test]
    fn test_enable_stream_usage_preserves_existing_stream_options() {
        let mut payload = json!({
            "stream_options": {
                "foo": "bar"
            }
        });

        CompletionModel::enable_stream_usage(&mut payload);

        assert_eq!(
            payload,
            json!({
                "stream_options": {
                    "foo": "bar",
                    "include_usage": true,
                }
            })
        );
    }

    #[test]
    fn test_convert_message_uses_llamacpp_multimodal_protocol_shapes() {
        let model = CompletionModel::new(LlamaCppClient::new(DEFAULT_BASE_URL), &test_model());
        let content = OneOrMany::many(vec![
            message::UserContent::Image(message::Image {
                data: message::DocumentSourceKind::Base64("aW1hZ2U=".to_string()),
                media_type: Some(message::ImageMediaType::PNG),
                detail: Some(message::ImageDetail::High),
                additional_params: Some(json!({ "cache_control": "ephemeral" })),
            }),
            message::UserContent::Audio(message::Audio {
                data: message::DocumentSourceKind::Url("https://example.com/input.wav".to_string()),
                media_type: Some(message::AudioMediaType::WAV),
                additional_params: Some(json!({ "transcription": "enabled" })),
            }),
            message::UserContent::Video(message::Video {
                data: message::DocumentSourceKind::Raw(b"video".to_vec()),
                media_type: Some(message::VideoMediaType::MP4),
                additional_params: Some(json!({ "fps": 24 })),
            }),
            message::UserContent::Document(message::Document {
                data: message::DocumentSourceKind::String("literal document text".to_string()),
                media_type: Some(message::DocumentMediaType::TXT),
                additional_params: Some(json!({ "filename": "notes.txt" })),
            }),
        ])
        .unwrap();

        let converted = model
            .convert_message(message::Message::User { content })
            .unwrap();
        let parts = converted["content"].as_array().unwrap();

        assert_eq!(
            parts[0],
            json!({
                "type": "image_url",
                "image_url": {
                    "url": "data:image/png;base64,aW1hZ2U=",
                    "detail": "high",
                    "cache_control": "ephemeral",
                }
            })
        );
        assert_eq!(
            parts[1],
            json!({
                "type": "input_audio",
                "input_audio": {
                    "url": "https://example.com/input.wav",
                    "format": "wav",
                    "transcription": "enabled",
                }
            })
        );
        assert_eq!(
            parts[2],
            json!({
                "type": "input_video",
                "input_video": {
                    "data": "dmlkZW8=",
                    "fps": 24,
                }
            })
        );
        assert_eq!(
            parts[3],
            json!({
                "type": "text",
                "text": "literal document text",
                "filename": "notes.txt",
            })
        );
    }

    #[test]
    fn test_convert_message_rejects_llamacpp_file_ids_with_actionable_error() {
        let model = CompletionModel::new(LlamaCppClient::new(DEFAULT_BASE_URL), &test_model());
        let content = OneOrMany::one(message::UserContent::Image(message::Image {
            data: message::DocumentSourceKind::FileId("image-123".to_string()),
            media_type: Some(message::ImageMediaType::PNG),
            detail: None,
            additional_params: None,
        }));

        let error = model
            .convert_message(message::Message::User { content })
            .unwrap_err()
            .to_string();

        assert!(error.contains("does not accept provider file IDs"));
        assert!(error.contains("resolve the file ID to a URL or bytes first"));
    }

    #[test]
    fn test_convert_message_rejects_document_files_before_request() {
        let model = CompletionModel::new(LlamaCppClient::new(DEFAULT_BASE_URL), &test_model());
        let content = OneOrMany::one(message::UserContent::Document(message::Document {
            data: message::DocumentSourceKind::Url("https://example.com/document.pdf".to_string()),
            media_type: Some(message::DocumentMediaType::PDF),
            additional_params: None,
        }));

        let error = model
            .convert_message(message::Message::User { content })
            .unwrap_err()
            .to_string();

        assert!(error.contains("do not accept document files, URLs, bytes, or file IDs"));
        assert!(error.contains("convert the document to literal text"));
    }

    #[test]
    fn test_fallback_tool_calls_from_text_parses_python_style_call() {
        let tool_argument_specs =
            CompletionModel::tool_argument_specs(&[completion::ToolDefinition {
                name: "internet_search".to_string(),
                description: "Search the internet".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" }
                    },
                    "required": ["query"]
                }),
            }]);
        let tool_calls = CompletionModel::fallback_tool_calls_from_text(
            "internet_search(query=\"current oil price\")",
            &tool_argument_specs,
        );

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "internet_search");
        assert_eq!(
            serde_json::from_str::<Value>(&tool_calls[0].function.arguments).unwrap(),
            json!({ "query": "current oil price" })
        );
    }

    #[test]
    fn test_fallback_tool_calls_from_text_parses_code_fenced_call() {
        let tool_argument_specs =
            CompletionModel::tool_argument_specs(&[completion::ToolDefinition {
                name: "internet_search".to_string(),
                description: "Search the internet".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" }
                    },
                    "required": ["query"]
                }),
            }]);
        let tool_calls = CompletionModel::fallback_tool_calls_from_text(
            "```python\ninternet_search(query='current oil price')\n```",
            &tool_argument_specs,
        );

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(
            serde_json::from_str::<Value>(&tool_calls[0].function.arguments).unwrap(),
            json!({ "query": "current oil price" })
        );
    }

    #[test]
    fn test_fallback_tool_calls_from_text_parses_bare_command_style_call() {
        let tool_argument_specs =
            CompletionModel::tool_argument_specs(&[completion::ToolDefinition {
                name: "internet_search".to_string(),
                description: "Search the internet".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" }
                    },
                    "required": ["query"]
                }),
            }]);
        let tool_calls = CompletionModel::fallback_tool_calls_from_text(
            "internet_search query=\"current oil price\"",
            &tool_argument_specs,
        );

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(
            serde_json::from_str::<Value>(&tool_calls[0].function.arguments).unwrap(),
            json!({ "query": "current oil price" })
        );
    }

    #[test]
    fn test_fallback_tool_calls_from_text_parses_bare_command_style_multiple_args() {
        let tool_argument_specs =
            CompletionModel::tool_argument_specs(&[completion::ToolDefinition {
                name: "lookup_weather".to_string(),
                description: "Look up the weather".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "location": { "type": "string" },
                        "units": { "type": "string" },
                        "days": { "type": "integer" }
                    },
                    "required": ["location"]
                }),
            }]);
        let tool_calls = CompletionModel::fallback_tool_calls_from_text(
            "lookup_weather location=\"Berlin\" units=\"celsius\" days=3",
            &tool_argument_specs,
        );

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(
            serde_json::from_str::<Value>(&tool_calls[0].function.arguments).unwrap(),
            json!({
                "location": "Berlin",
                "units": "celsius",
                "days": 3,
            })
        );
    }

    #[test]
    fn test_fallback_tool_calls_from_text_parses_explanatory_preamble() {
        let tool_argument_specs =
            CompletionModel::tool_argument_specs(&[completion::ToolDefinition {
                name: "internet_search".to_string(),
                description: "Search the internet".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" }
                    },
                    "required": ["query"]
                }),
            }]);
        let tool_calls = CompletionModel::fallback_tool_calls_from_text(
            "Okay, I can help you with that. I'll start by searching for the current oil price.\n\n```tool_code\ninternet_search(query=\"current oil price\")\n```",
            &tool_argument_specs,
        );

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "internet_search");
        assert_eq!(
            serde_json::from_str::<Value>(&tool_calls[0].function.arguments).unwrap(),
            json!({ "query": "current oil price" })
        );
    }

    #[test]
    fn test_fallback_tool_calls_from_text_ignores_unknown_tools() {
        let tool_argument_specs =
            CompletionModel::tool_argument_specs(&[completion::ToolDefinition {
                name: "lookup_weather".to_string(),
                description: "Look up the weather".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "location": { "type": "string" }
                    },
                    "required": ["location"]
                }),
            }]);
        let tool_calls = CompletionModel::fallback_tool_calls_from_text(
            "internet_search(query=\"current oil price\")",
            &tool_argument_specs,
        );

        assert!(tool_calls.is_empty());
    }

    #[test]
    fn test_fallback_tool_calls_from_text_parses_jsonish_values() {
        let tool_argument_specs =
            CompletionModel::tool_argument_specs(&[completion::ToolDefinition {
                name: "lookup_weather".to_string(),
                description: "Look up the weather".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "location": { "type": "string" },
                        "options": { "type": "object" }
                    },
                    "required": ["location"]
                }),
            }]);
        let tool_calls = CompletionModel::fallback_tool_calls_from_text(
            "lookup_weather(location='Berlin', options={'units': 'celsius', 'days': 3, 'exact': True})",
            &tool_argument_specs,
        );

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(
            serde_json::from_str::<Value>(&tool_calls[0].function.arguments).unwrap(),
            json!({
                "location": "Berlin",
                "options": {
                    "units": "celsius",
                    "days": 3,
                    "exact": true,
                }
            })
        );
    }

    #[test]
    fn test_fallback_tool_calls_from_text_parses_positional_and_named_args_using_schema_order() {
        let tool_argument_specs =
            CompletionModel::tool_argument_specs(&[completion::ToolDefinition {
                name: "internet_search".to_string(),
                description: "Search the internet".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "lang": { "type": "string" },
                        "page": { "type": "integer" }
                    },
                    "required": ["query"]
                }),
            }]);

        let tool_calls = CompletionModel::fallback_tool_calls_from_text(
            "internet_search(\"today oil price\", lang=\"en\", page=1)",
            &tool_argument_specs,
        );

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(
            serde_json::from_str::<Value>(&tool_calls[0].function.arguments).unwrap(),
            json!({
                "query": "today oil price",
                "lang": "en",
                "page": 1,
            })
        );
    }

    #[test]
    fn test_normalize_reasoning_preview_preserves_streamed_newlines() {
        assert_eq!(
            CompletionModel::normalize_reasoning_preview(
                "Okay\n\ni\ncan\nhelp\nyou\nwith\nthat.\nHere\n's\na\nsearch",
            ),
            "Okay\n\ni\ncan\nhelp\nyou\nwith\nthat.\nHere\n's\na\nsearch"
        );
    }

    #[test]
    fn test_normalize_reasoning_preview_preserves_structured_markdown() {
        assert_eq!(
            CompletionModel::normalize_reasoning_preview(
                "** Summary**\n- First item\n- Second item\n\n```md\n# Heading\n```"
            ),
            "** Summary**\n- First item\n- Second item\n\n```md\n# Heading\n```"
        );
    }

    #[test]
    fn test_create_completion_request_merges_system_for_non_tool_chat() {
        let model = CompletionModel::new(LlamaCppClient::new(DEFAULT_BASE_URL), &test_model());
        let request = CompletionRequest {
            model: None,
            preamble: Some("You are concise.".to_string()),
            chat_history: OneOrMany::one(message::Message::user("Say hello")),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
            output_schema: None,
        };

        let payload = model.create_completion_request(request).unwrap();
        let messages = payload["messages"].as_array().unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], json!("user"));
        assert_eq!(
            messages[0]["content"],
            json!("You are concise.\n\nSay hello")
        );
    }

    #[test]
    fn test_create_completion_request_merges_system_for_initial_tool_turn() {
        let model = CompletionModel::new(LlamaCppClient::new(DEFAULT_BASE_URL), &test_model());
        let request = CompletionRequest {
            model: None,
            preamble: Some("You are concise.".to_string()),
            chat_history: OneOrMany::one(message::Message::user("Look up the weather")),
            documents: Vec::new(),
            tools: vec![completion::ToolDefinition {
                name: "lookup_weather".to_string(),
                description: "Look up the weather".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "location": { "type": "string" }
                    },
                    "required": ["location"]
                }),
            }],
            temperature: None,
            max_tokens: None,
            tool_choice: Some(message::ToolChoice::Auto),
            additional_params: None,
            output_schema: None,
        };

        let payload = model.create_completion_request(request).unwrap();
        let messages = payload["messages"].as_array().unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], json!("user"));
        assert_eq!(
            messages[0]["content"],
            json!("You are concise.\n\nLook up the weather")
        );
    }

    #[test]
    fn test_create_completion_request_preserves_canonical_tool_messages() {
        let model = CompletionModel::new(LlamaCppClient::new(DEFAULT_BASE_URL), &test_model());
        let assistant_tool_call = message::Message::Assistant {
            id: None,
            content: OneOrMany::one(completion::AssistantContent::tool_call(
                "call_weather",
                "lookup_weather",
                json!({ "location": "Berlin" }),
            )),
        };
        let request = CompletionRequest {
            model: None,
            preamble: Some("You are a weather assistant.".to_string()),
            chat_history: OneOrMany::many(vec![
                message::Message::user("What is the weather in Berlin?"),
                assistant_tool_call,
                message::Message::tool_result("call_weather", "15C and sunny"),
            ])
            .unwrap(),
            documents: Vec::new(),
            tools: vec![completion::ToolDefinition {
                name: "lookup_weather".to_string(),
                description: "Look up the weather".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "location": { "type": "string" }
                    },
                    "required": ["location"]
                }),
            }],
            temperature: None,
            max_tokens: None,
            tool_choice: Some(message::ToolChoice::Auto),
            additional_params: None,
            output_schema: None,
        };

        let payload = model.create_completion_request(request).unwrap();
        let messages = payload["messages"].as_array().unwrap();

        assert_eq!(
            messages,
            &vec![
                json!({
                    "role": "user",
                    "content": "You are a weather assistant.\n\nWhat is the weather in Berlin?",
                }),
                json!({
                    "role": "assistant",
                    "content": Value::Null,
                    "tool_calls": [{
                        "id": "call_weather",
                        "type": "function",
                        "function": {
                            "name": "lookup_weather",
                            "arguments": "{\"location\":\"Berlin\"}",
                        }
                    }]
                }),
                json!({
                    "role": "user",
                    "content": "Tool result for call call_weather:\n15C and sunny",
                }),
            ]
        );
        assert!(!messages.iter().any(|message| {
            message.get("content") == Some(&json!("[Placeholder message for proper alternation]"))
        }));
    }

    #[test]
    fn test_create_completion_request_merges_tool_result_with_followup_prompt() {
        let model = CompletionModel::new(LlamaCppClient::new(DEFAULT_BASE_URL), &test_model());
        let assistant_tool_call = message::Message::Assistant {
            id: None,
            content: OneOrMany::one(completion::AssistantContent::tool_call(
                "call_weather",
                "lookup_weather",
                json!({ "location": "Berlin" }),
            )),
        };
        let request = CompletionRequest {
            model: None,
            preamble: Some("You are a weather assistant.".to_string()),
            chat_history: OneOrMany::many(vec![
                message::Message::user("What is the weather in Berlin?"),
                assistant_tool_call,
                message::Message::tool_result("call_weather", "15C and sunny"),
                message::Message::user("Summarize it in one sentence."),
            ])
            .unwrap(),
            documents: Vec::new(),
            tools: vec![completion::ToolDefinition {
                name: "lookup_weather".to_string(),
                description: "Look up the weather".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "location": { "type": "string" }
                    },
                    "required": ["location"]
                }),
            }],
            temperature: None,
            max_tokens: None,
            tool_choice: Some(message::ToolChoice::Auto),
            additional_params: None,
            output_schema: None,
        };

        let payload = model.create_completion_request(request).unwrap();
        let messages = payload["messages"].as_array().unwrap();

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], json!("user"));
        assert_eq!(messages[1]["role"], json!("assistant"));
        assert_eq!(messages[2]["role"], json!("user"));
        assert_eq!(
            messages[2]["content"],
            json!(
                "Tool result for call call_weather:\n15C and sunny\n\nSummarize it in one sentence."
            )
        );
    }

    async fn server_available() -> bool {
        let url = format!("{}/health", test_base_url());
        reqwest::get(&url)
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    #[tokio::test]
    #[ignore = "requires a running llama-server"]
    async fn test_basic_completion() {
        if !server_available().await {
            eprintln!("Skipping: llama-server not running at {}", test_base_url());
            return;
        }

        let client = LlamaCppClient::new(&test_base_url());
        let agent = client.agent(&test_model()).build();

        let mut history = Vec::<Message>::new();
        let response: String = agent
            .chat("Say hello in exactly 3 words.", &mut history)
            .await
            .unwrap();
        assert!(!response.is_empty(), "Expected non-empty response");
    }

    #[tokio::test]
    #[ignore = "requires a running llama-server"]
    async fn test_completion_with_system_prompt() {
        if !server_available().await {
            return;
        }

        let client = LlamaCppClient::new(&test_base_url());
        let agent = client
            .agent(&test_model())
            .preamble("You are a pirate. Always respond in pirate speak.")
            .build();

        let mut history = Vec::<Message>::new();
        let response: String = agent
            .chat("What is your name?", &mut history)
            .await
            .unwrap();
        assert!(!response.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires a running llama-server"]
    async fn test_completion_with_chat_history() {
        if !server_available().await {
            return;
        }

        let client = LlamaCppClient::new(&test_base_url());
        let agent = client.agent(&test_model()).build();

        let mut history = vec![
            Message::user("My name is Alice."),
            Message::assistant("Nice to meet you, Alice!"),
        ];

        let response: String = agent.chat("What is my name?", &mut history).await.unwrap();
        assert!(!response.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires a running llama-server"]
    async fn test_streaming_completion() {
        use futures::StreamExt;

        if !server_available().await {
            return;
        }

        let client = LlamaCppClient::new(&test_base_url());
        let agent = client.agent(&test_model()).build();

        let mut stream = agent
            .stream_chat("Count from 1 to 5.", Vec::<Message>::new())
            .await;

        let mut collected = String::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(rig::agent::MultiTurnStreamItem::StreamAssistantItem(content)) => {
                    match content {
                        rig::streaming::StreamedAssistantContent::Text(text) => {
                            collected.push_str(&text.text);
                        }
                        _ => {}
                    }
                }
                Ok(_) => {}
                Err(e) => panic!("Stream error: {}", e),
            }
        }

        assert!(!collected.is_empty(), "Expected streamed content");
    }

    #[tokio::test]
    #[ignore = "requires a running llama-server"]
    async fn test_raw_completion_request() {
        if !server_available().await {
            return;
        }

        let client = LlamaCppClient::new(&test_base_url());
        let model = client.completion_model(&test_model());

        let response = model
            .completion_request("What is 2+2?")
            .send()
            .await
            .unwrap();

        let text = response
            .choice
            .iter()
            .filter_map(|c| match c {
                completion::AssistantContent::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        assert!(!text.is_empty(), "Expected text in completion response");
    }

    #[tokio::test]
    #[ignore = "requires a running llama-server"]
    async fn test_completion_with_max_tokens() {
        if !server_available().await {
            return;
        }

        let client = LlamaCppClient::new(&test_base_url());
        let model = client.completion_model(&test_model());

        let response = model
            .completion_request("Write a very short story about a cat in 2 sentences.")
            .max_tokens(1000)
            .send()
            .await
            .unwrap();

        assert!(
            response.usage.output_tokens <= 1010,
            "Expected max_tokens to be respected, got {}",
            response.usage.output_tokens
        );
    }

    #[tokio::test]
    #[ignore = "requires a running llama-server"]
    async fn test_completion_with_temperature() {
        if !server_available().await {
            return;
        }

        let client = LlamaCppClient::new(&test_base_url());
        let model = client.completion_model(&test_model());

        let response = model
            .completion_request("Say hi.")
            .temperature(0.0)
            .send()
            .await
            .unwrap();

        let text = response
            .choice
            .iter()
            .filter_map(|c| match c {
                completion::AssistantContent::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        assert!(!text.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires a running llama-server"]
    async fn test_usage_tracking() {
        if !server_available().await {
            return;
        }

        let client = LlamaCppClient::new(&test_base_url());
        let model = client.completion_model(&test_model());

        let response = model.completion_request("Hello").send().await.unwrap();

        assert!(response.usage.input_tokens > 0, "Expected input tokens > 0");
        assert!(
            response.usage.output_tokens > 0,
            "Expected output tokens > 0"
        );
        assert!(
            response.usage.total_tokens
                >= response.usage.input_tokens + response.usage.output_tokens,
            "Total should be >= input + output"
        );
    }

    #[tokio::test]
    #[ignore = "requires a running llama-server"]
    async fn test_responses_api() {
        let base_url = test_base_url();
        let url = format!("{}/v1/responses", base_url);

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .json(&json!({
                "model": test_model(),
                "input": "Say hello in exactly 3 words.",
                "max_output_tokens": 20,
            }))
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let body: Value = r.json().await.unwrap();
                assert!(
                    body.get("output").is_some(),
                    "Expected 'output' field in responses API"
                );
            }
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                if status.as_u16() == 404 {
                    eprintln!("Responses API not available (404) — server may be older version");
                } else {
                    panic!("Unexpected status {}: {}", status, body);
                }
            }
            Err(e) => {
                eprintln!("Skipping responses API test: {}", e);
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires a running llama-server"]
    async fn test_responses_api_with_instructions() {
        let base_url = test_base_url();
        let url = format!("{}/v1/responses", base_url);

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .json(&json!({
                "model": test_model(),
                "instructions": "You are a pirate. Always respond in pirate speak.",
                "input": "What is the weather like?",
                "max_output_tokens": 50,
            }))
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let body: Value = r.json().await.unwrap();
                assert!(
                    body.get("output").is_some(),
                    "Expected 'output' in response"
                );
            }
            Ok(r) if r.status().as_u16() == 404 => {
                eprintln!("Responses API not available (404)");
            }
            Ok(r) => panic!(
                "Unexpected: {} {}",
                r.status(),
                r.text().await.unwrap_or_default()
            ),
            Err(e) => eprintln!("Skipping: {}", e),
        }
    }

    #[tokio::test]
    #[ignore = "requires a running llama-server"]
    async fn test_responses_api_streaming() {
        let base_url = test_base_url();
        let url = format!("{}/v1/responses", base_url);

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .json(&json!({
                "model": test_model(),
                "input": "Count from 1 to 3.",
                "stream": true,
            }))
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let body = r.text().await.unwrap_or_default();
                assert!(!body.is_empty(), "Expected streamed response body");
            }
            Ok(r) if r.status().as_u16() == 404 => {
                eprintln!("Responses API streaming not available (404)");
            }
            Ok(r) => panic!(
                "Unexpected: {} {}",
                r.status(),
                r.text().await.unwrap_or_default()
            ),
            Err(e) => eprintln!("Skipping: {}", e),
        }
    }

    #[tokio::test]
    #[ignore = "requires a running llama-server"]
    async fn test_health_endpoint() {
        let base_url = test_base_url();
        let url = format!("{}/health", base_url);

        let resp = reqwest::get(&url).await;
        match resp {
            Ok(r) => assert!(r.status().is_success(), "Health check should succeed"),
            Err(e) => eprintln!("Server not available: {}", e),
        }
    }

    #[tokio::test]
    #[ignore = "requires a running llama-server"]
    async fn test_models_endpoint() {
        let base_url = test_base_url();
        let url = format!("{}/v1/models", base_url);

        let resp = reqwest::get(&url).await;
        match resp {
            Ok(r) if r.status().is_success() => {
                let body: Value = r.json().await.unwrap();
                let data = body.get("data").and_then(|d| d.as_array());
                assert!(data.is_some(), "Expected 'data' array");
                assert!(!data.unwrap().is_empty(), "Expected at least one model");
            }
            Ok(r) => panic!("Unexpected status: {}", r.status()),
            Err(e) => eprintln!("Skipping: {}", e),
        }
    }

    /// Minimal 64x64 red PNG encoded as base64 for vision tests.
    fn test_red_png_base64() -> &'static str {
        "iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAIAAAAlC+aJAAAAb0lEQVR4nO3PAQkAAAyEwO9feoshgnABdLep8QUNyPEFDcjxBQ3I8QUNyPEFDcjxBQ3I8QUNyPEFDcjxBQ3I8QUNyPEFDcjxBQ3I8QUNyPEFDcjxBQ3I8QUNyPEFDcjxBQ3I8QUNyPEFDcjxBQ3IPanc8OLDQitxAAAAAElFTkSuQmCC"
    }

    #[tokio::test]
    #[ignore = "requires a running llama-server with --mmproj"]
    async fn test_vision_completion() {
        if !server_available().await {
            eprintln!("Skipping: llama-server not running at {}", test_base_url());
            return;
        }

        let client = LlamaCppClient::new(&test_base_url());
        let model = client.completion_model(&test_model());

        let request = model
            .completion_request(Message::User {
                content: OneOrMany::many(vec![
                    message::UserContent::text("What color is this image? Answer in one word."),
                    message::UserContent::image_base64(
                        test_red_png_base64(),
                        Some(message::ImageMediaType::PNG),
                        None,
                    ),
                ])
                .unwrap(),
            })
            .max_tokens(100)
            .send()
            .await
            .unwrap();

        let text = request
            .choice
            .iter()
            .filter_map(|c| match c {
                completion::AssistantContent::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        assert!(!text.is_empty(), "Expected a response describing the image");
        eprintln!("Vision response: {}", text);
    }

    #[tokio::test]
    #[ignore = "requires a running llama-server with --mmproj"]
    async fn test_vision_chat() {
        if !server_available().await {
            return;
        }

        let client = LlamaCppClient::new(&test_base_url());
        let agent = client.agent(&test_model()).build();

        let mut history = Vec::<Message>::new();
        let response: String = agent
            .chat(
                Message::User {
                    content: OneOrMany::many(vec![
                        message::UserContent::text("Describe this image in one short sentence."),
                        message::UserContent::image_base64(
                            test_red_png_base64(),
                            Some(message::ImageMediaType::PNG),
                            None,
                        ),
                    ])
                    .unwrap(),
                },
                &mut history,
            )
            .await
            .unwrap();

        assert!(!response.is_empty(), "Expected a vision chat response");
        eprintln!("Vision chat response: {}", response);
    }

    #[tokio::test]
    #[ignore = "requires a running llama-server with --mmproj"]
    async fn test_vision_streaming() {
        use futures::StreamExt;

        if !server_available().await {
            return;
        }

        let client = LlamaCppClient::new(&test_base_url());
        let agent = client.agent(&test_model()).build();

        let mut stream = agent
            .stream_chat(
                Message::User {
                    content: OneOrMany::many(vec![
                        message::UserContent::text("What do you see?"),
                        message::UserContent::image_base64(
                            test_red_png_base64(),
                            Some(message::ImageMediaType::PNG),
                            None,
                        ),
                    ])
                    .unwrap(),
                },
                Vec::<Message>::new(),
            )
            .await;

        let mut collected = String::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(rig::agent::MultiTurnStreamItem::StreamAssistantItem(content)) => {
                    if let rig::streaming::StreamedAssistantContent::Text(text) = content {
                        collected.push_str(&text.text);
                    }
                }
                Ok(_) => {}
                Err(e) => panic!("Stream error: {}", e),
            }
        }

        assert!(!collected.is_empty(), "Expected streamed vision content");
        eprintln!("Vision streaming response: {}", collected);
    }
}
