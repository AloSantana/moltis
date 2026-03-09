//! RPC method handlers for multi-agent orchestration.
//!
//! Methods:
//! - `agents.roles.list`    — list available agent roles with stats (read)
//! - `agents.roles.route`   — route a task to the best agent (read)
//! - `agents.roles.execute` — execute a task with a specific agent (write)

#[cfg(feature = "agents-orchestration")]
pub(super) fn register(reg: &mut super::MethodRegistry) {
    use moltis_agents::{
        orchestrator::AgentRole,
        parallel::{OrchestrationPlan, execute_plan},
        routing::ranked_candidates,
    };
    use moltis_protocol::error_codes;

    // agents.roles.list — list all roles with statistics.
    reg.register(
        "agents.roles.list",
        Box::new(|ctx| {
            Box::pin(async move {
                let orch = &ctx.state.orchestrator;
                let stats = orch.stats_snapshot().await;
                let enabled = orch.enabled_roles().await;

                let roles: Vec<serde_json::Value> = AgentRole::ALL
                    .iter()
                    .map(|&role| {
                        let s = stats.get(&role).cloned().unwrap_or_default();
                        serde_json::json!({
                            "role": role,
                            "displayName": role.display_name(),
                            "description": role.description(),
                            "enabled": enabled.contains(&role),
                            "stats": {
                                "taskCount": s.task_count,
                                "successCount": s.success_count,
                                "failureCount": s.failure_count,
                                "successRate": s.success_rate(),
                                "meanLatencyMs": s.mean_latency_ms,
                            },
                        })
                    })
                    .collect();

                Ok(serde_json::json!({ "agents": roles }))
            })
        }),
    );

    // agents.roles.route — score and rank candidates for a task.
    reg.register(
        "agents.roles.route",
        Box::new(|ctx| {
            Box::pin(async move {
                let task = ctx
                    .params
                    .get("task")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        moltis_protocol::ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "missing 'task' parameter",
                        )
                    })?;

                let exclude: Vec<AgentRole> = ctx
                    .params
                    .get("exclude")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();

                let enabled = ctx.state.orchestrator.enabled_roles().await;
                let candidates: Vec<AgentRole> = enabled
                    .into_iter()
                    .filter(|r| !exclude.contains(r))
                    .collect();

                let ranked = ranked_candidates(task, &candidates);
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

                Ok(serde_json::json!({
                    "best": best,
                    "ranked": ranked_json,
                }))
            })
        }),
    );

    // agents.roles.execute — run a task (single agent or plan).
    reg.register(
        "agents.roles.execute",
        Box::new(|ctx| {
            Box::pin(async move {
                let task = ctx
                    .params
                    .get("task")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        moltis_protocol::ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "missing 'task' parameter",
                        )
                    })?
                    .to_string();

                let plan_name = ctx.params.get("plan").and_then(|v| v.as_str());

                if let Some(name) = plan_name {
                    let plan = match name {
                        "comprehensive_analysis" => OrchestrationPlan::comprehensive_analysis(),
                        "feature_review" => OrchestrationPlan::feature_review(),
                        "security_audit" => OrchestrationPlan::security_audit(),
                        other => {
                            return Err(moltis_protocol::ErrorShape::new(
                                error_codes::INVALID_REQUEST,
                                format!("unknown plan: {other}"),
                            ));
                        },
                    };
                    let result = execute_plan(&ctx.state.orchestrator, &plan, &task).await;
                    return serde_json::to_value(&result).map_err(|e| {
                        moltis_protocol::ErrorShape::new(error_codes::INTERNAL, e.to_string())
                    });
                }

                // Single-role execution.
                let role: Option<AgentRole> = ctx
                    .params
                    .get("role")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());

                let exclude = if let Some(r) = role {
                    AgentRole::ALL.iter().copied().filter(|&a| a != r).collect()
                } else {
                    Vec::new()
                };

                ctx.state
                    .orchestrator
                    .route_and_record(&task, &exclude)
                    .await
                    .map(|r| serde_json::to_value(&r).unwrap_or_default())
                    .ok_or_else(|| {
                        moltis_protocol::ErrorShape::new(
                            error_codes::UNAVAILABLE,
                            "no eligible agent available",
                        )
                    })
            })
        }),
    );
}
