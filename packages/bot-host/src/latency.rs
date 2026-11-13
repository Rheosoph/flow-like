//! One structured log line per live-call turn: where the time between a spoken utterance and the
//! bot's reply went on this host.

use chrono::{DateTime, Utc};
use flow_like::flow::copilot::decision::Decision;
use std::time::Instant;

#[derive(Debug)]
pub struct CallTrace {
    started: Instant,
    pub turn_id: String,
    claim_ms: Option<i64>,
    pub transcript: bool,
    pub parallel: bool,
    decision_started: Option<Instant>,
    decision_ms: Option<u64>,
    decision: &'static str,
    run_started: Option<Instant>,
    model_ms: Option<u64>,
}

pub fn decision_label<E>(decision: &Result<Result<Decision, E>, tokio::time::error::Elapsed>) -> &'static str {
    match decision {
        Ok(Ok(Decision::AnswerNow)) => "answer_now",
        Ok(Ok(Decision::NoAnswer)) => "no_answer",
        Ok(Ok(Decision::WaitFor(_))) => "wait_for",
        Ok(Ok(Decision::Acknowledge)) => "acknowledge",
        Ok(Err(_)) => "error",
        Err(_) => "timeout",
    }
}

impl CallTrace {
    /// `queued_at` is when the utterance's turn was queued; the trace starts when a host picks it up.
    pub fn new(queued_at: Option<DateTime<Utc>>) -> Self {
        Self {
            started: Instant::now(),
            turn_id: String::new(),
            claim_ms: queued_at.map(|queued| (Utc::now() - queued).num_milliseconds()),
            transcript: false,
            parallel: false,
            decision_started: None,
            decision_ms: None,
            decision: "none",
            run_started: None,
            model_ms: None,
        }
    }

    /// Counts from `queued_at` instead of from this trace's creation.
    pub fn queued(&mut self, queued_at: Option<DateTime<Utc>>) {
        self.claim_ms = queued_at.map(|queued| (Utc::now() - queued).num_milliseconds());
    }

    pub fn queued_at(rfc3339: &str) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(rfc3339).ok().map(|at| at.to_utc())
    }

    pub fn decision_started(&mut self) {
        self.decision_started = Some(Instant::now());
    }

    pub fn decided(&mut self, label: &'static str) {
        self.decision = label;
        self.decision_ms = self
            .decision_started
            .map(|started| started.elapsed().as_millis() as u64);
    }

    pub fn run_started(&mut self) {
        self.run_started = Some(Instant::now());
    }

    pub fn run_finished(&mut self) {
        self.model_ms = self
            .run_started
            .map(|started| started.elapsed().as_millis() as u64);
    }

    pub fn log(&self, bot_id: &str, status: &str) {
        if !self.transcript {
            return;
        }
        tracing::info!(
            target: "bot_call_latency",
            turn_id = %self.turn_id,
            bot_id,
            claim_ms = self.claim_ms,
            decision_ms = self.decision_ms,
            decision = self.decision,
            parallel = self.parallel,
            model_ms = self.model_ms,
            total_ms = self.started.elapsed().as_millis() as u64,
            status,
            "Bot call turn finished"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trace_records_phases_and_only_logs_transcript_turns() {
        let mut trace = CallTrace::new(None);
        trace.decision_started();
        trace.decided("no_answer");
        trace.run_started();
        trace.run_finished();
        assert_eq!(trace.decision, "no_answer");
        assert!(trace.decision_ms.is_some() && trace.model_ms.is_some());
        assert!(!trace.transcript);
    }
}
