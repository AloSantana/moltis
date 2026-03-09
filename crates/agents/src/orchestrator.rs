//! Agent orchestration layer.
//!
//! Maintains a registry of specialised agent roles, routes tasks to the best
//! matching agent, and tracks per-role execution statistics.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

// ── Agent role ───────────────────────────────────────────────────────────────

/// Specialised agent personas available in the orchestration system.
///
/// Variants are matched structurally (not by string) throughout the codebase;
/// conversion to a display string only happens at serialisation boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    /// Fast autonomous code implementation.
    RapidImplementer,
    /// System architecture and design.
    Architect,
    /// Debugging and root-cause analysis.
    DebugDetective,
    /// Comprehensive research and analysis.
    DeepResearcher,
    /// Complete web-application development.
    FullStackDeveloper,
    /// Docker, Kubernetes, CI/CD.
    DevOpsInfra,
    /// Testing and validation.
    TestingExpert,
    /// Performance profiling and optimisation.
    PerformanceOptimizer,
    /// Security and quality reviews.
    CodeReviewer,
    /// Documentation creation.
    DocsMaster,
    /// Repository setup and tooling.
    RepoOptimizer,
    /// API design and implementation.
    ApiDeveloper,
}

impl AgentRole {
    /// All roles in definition order.
    pub const ALL: &'static [Self] = &[
        Self::RapidImplementer,
        Self::Architect,
        Self::DebugDetective,
        Self::DeepResearcher,
        Self::FullStackDeveloper,
        Self::DevOpsInfra,
        Self::TestingExpert,
        Self::PerformanceOptimizer,
        Self::CodeReviewer,
        Self::DocsMaster,
        Self::RepoOptimizer,
        Self::ApiDeveloper,
    ];

    /// Human-readable display name.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::RapidImplementer => "Rapid Implementer",
            Self::Architect => "Architect",
            Self::DebugDetective => "Debug Detective",
            Self::DeepResearcher => "Deep Researcher",
            Self::FullStackDeveloper => "Full-Stack Developer",
            Self::DevOpsInfra => "DevOps & Infra",
            Self::TestingExpert => "Testing Expert",
            Self::PerformanceOptimizer => "Performance Optimizer",
            Self::CodeReviewer => "Code Reviewer",
            Self::DocsMaster => "Docs Master",
            Self::RepoOptimizer => "Repo Optimizer",
            Self::ApiDeveloper => "API Developer",
        }
    }

    /// Short description of what this role is best at.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::RapidImplementer => "Fast autonomous code implementation",
            Self::Architect => "System architecture and design",
            Self::DebugDetective => "Debugging and root-cause analysis",
            Self::DeepResearcher => "Comprehensive research and analysis",
            Self::FullStackDeveloper => "Complete web-application development",
            Self::DevOpsInfra => "Docker, Kubernetes, CI/CD pipelines",
            Self::TestingExpert => "Testing and validation",
            Self::PerformanceOptimizer => "Performance profiling and optimisation",
            Self::CodeReviewer => "Security and quality code reviews",
            Self::DocsMaster => "Documentation creation",
            Self::RepoOptimizer => "Repository setup and tooling",
            Self::ApiDeveloper => "API design and implementation",
        }
    }
}

impl std::fmt::Display for AgentRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

// ── Execution statistics ─────────────────────────────────────────────────────

/// Running statistics for a single agent role.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentStats {
    /// Total tasks routed to this agent.
    pub task_count: u64,
    /// Number of successful completions.
    pub success_count: u64,
    /// Number of failures.
    pub failure_count: u64,
    /// Sum of all recorded latencies (for computing mean).
    #[serde(skip)]
    total_latency: Duration,
    /// Mean latency in milliseconds (serialised field).
    pub mean_latency_ms: u64,
}

impl AgentStats {
    /// Record a completed task.
    pub fn record(&mut self, success: bool, latency: Duration) {
        self.task_count += 1;
        self.total_latency += latency;
        self.mean_latency_ms =
            self.total_latency.as_millis() as u64 / self.task_count.max(1);
        if success {
            self.success_count += 1;
        } else {
            self.failure_count += 1;
        }
    }

    /// Success rate in the range `[0.0, 1.0]`.
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        if self.task_count == 0 {
            return 1.0;
        }
        self.success_count as f64 / self.task_count as f64
    }
}

// ── Execution result ─────────────────────────────────────────────────────────

/// Outcome of a single orchestrated agent execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExecutionResult {
    /// The role that handled the task.
    pub role: AgentRole,
    /// Whether the execution succeeded.
    pub success: bool,
    /// Output produced by the agent.
    pub output: String,
    /// Wall-clock latency.
    pub latency_ms: u64,
}

// ── Orchestrator ─────────────────────────────────────────────────────────────

/// Multi-agent orchestrator.
///
/// Holds per-role statistics and delegates routing/execution decisions to the
/// [`crate::routing`] and [`crate::parallel`] modules.  All state is behind an
/// `Arc<RwLock<…>>` so it can be shared across async tasks.
pub struct Orchestrator {
    inner: Arc<RwLock<OrchestratorInner>>,
}

