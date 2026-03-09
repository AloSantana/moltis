//! Parallel agent execution patterns.
//!
//! Runs independent agent tasks concurrently using `futures::future::join_all`
//! (per CLAUDE.md) and synthesises the results.

use std::time::Instant;

use futures::future;
use serde::{Deserialize, Serialize};

use crate::orchestrator::{AgentExecutionResult, AgentRole, Orchestrator};

// ── Orchestration patterns ────────────────────────────────────────────────────

/// A named orchestration pattern describing which roles participate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationPlan {
    /// Human-readable name of the pattern.
    pub name: String,
    /// Roles to run in the first (parallel) wave.
    pub initial_roles: Vec<AgentRole>,
    /// Optional role that synthesises results from the first wave.
    pub synthesiser: Option<AgentRole>,
}

impl OrchestrationPlan {
    /// **Comprehensive Analysis** — explorer roles in parallel, then synthesis.
    #[must_use]
    pub fn comprehensive_analysis() -> Self {
        Self {
            name: "Comprehensive Analysis".into(),
            initial_roles: vec![
                AgentRole::DeepResearcher,
                AgentRole::Architect,
                AgentRole::CodeReviewer,
            ],
            synthesiser: Some(AgentRole::DocsMaster),
        }
    }

    /// **Feature Review** — affected-domain agents, then test engineer.
    #[must_use]
    pub fn feature_review() -> Self {
        Self {
            name: "Feature Review".into(),
            initial_roles: vec![
                AgentRole::RapidImplementer,
                AgentRole::CodeReviewer,
            ],
            synthesiser: Some(AgentRole::TestingExpert),
        }
    }

    /// **Security Audit** — auditor + penetration tester, then synthesis.
    #[must_use]
    pub fn security_audit() -> Self {
        Self {
            name: "Security Audit".into(),
            initial_roles: vec![
                AgentRole::CodeReviewer,
                AgentRole::DebugDetective,
            ],
            synthesiser: Some(AgentRole::Architect),
        }
    }
}

// ── Parallel execution ────────────────────────────────────────────────────────

/// Results from a parallel execution wave.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelExecutionResult {
    /// Pattern that was executed.
    pub plan_name: String,
    /// Individual results from the initial parallel wave.
    pub wave_results: Vec<AgentExecutionResult>,
    /// Result from the synthesiser role (if any).
    pub synthesis: Option<AgentExecutionResult>,
    /// Total wall-clock latency across the whole plan.
    pub total_latency_ms: u64,
}

/// Execute an orchestration plan against the provided orchestrator.
///
/// The initial-wave roles run **concurrently**; the synthesiser (if any) runs
/// after all wave results are collected, receiving a combined task description.
pub async fn execute_plan(
    orch: &Orchestrator,
    plan: &OrchestrationPlan,
    task: &str,
) -> ParallelExecutionResult {
    let start = Instant::now();

    // Build one future per initial role.
    let wave_futures: Vec<_> = plan
        .initial_roles
        .iter()
        .map(|&role| {
            // Exclude all other initial roles so scoring picks `role`.
            let exclude: Vec<AgentRole> = plan
                .initial_roles
                .iter()
                .copied()
                .filter(|&r| r != role)
                .collect();
            let task = task.to_string();
            async move {
                // Build a single-role task description and force selection.
                let result = orch.route_and_record(&task, &exclude).await;
                result.unwrap_or_else(|| AgentExecutionResult {
                    role,
                    success: false,
                    output: "No eligible agent available".into(),
                    latency_ms: 0,
                })
            }
        })
        .collect();

    // Run all wave futures concurrently.
    let wave_results = future::join_all(wave_futures).await;

    // Synthesise: build a combined task from all outputs.
    let synthesis = if let Some(synth_role) = plan.synthesiser {
        let combined = wave_results
            .iter()
            .map(|r| format!("[{}]: {}", r.role.display_name(), r.output))
            .collect::<Vec<_>>()
            .join("\n");
        let synth_task = format!("Synthesise the following analyses:\n{combined}");
        let exclude: Vec<AgentRole> = plan
            .initial_roles
            .iter()
            .copied()
            .filter(|&r| r != synth_role)
            .collect();
        orch.route_and_record(&synth_task, &exclude).await
    } else {
        None
    };

    ParallelExecutionResult {
        plan_name: plan.name.clone(),
        wave_results,
        synthesis,
        total_latency_ms: start.elapsed().as_millis() as u64,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parallel_wave_produces_correct_count() {
        let orch = Orchestrator::new();
        let plan = OrchestrationPlan::comprehensive_analysis();
        let result = execute_plan(&orch, &plan, "analyse the authentication module").await;

        assert_eq!(result.wave_results.len(), plan.initial_roles.len());
        assert!(result.synthesis.is_some());
    }

    #[tokio::test]
    async fn feature_review_plan() {
        let orch = Orchestrator::new();
        let plan = OrchestrationPlan::feature_review();
        let result = execute_plan(&orch, &plan, "add new user registration feature").await;

        assert_eq!(result.wave_results.len(), 2);
        assert!(result.synthesis.is_some());
    }

    #[tokio::test]
    async fn security_audit_plan() {
        let orch = Orchestrator::new();
        let plan = OrchestrationPlan::security_audit();
        let result = execute_plan(&orch, &plan, "audit the payment processing code").await;

        assert_eq!(result.wave_results.len(), 2);
    }

    #[tokio::test]
    async fn plan_without_synthesiser() {
        let plan = OrchestrationPlan {
            name: "No Synthesis".into(),
            initial_roles: vec![AgentRole::Architect, AgentRole::DocsMaster],
            synthesiser: None,
        };
        let orch = Orchestrator::new();
        let result = execute_plan(&orch, &plan, "document the system design").await;

        assert_eq!(result.wave_results.len(), 2);
        assert!(result.synthesis.is_none());
    }

    #[tokio::test]
    async fn stats_are_recorded_for_all_roles() {
        let orch = Orchestrator::new();
        let plan = OrchestrationPlan::comprehensive_analysis();
        let result = execute_plan(&orch, &plan, "deep research task").await;

        let stats = orch.stats_snapshot().await;
        // Verify that every role returned in the wave has at least one recorded task.
        for wave_result in &result.wave_results {
            let s = stats.get(&wave_result.role).unwrap();
            assert!(
                s.task_count >= 1,
                "stats not recorded for {}",
                wave_result.role
            );
        }
    }
}
