//! Offline doubles for tests and the evaluation harness: a rule-based model, a deterministic
//! embedder, an in-memory history and a transcript builder.

use std::collections::{BTreeMap, HashMap};
use std::sync::{LazyLock, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use flow_like_types::{Result, anyhow};
use regex::Regex;
use serde_json::{Value, json};

use crate::llm::{Embedder, LlmRequest, LlmTask, MemoryLlm};
use crate::model::{HistoryItem, Role, Seq, SeqSpan, ToolPart};
use crate::pointer::PartRef;
use crate::summary::{Section, parse_summary_sections};
use crate::text::{normalize, truncate_chars};
use crate::tools::HistoryReader;

type Answer = Box<dyn Fn(&LlmRequest) -> Result<String> + Send + Sync>;

/// A deterministic stand-in for the model. By default it summarises extractively (one claim per
/// user message), extracts "my X is Y" facts, decides updates by attribute, and is fooled by
/// any tool result that says "remember …" (it proposes it as a user preference), so the trust
/// rules get exercised. Any task can be scripted with [`StubLlm::with`].
#[derive(Default)]
pub struct StubLlm {
    requests: Mutex<Vec<LlmRequest>>,
    answers: Mutex<HashMap<LlmTask, Answer>>,
}

static MESSAGE_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#(\d+) \[\S+ \S+ (\w+)\] (.*)$").expect("pattern"));
static PART_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s+#(\d+)\.((?:t|ui)\d+) \[(?:tool|ui) ([^\]]*)\]\s*(.*)$").expect("pattern")
});
static MY_X_IS_Y: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bmy ([a-z][a-z' ]{1,40}?) is ([^.,;!?]+)").expect("pattern")
});
static FACT_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[(f_[a-z0-9]+)\] (.*?) \(").expect("pattern"));
static ATTRIBUTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^the user's (.+?) is (.+?)\.?$").expect("pattern"));

/// A message line of a rendered span, or one of its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpanLine {
    Message {
        seq: Seq,
        role: String,
        text: String,
    },
    Part {
        seq: Seq,
        part: String,
        header: String,
        text: String,
    },
}

/// The lines of the block that starts after `heading` and ends at the first blank line.
pub fn span_lines(prompt: &str, heading: &str) -> Vec<SpanLine> {
    let Some(start) = prompt.find(heading) else {
        return Vec::new();
    };
    prompt[start + heading.len()..]
        .lines()
        .skip_while(|line| line.trim().is_empty())
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| {
            if let Some(caps) = MESSAGE_LINE.captures(line) {
                return Some(SpanLine::Message {
                    seq: caps[1].parse().ok()?,
                    role: caps[2].to_string(),
                    text: caps[3].to_string(),
                });
            }
            let caps = PART_LINE.captures(line)?;
            Some(SpanLine::Part {
                seq: caps[1].parse().ok()?,
                part: caps[2].to_string(),
                header: caps[3].to_string(),
                text: caps[4].to_string(),
            })
        })
        .collect()
}

fn claim(text: &str, seq: Seq) -> Value {
    json!({ "text": truncate_chars(text, 160), "seqs": [seq], "unverified": false })
}