struct OrchestratorInner {
    stats: HashMap<AgentRole, AgentStats>,
    /// Roles that are currently disabled.
    disabled: std::collections::HashSet<AgentRole>,
}

impl Default for OrchestratorInner {
    fn default() -> Self {
        let stats = AgentRole::ALL.iter().map(|&r| (r, AgentStats::default())).collect();
        Self {
            stats,
            disabled: std::collections::HashSet::new(),
        }
    }
}

impl Default for Orchestrator {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(OrchestratorInner::default())),
        }
    }
}

impl Orchestrator {
    /// Create a new orchestrator with all roles enabled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Disable an agent role so it is excluded from routing.
    pub async fn disable_role(&self, role: AgentRole) {
        self.inner.write().await.disabled.insert(role);
    }

    /// Re-enable a previously disabled role.
    pub async fn enable_role(&self, role: AgentRole) {
        self.inner.write().await.disabled.remove(&role);
    }

    /// Return a snapshot of statistics for all roles.
    pub async fn stats_snapshot(&self) -> HashMap<AgentRole, AgentStats> {
        self.inner.read().await.stats.clone()
    }

    /// Record the result of an execution for the given role.
    pub async fn record(&self, role: AgentRole, success: bool, latency: Duration) {
        let mut inner = self.inner.write().await;
        inner.stats.entry(role).or_default().record(success, latency);
    }

    /// List all enabled roles.
    pub async fn enabled_roles(&self) -> Vec<AgentRole> {
        let inner = self.inner.read().await;
        AgentRole::ALL
            .iter()
            .filter(|r| !inner.disabled.contains(r))
            .copied()
            .collect()
    }

    /// Route a task description to the best available agent role and record
    /// the (synthetic) result.  Returns the chosen role and a stub output.
    ///
    /// In a full integration the caller would invoke the agent loop; this
    /// method focuses on selection and stat tracking.
    pub async fn route_and_record(
        &self,
        task: &str,
        exclude: &[AgentRole],
    ) -> Option<AgentExecutionResult> {
        let enabled = self.enabled_roles().await;
        let candidates: Vec<AgentRole> = enabled
            .into_iter()
            .filter(|r| !exclude.contains(r))
            .collect();

        let chosen = crate::routing::score_and_select(task, &candidates)?;
        let start = Instant::now();

        // Stub: real integration would run the agent loop here.
        let output = format!(
            "Task routed to {} ({})",
            chosen.display_name(),
            chosen.description()
        );
        let latency = start.elapsed();
        self.record(chosen, true, latency).await;

        Some(AgentExecutionResult {
            role: chosen,
            success: true,
            output,
            latency_ms: latency.as_millis() as u64,
        })
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{AgentRole, AgentStats, Orchestrator};
    use std::time::Duration;

    #[test]
    fn all_roles_have_display_names() {
        for &role in AgentRole::ALL {
            assert!(!role.display_name().is_empty());
            assert!(!role.description().is_empty());
        }
    }

    #[test]
    fn agent_stats_record() {
        let mut stats = AgentStats::default();
        assert_eq!(stats.success_rate(), 1.0);

        stats.record(true, Duration::from_millis(100));
        stats.record(false, Duration::from_millis(200));

        assert_eq!(stats.task_count, 2);
        assert_eq!(stats.success_count, 1);
        assert_eq!(stats.failure_count, 1);
        assert!((stats.success_rate() - 0.5).abs() < f64::EPSILON);
        assert_eq!(stats.mean_latency_ms, 150);
    }

    #[tokio::test]
    async fn orchestrator_disable_enable() {
        let orch = Orchestrator::new();
        let all_count = orch.enabled_roles().await.len();

        orch.disable_role(AgentRole::Architect).await;
        let after_disable = orch.enabled_roles().await;
        assert_eq!(after_disable.len(), all_count - 1);
        assert!(!after_disable.contains(&AgentRole::Architect));

        orch.enable_role(AgentRole::Architect).await;
        assert_eq!(orch.enabled_roles().await.len(), all_count);
    }

    #[tokio::test]
    async fn orchestrator_route_and_record() {
        let orch = Orchestrator::new();
        let result = orch.route_and_record("implement a new feature", &[]).await;
        assert!(result.is_some());
        let r = result.unwrap();
        assert!(r.success);

        let stats = orch.stats_snapshot().await;
        let chosen_stats = stats.get(&r.role).unwrap();
        assert_eq!(chosen_stats.task_count, 1);
        assert_eq!(chosen_stats.success_count, 1);
    }

    #[tokio::test]
    async fn orchestrator_exclude_list() {
        let orch = Orchestrator::new();
        // Exclude all but one role to force a deterministic selection.
        let exclude: Vec<AgentRole> = AgentRole::ALL
            .iter()
            .copied()
            .filter(|&r| r != AgentRole::Architect)
            .collect();
        let result = orch.route_and_record("design a system", &exclude).await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().role, AgentRole::Architect);
    }

    #[test]
    fn agent_role_serialises_as_snake_case() {
        let json = serde_json::to_string(&AgentRole::RapidImplementer).unwrap();
        assert_eq!(json, "\"rapid_implementer\"");
    }
}

