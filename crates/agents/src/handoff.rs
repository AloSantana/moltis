//! Agent handoff mechanism.
//!
//! Records context transfers between agent roles and maintains a full handoff
//! history for debugging.  Context includes conversation history, modified
//! files, and arbitrary task state.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::orchestrator::AgentRole;

// ── Context ───────────────────────────────────────────────────────────────────

/// Snapshot of task state transferred during a handoff.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HandoffContext {
    /// Conversation turns accumulated so far (role → content pairs).
    pub history: Vec<HistoryEntry>,
    /// Paths of files modified during this task.
    pub modified_files: Vec<String>,
    /// Arbitrary key/value task state (e.g. partial results, flags).
    pub task_state: HashMap<String, serde_json::Value>,
    /// Human-readable summary of progress.
    pub summary: Option<String>,
}

impl HandoffContext {
    /// Create an empty context.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a conversation entry.
    pub fn push_history(&mut self, role: &str, content: impl Into<String>) {
        self.history.push(HistoryEntry {
            role: role.to_string(),
            content: content.into(),
        });
    }

    /// Record a modified file path.
    pub fn add_modified_file(&mut self, path: impl Into<String>) {
        self.modified_files.push(path.into());
    }

    /// Insert or overwrite a task-state entry.
    pub fn set_state(&mut self, key: impl Into<String>, value: serde_json::Value) {
        self.task_state.insert(key.into(), value);
    }
}

/// A single turn in the conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Who produced this turn (e.g. `"user"`, `"assistant"`, agent role name).
    pub role: String,
    pub content: String,
}

// ── Handoff record ────────────────────────────────────────────────────────────

/// A single handoff event recorded in the history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffRecord {
    /// Role that is passing control.
    pub from: AgentRole,
    /// Role that is receiving control.
    pub to: AgentRole,
    /// Human-readable reason for the transfer.
    pub reason: String,
    /// Context snapshot at the moment of the handoff.
    pub context: HandoffContext,
    /// Unix timestamp (milliseconds) when the handoff occurred.
    pub timestamp_ms: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Manager ───────────────────────────────────────────────────────────────────

/// Tracks all handoffs for a single task execution.
#[derive(Debug, Default)]
pub struct HandoffManager {
    history: Vec<HandoffRecord>,
}

impl HandoffManager {
    /// Create a new, empty manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a handoff from `from` to `to` with the given reason and context.
    pub fn record(
        &mut self,
        from: AgentRole,
        to: AgentRole,
        reason: impl Into<String>,
        context: HandoffContext,
    ) {
        self.history.push(HandoffRecord {
            from,
            to,
            reason: reason.into(),
            context,
            timestamp_ms: now_ms(),
        });
    }

    /// Return the full handoff history (newest last).
    #[must_use]
    pub fn history(&self) -> &[HandoffRecord] {
        &self.history
    }

    /// Return the most recent handoff, if any.
    #[must_use]
    pub fn last(&self) -> Option<&HandoffRecord> {
        self.history.last()
    }

    /// Number of handoffs recorded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.history.len()
    }

    /// `true` if no handoffs have been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn make_context() -> HandoffContext {
        let mut ctx = HandoffContext::new();
        ctx.push_history("user", "Fix the login bug");
        ctx.push_history("assistant", "Investigating…");
        ctx.add_modified_file("src/auth.rs");
        ctx.set_state("step", serde_json::json!("root-cause-found"));
        ctx
    }

    #[test]
    fn handoff_context_fields() {
        let ctx = make_context();
        assert_eq!(ctx.history.len(), 2);
        assert_eq!(ctx.modified_files, ["src/auth.rs"]);
        assert_eq!(ctx.task_state["step"], serde_json::json!("root-cause-found"));
    }

    #[test]
    fn handoff_manager_record_and_retrieve() {
        let mut mgr = HandoffManager::new();
        assert!(mgr.is_empty());

        let ctx = make_context();
        mgr.record(
            AgentRole::DebugDetective,
            AgentRole::TestingExpert,
            "Security review needed",
            ctx.clone(),
        );

        assert_eq!(mgr.len(), 1);
        let rec = mgr.last().unwrap();
        assert_eq!(rec.from, AgentRole::DebugDetective);
        assert_eq!(rec.to, AgentRole::TestingExpert);
        assert_eq!(rec.reason, "Security review needed");
        assert_eq!(rec.context.history.len(), 2);
    }

    #[test]
    fn handoff_manager_multiple_records() {
        let mut mgr = HandoffManager::new();
        let ctx = HandoffContext::new();

        mgr.record(
            AgentRole::Architect,
            AgentRole::RapidImplementer,
            "Ready to implement",
            ctx.clone(),
        );
        mgr.record(
            AgentRole::RapidImplementer,
            AgentRole::TestingExpert,
            "Regression testing needed",
            ctx.clone(),
        );

        assert_eq!(mgr.len(), 2);
        assert_eq!(mgr.history()[0].from, AgentRole::Architect);
        assert_eq!(mgr.history()[1].from, AgentRole::RapidImplementer);
        assert_eq!(mgr.last().unwrap().to, AgentRole::TestingExpert);
    }

    #[test]
    fn handoff_record_serialises() {
        let mut mgr = HandoffManager::new();
        mgr.record(
            AgentRole::CodeReviewer,
            AgentRole::DebugDetective,
            "Vulnerability found",
            HandoffContext::new(),
        );
        let json = serde_json::to_string(mgr.last().unwrap()).unwrap();
        assert!(json.contains("code_reviewer"));
        assert!(json.contains("debug_detective"));
    }
}