fn summarise(request: &LlmRequest) -> Value {
    let mut decisions = Vec::new();
    let mut open_questions = Vec::new();
    let mut title = String::new();
    for line in span_lines(&request.prompt, "Messages:") {
        match line {
            SpanLine::Message { seq, role, text } if role == "user" => {
                if title.is_empty() {
                    title = text
                        .split_whitespace()
                        .take(6)
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                decisions.push(claim(&text, seq));
            }
            SpanLine::Part {
                seq, header, text, ..
            } if header.contains("failed") => {
                open_questions.push(claim(&format!("{header} failed: {text}"), seq));
            }
            _ => {}
        }
    }
    json!({
        "title": title,
        "decisions": decisions,
        "preferences": [],
        "tasks": [],
        "artifacts": [],
        "open_questions": open_questions,
        "superseded": []
    })
}

fn merge(request: &LlmRequest) -> Value {
    let body = parse_summary_sections(&request.prompt);
    let mut answer = json!({ "title": "Earlier conversation" });
    for section in Section::ALL {
        let claims: Vec<Value> = body
            .sections
            .get(&section)
            .map(|claims| {
                claims
                    .iter()
                    .map(|claim| json!({ "text": claim.text, "seqs": claim.seqs, "unverified": claim.unverified }))
                    .collect()
            })
            .unwrap_or_default();
        answer[section.key()] = json!(claims);
    }
    answer
}

fn extract(request: &LlmRequest) -> Value {
    let mut facts = Vec::new();
    for line in span_lines(&request.prompt, "Messages to extract from:") {
        match line {
            SpanLine::Message { seq, role, text } if role == "user" => {
                for caps in MY_X_IS_Y.captures_iter(&text) {
                    let attribute = caps[1].trim().to_lowercase();
                    let value = caps[2].trim();
                    let kind = if attribute.contains("favourite") || attribute.contains("favorite")
                    {
                        "preference"
                    } else {
                        "profile"
                    };
                    facts.push(json!({
                        "text": format!("The user's {attribute} is {value}."),
                        "kind": kind,
                        "subject": "user",
                        "valid_from": null,
                        "valid_to": null,
                        "evidence": [{ "pointer": format!("#{seq}"), "quote": &caps[0] }],
                        "source": "user_stated",
                        "confidence": 0.9
                    }));
                }
            }
            SpanLine::Part {
                seq, part, text, ..
            } if text.to_lowercase().contains("remember") => {
                facts.push(json!({
                    "text": format!("The user wants this: {text}"),
                    "kind": "preference",
                    "subject": "user",
                    "valid_from": null,
                    "valid_to": null,
                    "evidence": [{ "pointer": format!("#{seq}.{part}"), "quote": text }],
                    "source": "user_stated",
                    "confidence": 1.0
                }));
            }
            _ => {}
        }
    }
    json!({ "facts": facts })
}

fn attribute(text: &str) -> Option<(String, String)> {
    let caps = ATTRIBUTE.captures(text.trim())?;
    Some((normalize(&caps[1]), normalize(&caps[2])))
}

fn decide(request: &LlmRequest) -> Value {
    let mut existing = Vec::new();
    let mut candidate = None;
    let mut in_new = false;
    for line in request.prompt.lines() {
        if line.starts_with("New fact:") {
            in_new = true;
            continue;
        }
        if let Some(caps) = FACT_LINE.captures(line) {
            if in_new {
                candidate = Some(caps[2].to_string());
            } else {
                existing.push((caps[1].to_string(), caps[2].to_string()));
            }
        }
    }
    let Some((name, value)) = candidate.as_deref().and_then(attribute) else {
        return json!({ "action": "add", "target_id": null });
    };
    for (id, text) in existing {
        if let Some((other_name, other_value)) = attribute(&text)
            && other_name == name
        {
            let action = if other_value == value {
                "merge"
            } else {
                "supersede"
            };
            return json!({ "action": action, "target_id": id });
        }
    }
    json!({ "action": "add", "target_id": null })
}

fn now_block(request: &LlmRequest) -> Value {
    let goal = span_lines(&request.prompt, "Latest messages:")
        .into_iter()
        .rev()
        .find_map(|line| match line {
            SpanLine::Message { role, text, .. } if role == "user" => Some(text),
            _ => None,
        })
        .unwrap_or_default();
    json!({ "goal": goal, "next_steps": [] })
}

impl StubLlm {
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers `task` with `answer` instead of the built-in rule.
    pub fn with(
        self,
        task: LlmTask,
        answer: impl Fn(&LlmRequest) -> Result<String> + Send + Sync + 'static,
    ) -> Self {
        self.answers
            .lock()
            .expect("stub answers")
            .insert(task, Box::new(answer));
        self
    }

    pub fn requests(&self) -> Vec<LlmRequest> {
        self.requests.lock().expect("stub requests").clone()
    }

    pub fn calls(&self, task: LlmTask) -> usize {
        self.requests()
            .iter()
            .filter(|request| request.task == task)
            .count()
    }
}

#[async_trait]
impl MemoryLlm for StubLlm {
    async fn complete(&self, request: LlmRequest) -> Result<String> {
        self.requests
            .lock()
            .expect("stub requests")
            .push(request.clone());
        if let Some(answer) = self
            .answers
            .lock()
            .expect("stub answers")
            .get(&request.task)
        {
            return answer(&request);
        }
        let answer = match request.task {
            LlmTask::SegmentSummary => summarise(&request),
            LlmTask::MergeSummaries => merge(&request),
            LlmTask::FactExtraction => extract(&request),
            LlmTask::FactUpdate => decide(&request),
            LlmTask::NowBlock => now_block(&request),
        };
        Ok(answer.to_string())
    }
}

/// Bag-of-words vectors from hashed words: texts sharing words are close, deterministic and
/// offline.
pub struct HashEmbedder {
    model: String,
    dimension: usize,
}

impl HashEmbedder {
    pub fn new(model: impl Into<String>, dimension: usize) -> Self {
        Self {
            model: model.into(),
            dimension: dimension.max(1),
        }
    }

    pub fn embed(&self, text: &str) -> Vec<f32> {
        let mut vector = vec![0.0f32; self.dimension];
        for word in normalize(text).split(' ').filter(|word| !word.is_empty()) {
            let hash = blake3::hash(word.as_bytes());
            let bytes: [u8; 8] = hash.as_bytes()[..8].try_into().expect("eight bytes");
            let value = u64::from_le_bytes(bytes);
            let bucket = (value % self.dimension as u64) as usize;
            vector[bucket] += if value >> 63 == 0 { 1.0 } else { -1.0 };
        }
        let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            vector.iter_mut().for_each(|x| *x /= norm);
        } else {
            vector[0] = 1.0;
        }
        vector
    }
}

