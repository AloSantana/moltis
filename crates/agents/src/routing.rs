//! Smart routing: score agent roles for a given task and select the best match.
//!
//! Scoring is purely type-based — roles are compared as `AgentRole` enum
//! variants, never as strings (per CLAUDE.md: "Match on types, never strings").

use crate::orchestrator::AgentRole;

// ── Keyword tables ────────────────────────────────────────────────────────────

/// A single routing rule: a slice of keywords and the role they signal.
struct RoutingRule {
    role: AgentRole,
    keywords: &'static [&'static str],
    /// Base priority added to keyword score (higher = preferred when tied).
    priority: u32,
}

/// Static routing rules ordered by specificity.
static ROUTING_RULES: &[RoutingRule] = &[
    RoutingRule {
        role: AgentRole::DebugDetective,
        keywords: &[
            "debug", "fix", "bug", "error", "crash", "fail", "broken", "issue",
            "regression", "traceback", "panic", "root cause", "diagnose",
        ],
        priority: 10,
    },
    RoutingRule {
        role: AgentRole::TestingExpert,
        keywords: &[
            "test", "spec", "coverage", "assert", "mock", "integration test",
            "unit test", "e2e", "playwright", "jest", "pytest", "validate",
        ],
        priority: 10,
    },
    RoutingRule {
        role: AgentRole::Architect,
        keywords: &[
            "architect", "design", "system design", "diagram", "schema",
            "structure", "pattern", "blueprint", "high-level", "microservice",
        ],
        priority: 8,
    },
    RoutingRule {
        role: AgentRole::DeepResearcher,
        keywords: &[
            "research", "analyse", "analyze", "investigate", "survey",
            "compare", "benchmark", "literature", "review", "study",
        ],
        priority: 8,
    },
    RoutingRule {
        role: AgentRole::DevOpsInfra,
        keywords: &[
            "docker", "kubernetes", "k8s", "ci", "cd", "deploy", "pipeline",
            "infra", "infrastructure", "helm", "terraform", "ansible",
            "container", "devops",
        ],
        priority: 9,
    },
    RoutingRule {
        role: AgentRole::PerformanceOptimizer,
        keywords: &[
            "performance", "profile", "benchmark", "optimise", "optimize",
            "speed", "latency", "throughput", "bottleneck", "slow", "memory leak",
        ],
        priority: 9,
    },
    RoutingRule {
        role: AgentRole::CodeReviewer,
        keywords: &[
            "review", "security", "audit", "vulnerability", "cve", "owasp",
            "quality", "lint", "static analysis", "code smell",
        ],
        priority: 9,
    },
    RoutingRule {
        role: AgentRole::DocsMaster,
        keywords: &[
            "document", "docs", "readme", "comment", "explain", "guide",
            "tutorial", "wiki", "changelog", "docstring",
        ],
        priority: 7,
    },
    RoutingRule {
        role: AgentRole::RepoOptimizer,
        keywords: &[
            "repo", "repository", "setup", "scaffold", "template", "boilerplate",
            "monorepo", "workspace", "cargo", "npm", "tooling", "linting",
        ],
        priority: 7,
    },
    RoutingRule {
        role: AgentRole::ApiDeveloper,
        keywords: &[
            "api", "rest", "graphql", "endpoint", "route", "request",
            "response", "swagger", "openapi", "grpc", "rpc",
        ],
        priority: 8,
    },
    RoutingRule {
        role: AgentRole::FullStackDeveloper,
        keywords: &[
            "fullstack", "full-stack", "frontend", "backend", "web", "react",
            "vue", "angular", "html", "css", "javascript", "typescript",
        ],
        priority: 6,
    },
    RoutingRule {
        role: AgentRole::RapidImplementer,
        keywords: &[
            "implement", "build", "create", "add", "feature", "code", "write",
            "develop", "generate", "make",
        ],
        priority: 5,
    },
];

