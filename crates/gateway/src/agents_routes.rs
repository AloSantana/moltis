//! REST API routes for multi-agent orchestration.
//!
//! Endpoints:
//! - `POST /api/agents/route`    — route a task to the best agent
//! - `POST /api/agents/execute`  — execute a task with a specific agent
//! - `GET  /api/agents/status`   — list all agents with their status/stats
//! - `POST /api/agents/handoff`  — record an agent handoff
//! - `GET  /api/agents/history`  — get handoff history for the current session

#[cfg(feature = "agents-orchestration")]
mod inner {
    use std::sync::Arc;

    use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
    use moltis_agents::{
        handoff::{HandoffContext, HandoffManager},
        orchestrator::AgentRole,
        parallel::{OrchestrationPlan, execute_plan},
        routing::ranked_candidates,
    };
    use serde::{Deserialize, Serialize};
    use tokio::sync::RwLock;

    use crate::server::AppState;

    // ── Request / response types ──────────────────────────────────────────────

    /// Request body for `POST /api/agents/route`.
    #[derive(Debug, Deserialize)]
    pub struct RouteRequest {
        /// Natural-language task description.
        pub task: String,
        /// Agent roles to exclude from selection.
        #[serde(default)]
        pub exclude: Vec<AgentRole>,
    }

    /// Request body for `POST /api/agents/execute`.
    #[derive(Debug, Deserialize)]
    pub struct ExecuteRequest {
        /// Natural-language task description.
        pub task: String,
        /// Optional specific role to use (bypasses routing if provided).
        pub role: Option<AgentRole>,
        /// Orchestration plan to use instead of single-role execution.
        pub plan: Option<PlanName>,
    }

    /// Named orchestration plan variants available via the API.
    #[derive(Debug, Deserialize, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum PlanName {
        ComprehensiveAnalysis,
        FeatureReview,
        SecurityAudit,
    }

    /// Request body for `POST /api/agents/handoff`.
    #[derive(Debug, Deserialize)]
    pub struct HandoffRequest {
        pub from: AgentRole,
        pub to: AgentRole,
        pub reason: String,
        #[serde(default)]
        pub context: HandoffContext,
    }

    // ── Shared handoff store (per-process, non-persistent) ───────────────────

    /// Process-global handoff manager, wrapped for sharing across handlers.
    static HANDOFF_MANAGER: std::sync::OnceLock<Arc<RwLock<HandoffManager>>> =
        std::sync::OnceLock::new();

    fn handoff_manager() -> Arc<RwLock<HandoffManager>> {
        HANDOFF_MANAGER
            .get_or_init(|| Arc::new(RwLock::new(HandoffManager::new())))
            .clone()
    }

    // ── Handlers ─────────────────────────────────────────────────────────────

    /// `GET /api/agents/status` — list all agent roles with their statistics.
    pub async fn agents_status(
        State(state): State<AppState>,
    ) -> impl IntoResponse {
        let stats = state.gateway.orchestrator.stats_snapshot().await;
        let enabled = state.gateway.orchestrator.enabled_roles().await;

        let roles: Vec<serde_json::Value> = AgentRole::ALL
            .iter()
            .map(|&role| {
                let role_stats = stats.get(&role).cloned().unwrap_or_default();
                serde_json::json!({
                    "role": role,
                    "displayName": role.display_name(),
                    "description": role.description(),
                    "enabled": enabled.contains(&role),
                    "stats": {
                        "taskCount": role_stats.task_count,
                        "successCount": role_stats.success_count,
                        "failureCount": role_stats.failure_count,
                        "successRate": role_stats.success_rate(),
                        "meanLatencyMs": role_stats.mean_latency_ms,
                    },
                })
            })
            .collect();

        Json(serde_json::json!({ "agents": roles }))
    }

    /// `POST /api/agents/route` — score and rank agents for a task.
    pub async fn agents_route(
        State(state): State<AppState>,
        Json(req): Json<RouteRequest>,
    ) -> impl IntoResponse {
        if req.task.trim().is_empty() {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "error": "task must not be empty" })),
            )
                .into_response();
        }

        let enabled = state.gateway.orchestrator.enabled_roles().await;
        let candidates: Vec<AgentRole> = enabled
            .into_iter()
            .filter(|r| !req.exclude.contains(r))
            .collect();

        let ranked = ranked_candidates(&req.task, &candidates);
        let best = ranked.first().map(|(r, _)| *r);

        let ranked_json: Vec<serde_json::Value> = ranked
            .iter()
            .map(|(role, score)| {
                serde_json::json!({
                    "role": role,
                    "displayName": role.display_name(),
                    "score": score,
                })
            })
            .collect();

        Json(serde_json::json!({
            "best": best,
            "ranked": ranked_json,
        }))
        .into_response()
    }

    /// `POST /api/agents/execute` — execute a task (single agent or plan).
    pub async fn agents_execute(
        State(state): State<AppState>,
        Json(req): Json<ExecuteRequest>,
    ) -> impl IntoResponse {
        if req.task.trim().is_empty() {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "error": "task must not be empty" })),
            )
                .into_response();
        }

        // If a plan is specified, run the parallel orchestration plan.
        if let Some(plan_name) = req.plan {
            let plan = match plan_name {
                PlanName::ComprehensiveAnalysis => OrchestrationPlan::comprehensive_analysis(),
                PlanName::FeatureReview => OrchestrationPlan::feature_review(),
                PlanName::SecurityAudit => OrchestrationPlan::security_audit(),
            };
            let result = execute_plan(&state.gateway.orchestrator, &plan, &req.task).await;
            return Json(serde_json::to_value(&result).unwrap_or_default()).into_response();
        }

        // Single-role execution.
        let exclude = if let Some(role) = req.role {
            // Exclude every role except the requested one.
            AgentRole::ALL
                .iter()
                .copied()
                .filter(|&r| r != role)
                .collect()
        } else {
            Vec::new()
        };

        match state
            .gateway
            .orchestrator
            .route_and_record(&req.task, &exclude)
            .await
        {
            Some(result) => Json(serde_json::to_value(&result).unwrap_or_default()).into_response(),
            None => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "no eligible agent available" })),
            )
                .into_response(),
        }
    }

    /// `POST /api/agents/handoff` — record a handoff between agents.
    pub async fn agents_handoff(
        Json(req): Json<HandoffRequest>,
    ) -> impl IntoResponse {
        let mgr = handoff_manager();
        mgr.write()
            .await
            .record(req.from, req.to, req.reason, req.context);
        StatusCode::NO_CONTENT
    }

    /// `GET /api/agents/history` — return all recorded handoffs.
    pub async fn agents_history() -> impl IntoResponse {
        let mgr = handoff_manager();
        let guard = mgr.read().await;
        let history = guard.history().to_vec();
        drop(guard);
        Json(serde_json::json!({ "handoffs": history }))
    }

    // ── Router builder ────────────────────────────────────────────────────────

    /// Build the `/api/agents` sub-router.
    pub fn agents_router() -> axum::Router<AppState> {
        use axum::routing::{get, post};

        axum::Router::new()
            .route("/status", get(agents_status))
            .route("/route", post(agents_route))
            .route("/execute", post(agents_execute))
            .route("/handoff", post(agents_handoff))
            .route("/history", get(agents_history))
    }
}

// ── Re-exports ────────────────────────────────────────────────────────────────

#[cfg(feature = "agents-orchestration")]
pub use inner::agents_router;
