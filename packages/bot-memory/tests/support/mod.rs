//! A host in miniature: a transcript, the engine on a temporary LanceDB directory, the stub
//! model and the hashing embedder. Each exchange is indexed, planned after the turn and
//! compacted when due, the way the bot hosts will run it.

#![allow(dead_code)]

use std::sync::Arc;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use flow_like_bot_memory::compaction::{CompactionConfig, CompactionPlan, TurnPhase};
use flow_like_bot_memory::engine::{CompactionInput, CompactionOutcome, MemoryEngine};
use flow_like_bot_memory::model::{HistoryItem, Seq};
use flow_like_bot_memory::prompt::{AssembledPrompt, ModelBudget, PromptInput, assemble};
use flow_like_bot_memory::store::{CompactionState, MemoryStores};
use flow_like_bot_memory::testing::{HashEmbedder, StubLlm, Transcript};
use flow_like_bot_memory::tools::HistoryReader;
use flow_like_types::Result;
use tempfile::TempDir;

pub fn tz() -> Tz {
    "Europe/Berlin".parse().expect("time zone")
}

pub fn start() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-01T08:00:00Z")
        .expect("timestamp")
        .into()
}

/// One compaction as it ran: the plan and the live items it was made from.
pub struct Ran {
    pub plan: CompactionPlan,
    pub items: Vec<HistoryItem>,
    pub outcome: CompactionOutcome,
}

pub struct Harness {
    _dir: TempDir,
    pub engine: MemoryEngine,
    pub llm: Arc<StubLlm>,
    pub transcript: Transcript,
    /// H for the planner.
    pub window: u32,
    pub state_block: String,
    pub open_tasks: Vec<Seq>,
    pub runs: Vec<Ran>,
}

impl Harness {
    pub async fn new(window: u32) -> Self {
        Self::with_llm(window, StubLlm::new()).await
    }

    pub async fn with_llm(window: u32, llm: StubLlm) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let embedder = Arc::new(HashEmbedder::new("hash-64", 64));
        let stores = MemoryStores::open_local(dir.path(), embedder)
            .await
            .expect("stores");
        let llm = Arc::new(llm);
        Self {
            _dir: dir,
            engine: MemoryEngine::new(stores, llm.clone(), CompactionConfig::default(), tz()),
            llm,
            transcript: Transcript::new(start()),
            window,
            state_block: "No open tasks.".to_string(),
            open_tasks: Vec::new(),
            runs: Vec::new(),
        }
    }

    pub async fn state(&self) -> CompactionState {
        self.engine.stores().state.load().await.expect("state")
    }

    pub async fn live(&self) -> Vec<HistoryItem> {
        let state = self.state().await;
        let latest = self.transcript.history.latest().unwrap_or(0);
        self.engine
            .live_items(&self.transcript.history, &state, latest)
            .await
            .expect("live items")
    }

    /// Indexes the new messages, plans after the turn and compacts when due.
    pub async fn settle(&mut self) -> Result<Option<&Ran>> {
        let state = self.state().await;
        let items = self.live().await;
        self.engine.index(&items).await?;
        let plan = self
            .engine
            .plan(
                &items,
                &state,
                self.window,
                &self.open_tasks,
                TurnPhase::AfterTurn,
            )
            .await?;
        let Some(compaction) = plan.compaction else {
            return Ok(None);
        };
        let preceding = self
            .transcript
            .history
            .before(compaction.evict.from, 10)
            .await?;
        let outcome = self
            .engine
            .compact(CompactionInput {
                plan: &compaction,
                items: &items,
                preceding: &preceding,
                state_block: &self.state_block,
                history_window: self.window,
                now: self.transcript.now(),
                model: Some("stub".into()),
            })
            .await?;
        self.runs.push(Ran {
            plan: compaction,
            items,
            outcome,
        });
        Ok(self.runs.last())
    }

    /// A user message, the bot's answer, then [`Harness::settle`].
    pub async fn exchange(&mut self, user: &str, bot: &str) -> Result<Seq> {
        let seq = self.transcript.user(user);
        self.transcript.bot(bot);
        self.settle().await?;
        Ok(seq)
    }

    /// Small talk until `compactions` have run.
    pub async fn fill_until(&mut self, compactions: u32) -> Result<()> {
        let mut turn = 0;
        while self.state().await.compactions < compactions {
            turn += 1;
            self.exchange(
                &format!(
                    "Filler question {turn} about the weather and the garden, nothing to keep."
                ),
                &format!("Filler answer {turn}: it is mild today and the tomatoes are fine."),
            )
            .await?;
            assert!(turn < 10_000, "no compaction ran");
        }
        Ok(())
    }

    /// The prompt for the next user message, the way the host assembles it.
    pub async fn prompt(&self, user_message: &str, budget: &ModelBudget) -> AssembledPrompt {
        let state = self.state().await;
        let items = self.live().await;
        let plan = self
            .engine
            .plan(
                &items,
                &state,
                self.window,
                &self.open_tasks,
                TurnPhase::BeforeCall,
            )
            .await
            .expect("plan");
        let summaries = self
            .engine
            .stores()
            .segments
            .active()
            .await
            .expect("summaries");
        let facts = self
            .engine
            .recall(user_message, budget, self.transcript.now())
            .await
            .expect("recall");
        assemble(
            &PromptInput {
                system_and_tools: "You are a helpful bot.",
                tool_tokens: 0,
                about_you: "",
                summaries: &summaries,
                now_block: state.now.as_ref(),
                state_block: &self.state_block,
                tail: &items,
                open_task_seqs: &self.open_tasks,
                masking: &plan.masking,
                facts: &facts,
                user_message,
                timezone: tz(),
            },
            budget,
        )
    }
}