// ── Scoring ───────────────────────────────────────────────────────────────────

/// Score a single role against the task description.
///
/// Returns the weighted keyword-match count plus the rule priority.
fn score_role(task_lower: &str, role: AgentRole) -> u32 {
    ROUTING_RULES
        .iter()
        .filter(|r| r.role == role)
        .map(|r| {
            let hits = r
                .keywords
                .iter()
                .filter(|&&kw| task_lower.contains(kw))
                .count() as u32;
            if hits > 0 { hits * 100 + r.priority } else { 0 }
        })
        .sum()
}

/// Score all candidate roles and return the one with the highest score.
///
/// Returns `None` when `candidates` is empty.
#[must_use]
pub fn score_and_select(task: &str, candidates: &[AgentRole]) -> Option<AgentRole> {
    let task_lower = task.to_lowercase();
    candidates
        .iter()
        .copied()
        .max_by_key(|&role| score_role(&task_lower, role))
}

/// Score all candidate roles and return them sorted best-first.
#[must_use]
pub fn ranked_candidates(task: &str, candidates: &[AgentRole]) -> Vec<(AgentRole, u32)> {
    let task_lower = task.to_lowercase();
    let mut scored: Vec<(AgentRole, u32)> = candidates
        .iter()
        .map(|&role| (role, score_role(&task_lower, role)))
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn debug_task_routes_to_debug_detective() {
        let all: Vec<AgentRole> = AgentRole::ALL.to_vec();
        let selected = score_and_select("debug the crash in the login flow", &all);
        assert_eq!(selected, Some(AgentRole::DebugDetective));
    }

    #[test]
    fn test_task_routes_to_testing_expert() {
        let all: Vec<AgentRole> = AgentRole::ALL.to_vec();
        let selected = score_and_select("write unit tests for the auth module", &all);
        assert_eq!(selected, Some(AgentRole::TestingExpert));
    }

    #[test]
    fn docker_task_routes_to_devops() {
        let all: Vec<AgentRole> = AgentRole::ALL.to_vec();
        let selected = score_and_select("create a docker-compose deploy pipeline", &all);
        assert_eq!(selected, Some(AgentRole::DevOpsInfra));
    }

    #[test]
    fn api_task_routes_to_api_developer() {
        let all: Vec<AgentRole> = AgentRole::ALL.to_vec();
        let selected = score_and_select("design a REST api endpoint with openapi spec", &all);
        assert_eq!(selected, Some(AgentRole::ApiDeveloper));
    }

    #[test]
    fn empty_candidates_returns_none() {
        assert_eq!(score_and_select("anything", &[]), None);
    }

    #[test]
    fn ranked_candidates_are_sorted_descending() {
        let all: Vec<AgentRole> = AgentRole::ALL.to_vec();
        let ranked = ranked_candidates("debug the crash", &all);
        assert_eq!(ranked.len(), all.len());
        // Scores are non-increasing.
        for w in ranked.windows(2) {
            assert!(w[0].1 >= w[1].1, "not sorted: {:?}", ranked);
        }
    }

    #[test]
    fn exclude_works_via_caller_filter() {
        // Simulate the caller filtering before calling score_and_select.
        let candidates: Vec<AgentRole> = AgentRole::ALL
            .iter()
            .copied()
            .filter(|&r| r != AgentRole::DebugDetective)
            .collect();
        let selected = score_and_select("debug the crash", &candidates);
        // DebugDetective excluded — some other role wins.
        assert!(selected.is_some());
        assert_ne!(selected, Some(AgentRole::DebugDetective));
    }

    #[test]
    fn generic_implement_task_routes_to_rapid_implementer() {
        // When no specialised keywords match, RapidImplementer should win.
        let all: Vec<AgentRole> = AgentRole::ALL.to_vec();
        let selected = score_and_select("implement a new feature", &all);
        assert_eq!(selected, Some(AgentRole::RapidImplementer));
    }
}