#[async_trait]
impl Embedder for HashEmbedder {
    fn model_id(&self) -> &str {
        &self.model
    }

    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|text| self.embed(text)).collect())
    }

    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.embed(text))
    }
}

/// An in-memory history with stored parts and tombstones.
#[derive(Default)]
pub struct MemoryHistory {
    items: Mutex<BTreeMap<Seq, HistoryItem>>,
    parts: Mutex<HashMap<(Seq, PartRef), String>>,
}

impl MemoryHistory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, item: HistoryItem) {
        self.items.lock().expect("history").insert(item.seq, item);
    }

    pub fn push_part(&self, seq: Seq, part: PartRef, content: impl Into<String>) {
        self.parts
            .lock()
            .expect("parts")
            .insert((seq, part), content.into());
    }

    /// Leaves a tombstone; the message's parts go with it.
    pub fn delete(&self, seq: Seq) -> bool {
        let mut items = self.items.lock().expect("history");
        let Some(item) = items.get_mut(&seq) else {
            return false;
        };
        item.deleted = true;
        item.text.clear();
        item.text_tokens = 0;
        item.tools.clear();
        item.ui.clear();
        self.parts
            .lock()
            .expect("parts")
            .retain(|(part_seq, _), _| *part_seq != seq);
        true
    }

    pub fn items(&self) -> Vec<HistoryItem> {
        self.items
            .lock()
            .expect("history")
            .values()
            .cloned()
            .collect()
    }

    pub fn latest(&self) -> Option<Seq> {
        self.items
            .lock()
            .expect("history")
            .keys()
            .next_back()
            .copied()
    }

    pub fn get(&self, seq: Seq) -> Option<HistoryItem> {
        self.items.lock().expect("history").get(&seq).cloned()
    }
}

#[async_trait]
impl HistoryReader for MemoryHistory {
    async fn message(&self, seq: Seq) -> Result<Option<HistoryItem>> {
        Ok(self.get(seq))
    }

    async fn before(&self, seq: Seq, limit: usize) -> Result<Vec<HistoryItem>> {
        let items = self.items.lock().expect("history");
        let mut found: Vec<HistoryItem> = items
            .range(..seq)
            .rev()
            .take(limit)
            .map(|(_, item)| item.clone())
            .collect();
        found.reverse();
        Ok(found)
    }

    async fn after(&self, seq: Seq, limit: usize) -> Result<Vec<HistoryItem>> {
        let items = self.items.lock().expect("history");
        Ok(items
            .range(seq.saturating_add(1)..)
            .take(limit)
            .map(|(_, item)| item.clone())
            .collect())
    }

    async fn range(&self, span: SeqSpan, limit: usize) -> Result<Vec<HistoryItem>> {
        let items = self.items.lock().expect("history");
        Ok(items
            .range(span.from..=span.to)
            .take(limit)
            .map(|(_, item)| item.clone())
            .collect())
    }

    async fn part(&self, seq: Seq, part: PartRef) -> Result<Option<String>> {
        Ok(self.parts.lock().expect("parts").get(&(seq, part)).cloned())
    }
}

/// Builds a conversation into a [`MemoryHistory`] with consecutive seqs and a moving clock.
pub struct Transcript {
    pub history: MemoryHistory,
    next_seq: Seq,
    clock: DateTime<Utc>,
}

impl Transcript {
    pub fn new(start: DateTime<Utc>) -> Self {
        Self {
            history: MemoryHistory::new(),
            next_seq: 1,
            clock: start,
        }
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.clock
    }

    pub fn advance(&mut self, by: Duration) -> &mut Self {
        self.clock += by;
        self
    }

    fn add(
        &mut self,
        role: Role,
        text: &str,
        build: impl FnOnce(HistoryItem) -> HistoryItem,
    ) -> Seq {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.clock += Duration::minutes(1);
        self.history.push(build(HistoryItem::new(
            seq,
            format!("msg_{seq}"),
            role,
            self.clock,
            text,
        )));
        seq
    }

    pub fn user(&mut self, text: &str) -> Seq {
        self.add(Role::User, text, |item| item)
    }

    pub fn bot(&mut self, text: &str) -> Seq {
        self.add(Role::Assistant, text, |item| item)
    }

    /// A bot message with one finished tool call whose full result is stored as part `t1`.
    pub fn bot_with_tool(
        &mut self,
        text: &str,
        tool: &str,
        result: &str,
        gist: &str,
        failed: bool,
    ) -> Seq {
        let tokens = crate::text::estimate_tokens(result);
        let call_id = format!("call_{}", self.next_seq);
        let seq = self.add(Role::Assistant, text, |item| {
            item.with_tool(ToolPart::call(1, call_id, tool, 12).with_result(
                tokens,
                failed,
                Some(gist),
            ))
        });
        self.history.push_part(seq, PartRef::Tool(1), result);
        seq
    }

    /// A bot message whose tool call has no result yet.
    pub fn bot_with_pending_call(&mut self, text: &str, tool: &str, call_id: &str) -> Seq {
        self.add(Role::Assistant, text, |item| {
            item.with_tool(ToolPart::call(1, call_id, tool, 12))
        })
    }

    /// A tool result arriving in its own item for an earlier call.
    pub fn tool_result(&mut self, call_id: &str, tool: &str, result: &str) -> Seq {
        let tokens = crate::text::estimate_tokens(result);
        let seq = self.add(Role::Tool, "", |item| {
            item.with_tool(ToolPart::call(1, call_id, tool, 0).with_result(
                tokens,
                false,
                Some(&truncate_chars(result, 60)),
            ))
        });
        self.history.push_part(seq, PartRef::Tool(1), result);
        seq
    }

    pub fn items(&self) -> Vec<HistoryItem> {
        self.history.items()
    }
}

/// The first message of `items` that is not a tombstone and contains `needle`.
pub fn find_message<'a>(items: &'a [HistoryItem], needle: &str) -> Result<&'a HistoryItem> {
    items
        .iter()
        .find(|item| !item.deleted && item.text.contains(needle))
        .ok_or_else(|| anyhow!("No message contains '{needle}'"))
}
