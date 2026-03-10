//! Repo format detection and adapters.
//!
//! Different AI coding tools use different layouts for their plugin/skill repos.
//! This module detects the format and normalizes repo contents into
//! `SkillMetadata` + body pairs that feed into the skills system.

use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::types::{SkillMetadata, SkillRequirements, SkillSource};

// ── Plugin format enum ──────────────────────────────────────────────────────

/// Detected format of a plugin/skill repository.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginFormat {
    /// Native `SKILL.md` format (single or multi-skill repo).
    #[default]
    Skill,
    /// Claude Code plugin: `.claude-plugin/plugin.json` + `agents/`, `commands/`, `skills/` dirs.
    ClaudeCode,
    /// Codex plugin: `codex-plugin.json` or `.codex/plugin.json` (future).
    Codex,
    /// GsdOpenCode plugin: `.opencode/rules/gsd-*.md` rule files (gsd-opencode compatible).
    /// Only repos with at least one `gsd-*.md` file in `.opencode/rules/` are matched —
    /// native `opencode` projects (without the `gsd-` prefix) are left untouched.
    GsdOpenCode,
    /// Fallback: `.md` files treated as generic skill prompts.
    Generic,
}

impl std::fmt::Display for PluginFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skill => write!(f, "skill"),
            Self::ClaudeCode => write!(f, "claude_code"),
            Self::Codex => write!(f, "codex"),
            Self::GsdOpenCode => write!(f, "gsd_opencode"),
            Self::Generic => write!(f, "generic"),
        }
    }
}

// ── Plugin skill entry ──────────────────────────────────────────────────────

/// A single skill entry scanned from a non-SKILL.md repo, with extra metadata
/// beyond what `SkillMetadata` carries.
#[derive(Debug, Clone, Serialize)]
pub struct PluginSkillEntry {
    pub metadata: SkillMetadata,
    pub body: String,
    /// Human-friendly display name (e.g. "Code Reviewer" for `code-reviewer`).
    pub display_name: Option<String>,
    /// Plugin author (from plugin.json).
    pub author: Option<String>,
    /// Relative path of the source `.md` file within the repo (e.g. `agents/code-reviewer.md`).
    pub source_file: Option<String>,
}

// ── Format adapter trait ────────────────────────────────────────────────────

/// A format adapter normalizes a non-SKILL.md repo into skill entries.
pub trait FormatAdapter: Send + Sync {
    /// Check whether the given repo directory matches this format.
    fn detect(&self, repo_dir: &Path) -> bool;

    /// Scan the repo and return enriched entries for each skill found.
    fn scan_skills(&self, repo_dir: &Path) -> anyhow::Result<Vec<PluginSkillEntry>>;
}

// ── Claude Code adapter ─────────────────────────────────────────────────────

/// Claude Code plugin metadata from `.claude-plugin/plugin.json`.
#[derive(Debug, Deserialize)]
struct ClaudePluginJson {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    author: Option<PluginAuthor>,
}

/// Claude Code marketplace metadata from `.claude-plugin/marketplace.json`.
#[derive(Debug, Deserialize)]
struct ClaudeMarketplaceJson {
    #[serde(default)]
    plugins: Vec<ClaudeMarketplacePlugin>,
}

#[derive(Debug, Deserialize)]
struct ClaudeMarketplacePlugin {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    author: Option<PluginAuthor>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    skills: Vec<String>,
}

/// Author field can be a string or an object with `name` (and optionally `email`).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PluginAuthor {
    Simple(String),
    Object { name: String },
}

impl PluginAuthor {
    fn name(&self) -> &str {
        match self {
            Self::Simple(s) => s,
            Self::Object { name } => name,
        }
    }
}

/// Adapter for Claude Code plugin repos.
pub struct ClaudeCodeAdapter;

impl ClaudeCodeAdapter {
    fn slug_to_display_name(slug: &str) -> String {
        slug.split('-')
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    Some(first) => first.to_uppercase().to_string() + c.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Resolve `relative` against `base`, rejecting path traversal / absolute paths.
    fn resolve_relative_safe(base: &Path, relative: &str) -> Option<PathBuf> {
        let trimmed = relative.trim();
        let rel = if trimmed.is_empty() {
            "."
        } else {
            trimmed
        };
        let mut normalized = PathBuf::new();
        for component in Path::new(rel).components() {
            match component {
                Component::Normal(seg) => normalized.push(seg),
                Component::CurDir => {},
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
            }
        }
        if normalized.as_os_str().is_empty() {
            Some(base.to_path_buf())
        } else {
            Some(base.join(normalized))
        }
    }

    fn scan_marketplace_manifest(
        &self,
        repo_dir: &Path,
        seen_names: &mut HashSet<String>,
    ) -> anyhow::Result<Vec<PluginSkillEntry>> {
        let marketplace_json_path = repo_dir.join(".claude-plugin/marketplace.json");
        let marketplace: ClaudeMarketplaceJson =
            serde_json::from_str(&std::fs::read_to_string(&marketplace_json_path)?)?;

        let mut results = Vec::new();
        for plugin in marketplace.plugins {
            if plugin.name.trim().is_empty() {
                tracing::warn!(path = %marketplace_json_path.display(), "skipping marketplace plugin with empty name");
                continue;
            }
            let plugin_name = plugin.name;
            let plugin_description = plugin.description;
            let author = plugin.author.as_ref().map(|a| a.name().to_string());
            let source = plugin.source.unwrap_or_else(|| ".".to_string());
            let Some(plugin_base) = Self::resolve_relative_safe(repo_dir, &source) else {
                tracing::warn!(plugin = %plugin_name, source = %source, "skipping marketplace plugin with unsafe source path");
                continue;
            };
            if !plugin_base.is_dir() {
                tracing::debug!(plugin = %plugin_name, source = %source, "skipping marketplace plugin source that is not a directory");
                continue;
            }

            for skill_ref in plugin.skills {
                let Some(skill_base) = Self::resolve_relative_safe(&plugin_base, &skill_ref) else {
                    tracing::warn!(plugin = %plugin_name, skill = %skill_ref, "skipping marketplace skill with unsafe path");
                    continue;
                };

                let (skill_dir, skill_md) = if skill_base
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("SKILL.md"))
                    && skill_base.is_file()
                {
                    let Some(parent) = skill_base.parent() else {
                        continue;
                    };
                    (parent.to_path_buf(), skill_base)
                } else {
                    let skill_md = skill_base.join("SKILL.md");
                    (skill_base, skill_md)
                };

                if !skill_md.is_file() {
                    tracing::debug!(plugin = %plugin_name, skill = %skill_ref, path = %skill_md.display(), "marketplace skill path has no SKILL.md");
                    continue;
                }

                let raw = match std::fs::read_to_string(&skill_md) {
                    Ok(content) => content,
                    Err(e) => {
                        tracing::warn!(path = %skill_md.display(), %e, "failed to read marketplace SKILL.md");
                        continue;
                    },
                };

                let content = match crate::parse::parse_skill(&raw, &skill_dir) {
                    Ok(content) => content,
                    Err(e) => {
                        tracing::warn!(path = %skill_md.display(), %e, "failed to parse marketplace SKILL.md");
                        continue;
                    },
                };

                let slug = content.metadata.name.clone();
                let namespaced_name = format!("{plugin_name}:{slug}");
                if !seen_names.insert(namespaced_name.clone()) {
                    continue;
                }

                let source_file = skill_md
                    .strip_prefix(repo_dir)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());

                let mut meta = content.metadata;
                meta.name = namespaced_name;
                meta.description = if meta.description.is_empty() {
                    plugin_description.clone().unwrap_or_default()
                } else {
                    meta.description
                };
                if meta.homepage.is_none() {
                    meta.homepage = author.as_ref().map(|a| format!("https://github.com/{a}"));
                }
                meta.source = Some(SkillSource::Plugin);

                results.push(PluginSkillEntry {
                    metadata: meta,
                    body: content.body,
                    display_name: Some(Self::slug_to_display_name(&slug)),
                    author: author.clone(),
                    source_file,
                });
            }
        }

        Ok(results)
    }

    /// Scan a single plugin directory (one that has `.claude-plugin/plugin.json`).
    /// `repo_root` is the top-level repo directory; `source_file` paths are
    /// computed relative to it so that GitHub URLs work for marketplace repos.
    fn scan_single_plugin(
        &self,
        plugin_dir: &Path,
        repo_root: &Path,
    ) -> anyhow::Result<Vec<PluginSkillEntry>> {
        let plugin_json_path = plugin_dir.join(".claude-plugin/plugin.json");
        let plugin_json: ClaudePluginJson =
            serde_json::from_str(&std::fs::read_to_string(&plugin_json_path)?)?;

        let plugin_name = &plugin_json.name;
        let author = plugin_json.author.as_ref().map(|a| a.name().to_string());
        let mut results = Vec::new();

        // Scan agents/, commands/, skills/ directories for .md files.
        for subdir in &["agents", "commands", "skills"] {
            let dir = plugin_dir.join(subdir);
            if !dir.is_dir() {
                continue;
            }
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let ext = path.extension().and_then(|e| e.to_str());
                if ext != Some("md") {
                    continue;
                }
                let stem = match path.file_stem().and_then(|s| s.to_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };

                let body = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(?path, %e, "failed to read plugin skill file");
                        continue;
                    },
                };

                // Extract description from first non-empty line of body.
                let description = body
                    .lines()
                    .find(|l| {
                        let trimmed = l.trim();
                        !trimmed.is_empty() && !trimmed.starts_with('#')
                    })
                    .unwrap_or("")
                    .trim()
                    .chars()
                    .take(120)
                    .collect::<String>();

                let namespaced_name = format!("{plugin_name}:{stem}");

                // Build display name from stem: "code-reviewer" → "Code Reviewer"
                let display_name = Self::slug_to_display_name(&stem);

                // Relative path within repo root (e.g. "plugins/pr-review-toolkit/agents/code-reviewer.md")
                let source_file = path
                    .strip_prefix(repo_root)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());

                let meta = SkillMetadata {
                    name: namespaced_name,
                    description: if description.is_empty() {
                        plugin_json.description.clone().unwrap_or_default()
                    } else {
                        description
                    },
                    homepage: author.as_ref().map(|a| format!("https://github.com/{a}")),
                    license: None,
                    compatibility: None,
                    allowed_tools: Vec::new(),
                    requires: SkillRequirements::default(),
                    path: path.parent().unwrap_or(plugin_dir).to_path_buf(),
                    source: Some(SkillSource::Plugin),
                    dockerfile: None,
                };

                results.push(PluginSkillEntry {
                    metadata: meta,
                    body,
                    display_name: Some(display_name),
                    author: author.clone(),
                    source_file,
                });
            }
        }

        Ok(results)
    }
}

impl FormatAdapter for ClaudeCodeAdapter {
    fn detect(&self, repo_dir: &Path) -> bool {
        // Single plugin: .claude-plugin/plugin.json at root
        // Marketplace repo: .claude-plugin/marketplace.json at root
        repo_dir.join(".claude-plugin/plugin.json").is_file()
            || repo_dir.join(".claude-plugin/marketplace.json").is_file()
    }

    fn scan_skills(&self, repo_dir: &Path) -> anyhow::Result<Vec<PluginSkillEntry>> {
        // Single plugin case
        if repo_dir.join(".claude-plugin/plugin.json").is_file() {
            return self.scan_single_plugin(repo_dir, repo_dir);
        }

        let mut seen_names = HashSet::new();
        let mut results = if repo_dir.join(".claude-plugin/marketplace.json").is_file() {
            self.scan_marketplace_manifest(repo_dir, &mut seen_names)?
        } else {
            Vec::new()
        };

        // Marketplace repo: scan plugins/ and external_plugins/ subdirs
        for container in &["plugins", "external_plugins"] {
            let container_dir = repo_dir.join(container);
            if !container_dir.is_dir() {
                continue;
            }
            let entries = match std::fs::read_dir(&container_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                if !path.join(".claude-plugin/plugin.json").is_file() {
                    continue;
                }
                match self.scan_single_plugin(&path, repo_dir) {
                    Ok(skills) => {
                        for skill in skills {
                            if !seen_names.insert(skill.metadata.name.clone()) {
                                continue;
                            }
                            results.push(skill);
                        }
                    },
                    Err(e) => {
                        tracing::warn!(?path, %e, "failed to scan sub-plugin");
                    },
                }
            }
        }

        Ok(results)
    }
}

// ── GsdOpenCode adapter ──────────────────────────────────────────────────────

/// GsdOpenCode YAML frontmatter for `.opencode/rules/gsd-*.md` files.
#[derive(Debug, Default, Deserialize)]
struct GsdOpenCodeFrontmatter {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    /// When `true` the rule is always injected into context (alwaysApply).
    #[serde(rename = "alwaysApply", default)]
    always_apply: bool,
}

/// Adapter for gsd-opencode rule repos (gsd-opencode compatible).
///
/// Detects repos with `.opencode/rules/` that contain at least one `gsd-*.md`
/// file. This scoping prevents the adapter from claiming native `opencode.ai`
/// projects that also store rules under `.opencode/rules/` without the
/// `gsd-` prefix.
///
/// Once detected, all `*.md` files in `.opencode/rules/` are scanned.
pub struct GsdOpenCodeAdapter;

impl GsdOpenCodeAdapter {
    /// Convert a filename stem to a display name.
    ///
    /// Strips the `gsd-` namespace prefix if present, then splits on both
    /// `-` and `_` so that kebab-case (`gsd-rust-idioms` → "Rust Idioms") and
    /// snake_case (`rust_idioms` → "Rust Idioms") file names render correctly.
    fn slug_to_display_name(slug: &str) -> String {
        let slug = slug.strip_prefix("gsd-").unwrap_or(slug);
        slug.split(['-', '_'])
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    Some(first) => first.to_uppercase().to_string() + c.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Parse optional YAML frontmatter from a markdown file.
    ///
    /// Returns `(frontmatter, body)` where `body` is the markdown content
    /// without the frontmatter block.
    fn parse_frontmatter(content: &str) -> (GsdOpenCodeFrontmatter, &str) {
        let Some(rest) = content.strip_prefix("---") else {
            return (GsdOpenCodeFrontmatter::default(), content);
        };
        // Accept `---\r\n` or `---\n`
        let rest = rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n')).unwrap_or(rest);
        let Some(end) = rest.find("\n---") else {
            return (GsdOpenCodeFrontmatter::default(), content);
        };
        let yaml = &rest[..end];
        let after = &rest[end + 4..]; // skip "\n---"
        let body = after.strip_prefix("\r\n").or_else(|| after.strip_prefix('\n')).unwrap_or(after);
        let fm: GsdOpenCodeFrontmatter = serde_yaml::from_str(yaml).unwrap_or_default();
        (fm, body)
    }

    /// Scan a single directory for `*.md` rule files.
    fn scan_md_dir(
        &self,
        scan_dir: &Path,
        repo_dir: &Path,
        plugin_name: &str,
        results: &mut Vec<PluginSkillEntry>,
        seen_names: &mut HashSet<String>,
    ) {
        let entries = match std::fs::read_dir(scan_dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(?path, %e, "failed to read gsd-opencode rule file");
                    continue;
                },
            };
            let (fm, body) = Self::parse_frontmatter(&content);
            let skill_name = fm.name.unwrap_or_else(|| stem.clone());
            let namespaced_name = format!("{plugin_name}:{skill_name}");
            if !seen_names.insert(namespaced_name.clone()) {
                continue;
            }
            let description = fm.description.unwrap_or_else(|| {
                // Fall back to first non-empty, non-heading line of body.
                body.lines()
                    .find(|l| {
                        let t = l.trim();
                        !t.is_empty() && !t.starts_with('#')
                    })
                    .unwrap_or("")
                    .trim()
                    .chars()
                    .take(120)
                    .collect()
            });
            let source_file = path
                .strip_prefix(repo_dir)
                .ok()
                .map(|p| p.to_string_lossy().to_string());
            let meta = SkillMetadata {
                name: namespaced_name,
                description,
                homepage: None,
                license: None,
                compatibility: if fm.always_apply {
                    Some("alwaysApply".to_string())
                } else {
                    None
                },
                allowed_tools: Vec::new(),
                requires: SkillRequirements::default(),
                path: path.parent().unwrap_or(scan_dir).to_path_buf(),
                source: Some(SkillSource::Plugin),
                dockerfile: None,
            };
            results.push(PluginSkillEntry {
                metadata: meta,
                body: body.to_string(),
                display_name: Some(Self::slug_to_display_name(&stem)),
                author: None,
                source_file,
            });
        }
    }
}

impl FormatAdapter for GsdOpenCodeAdapter {
    fn detect(&self, repo_dir: &Path) -> bool {
        // Require `.opencode/rules/` directory with at least one `gsd-*.md` file.
        // This prevents claiming native `opencode.ai` project directories that also
        // use `.opencode/rules/` but do not carry the `gsd-` prefix.
        let rules_dir = repo_dir.join(".opencode").join("rules");
        if !rules_dir.is_dir() {
            return false;
        }
        std::fs::read_dir(&rules_dir)
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| {
                let name = entry.file_name();
                let s = name.to_string_lossy();
                s.starts_with("gsd-") && s.ends_with(".md")
            })
    }

    fn scan_skills(&self, repo_dir: &Path) -> anyhow::Result<Vec<PluginSkillEntry>> {
        let opencode_dir = repo_dir.join(".opencode");
        // Derive a plugin name from the repo directory name.
        let plugin_name = repo_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("gsd-opencode");

        let mut results = Vec::new();
        let mut seen_names = HashSet::new();

        // Only scan `.opencode/rules/`; no fallback to bare `.opencode/` to avoid
        // processing native opencode config files.
        let rules_dir = opencode_dir.join("rules");
        if rules_dir.is_dir() {
            self.scan_md_dir(&rules_dir, repo_dir, plugin_name, &mut results, &mut seen_names);
        }

        Ok(results)
    }
}

// ── Format detection ────────────────────────────────────────────────────────

/// All known format adapters, in detection priority order.
fn adapters() -> Vec<(PluginFormat, Box<dyn FormatAdapter>)> {
    vec![
        (PluginFormat::ClaudeCode, Box::new(ClaudeCodeAdapter)),
        (PluginFormat::GsdOpenCode, Box::new(GsdOpenCodeAdapter)),
    ]
}

/// Detect the format of a repository.
pub fn detect_format(repo_dir: &Path) -> PluginFormat {
    for (format, adapter) in adapters() {
        if adapter.detect(repo_dir) {
            return format;
        }
    }

    // Check for native SKILL.md.
    if repo_dir.join("SKILL.md").is_file() || has_skill_md_recursive(repo_dir) {
        return PluginFormat::Skill;
    }

    PluginFormat::Generic
}

/// Scan a repo using the detected format adapter.
/// Returns `None` for `Skill` format (caller should use existing SKILL.md scanning).
pub fn scan_with_adapter(
    repo_dir: &Path,
    format: PluginFormat,
) -> Option<anyhow::Result<Vec<PluginSkillEntry>>> {
    match format {
        PluginFormat::Skill => None, // handled by existing scan_repo_skills
        PluginFormat::ClaudeCode => Some(ClaudeCodeAdapter.scan_skills(repo_dir)),
        PluginFormat::Codex => None, // not yet implemented
        PluginFormat::GsdOpenCode => Some(GsdOpenCodeAdapter.scan_skills(repo_dir)),
        PluginFormat::Generic => None,
    }
}

/// Check if there's at least one SKILL.md in subdirectories.
fn has_skill_md_recursive(dir: &Path) -> bool {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.join("SKILL.md").is_file() {
                return true;
            }
            if has_skill_md_recursive(&path) {
                return true;
            }
        }
    }
    false
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_format_display() {
        assert_eq!(PluginFormat::Skill.to_string(), "skill");
        assert_eq!(PluginFormat::ClaudeCode.to_string(), "claude_code");
        assert_eq!(PluginFormat::Codex.to_string(), "codex");
        assert_eq!(PluginFormat::GsdOpenCode.to_string(), "gsd_opencode");
        assert_eq!(PluginFormat::Generic.to_string(), "generic");
    }

    #[test]
    fn test_detect_skill_format_root() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("SKILL.md"), "---\nname: test\n---\nbody").unwrap();
        assert_eq!(detect_format(tmp.path()), PluginFormat::Skill);
    }

    #[test]
    fn test_detect_skill_format_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("mysub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("SKILL.md"), "---\nname: test\n---\nbody").unwrap();
        assert_eq!(detect_format(tmp.path()), PluginFormat::Skill);
    }

    #[test]
    fn test_detect_claude_code_format() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join(".claude-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{"name":"test-plugin","description":"A test"}"#,
        )
        .unwrap();
        assert_eq!(detect_format(tmp.path()), PluginFormat::ClaudeCode);
    }

    #[test]
    fn test_detect_generic_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("README.md"), "hello").unwrap();
        assert_eq!(detect_format(tmp.path()), PluginFormat::Generic);
    }

    #[test]
    fn test_claude_code_adapter_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Create plugin structure
        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/plugin.json"),
            r#"{"name":"pr-review-toolkit","description":"PR review tools","author":"anthropics"}"#,
        )
        .unwrap();

        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::write(
            root.join("agents/code-reviewer.md"),
            "Use this agent when you need to review code.\n\nDetailed instructions here.",
        )
        .unwrap();

        std::fs::create_dir_all(root.join("commands")).unwrap();
        std::fs::write(
            root.join("commands/review-pr.md"),
            "# Review PR\n\nReview the current pull request.",
        )
        .unwrap();

        let adapter = ClaudeCodeAdapter;
        assert!(adapter.detect(root));

        let results = adapter.scan_skills(root).unwrap();
        assert_eq!(results.len(), 2);

        let names: Vec<&str> = results.iter().map(|e| e.metadata.name.as_str()).collect();
        assert!(names.contains(&"pr-review-toolkit:code-reviewer"));
        assert!(names.contains(&"pr-review-toolkit:review-pr"));

        // Check source is Plugin
        for entry in &results {
            assert_eq!(entry.metadata.source, Some(SkillSource::Plugin));
            assert_eq!(entry.author.as_deref(), Some("anthropics"));
        }

        // Check display_name and source_file
        let reviewer = results
            .iter()
            .find(|e| e.metadata.name == "pr-review-toolkit:code-reviewer")
            .unwrap();
        assert_eq!(reviewer.display_name.as_deref(), Some("Code Reviewer"));
        assert_eq!(
            reviewer.source_file.as_deref(),
            Some("agents/code-reviewer.md")
        );

        let review_pr = results
            .iter()
            .find(|e| e.metadata.name == "pr-review-toolkit:review-pr")
            .unwrap();
        assert_eq!(review_pr.display_name.as_deref(), Some("Review Pr"));
        assert_eq!(
            review_pr.source_file.as_deref(),
            Some("commands/review-pr.md")
        );
    }

    #[test]
    fn test_claude_code_adapter_empty_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/plugin.json"),
            r#"{"name":"empty-plugin"}"#,
        )
        .unwrap();

        let adapter = ClaudeCodeAdapter;
        let results = adapter.scan_skills(root).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_claude_code_adapter_skips_non_md() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/plugin.json"),
            r#"{"name":"test-plugin"}"#,
        )
        .unwrap();

        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::write(root.join("agents/readme.txt"), "not a skill").unwrap();
        std::fs::write(root.join("agents/real.md"), "A real skill agent.").unwrap();

        let adapter = ClaudeCodeAdapter;
        let results = adapter.scan_skills(root).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].metadata.name, "test-plugin:real");
    }

    #[test]
    fn test_detect_claude_code_marketplace_format() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join(".claude-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("marketplace.json"),
            r#"{"name":"marketplace","plugins":[]}"#,
        )
        .unwrap();
        assert_eq!(detect_format(tmp.path()), PluginFormat::ClaudeCode);
    }

    #[test]
    fn test_claude_code_marketplace_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // marketplace.json at root
        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/marketplace.json"),
            r#"{"name":"marketplace"}"#,
        )
        .unwrap();

        // Sub-plugin in plugins/
        let p1 = root.join("plugins/my-plugin");
        std::fs::create_dir_all(p1.join(".claude-plugin")).unwrap();
        std::fs::write(
            p1.join(".claude-plugin/plugin.json"),
            r#"{"name":"my-plugin","description":"A plugin"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(p1.join("commands")).unwrap();
        std::fs::write(p1.join("commands/do-thing.md"), "Do the thing.").unwrap();

        // Sub-plugin in external_plugins/
        let p2 = root.join("external_plugins/ext-plugin");
        std::fs::create_dir_all(p2.join(".claude-plugin")).unwrap();
        std::fs::write(
            p2.join(".claude-plugin/plugin.json"),
            r#"{"name":"ext-plugin"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(p2.join("agents")).unwrap();
        std::fs::write(p2.join("agents/helper.md"), "A helper agent.").unwrap();

        // Dir without plugin.json should be skipped
        let p3 = root.join("plugins/no-plugin");
        std::fs::create_dir_all(&p3).unwrap();
        std::fs::write(p3.join("README.md"), "no plugin").unwrap();

        let adapter = ClaudeCodeAdapter;
        assert!(adapter.detect(root));

        let results = adapter.scan_skills(root).unwrap();
        assert_eq!(results.len(), 2);

        let names: Vec<&str> = results.iter().map(|e| e.metadata.name.as_str()).collect();
        assert!(names.contains(&"my-plugin:do-thing"));
        assert!(names.contains(&"ext-plugin:helper"));

        // source_file should be relative to repo root, not the sub-plugin dir
        let do_thing = results
            .iter()
            .find(|e| e.metadata.name == "my-plugin:do-thing")
            .unwrap();
        assert_eq!(
            do_thing.source_file.as_deref(),
            Some("plugins/my-plugin/commands/do-thing.md")
        );

        let helper = results
            .iter()
            .find(|e| e.metadata.name == "ext-plugin:helper")
            .unwrap();
        assert_eq!(
            helper.source_file.as_deref(),
            Some("external_plugins/ext-plugin/agents/helper.md")
        );
    }

    #[test]
    fn test_claude_code_marketplace_scan_skills_array() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/marketplace.json"),
            r#"{
  "name": "anthropic-agent-skills",
  "plugins": [
    {
      "name": "document-skills",
      "description": "Document processing skills",
      "source": "./",
      "skills": ["./skills/xlsx", "./skills/pdf"]
    }
  ]
}"#,
        )
        .unwrap();

        std::fs::create_dir_all(root.join("skills/xlsx")).unwrap();
        std::fs::write(
            root.join("skills/xlsx/SKILL.md"),
            r#"---
name: xlsx
description: Work with spreadsheets
---

Read and write spreadsheets.
"#,
        )
        .unwrap();

        std::fs::create_dir_all(root.join("skills/pdf")).unwrap();
        std::fs::write(
            root.join("skills/pdf/SKILL.md"),
            r#"---
name: pdf
description: Work with PDF documents
---

Read and write PDF documents.
"#,
        )
        .unwrap();

        let adapter = ClaudeCodeAdapter;
        assert!(adapter.detect(root));

        let results = adapter.scan_skills(root).unwrap();
        assert_eq!(results.len(), 2);

        let xlsx = results
            .iter()
            .find(|e| e.metadata.name == "document-skills:xlsx")
            .unwrap();
        assert_eq!(xlsx.display_name.as_deref(), Some("Xlsx"));
        assert_eq!(xlsx.source_file.as_deref(), Some("skills/xlsx/SKILL.md"));
        assert_eq!(xlsx.metadata.source, Some(SkillSource::Plugin));
        assert_eq!(xlsx.metadata.path, root.join("skills/xlsx"));
        assert!(xlsx.body.contains("spreadsheets"));

        let pdf = results
            .iter()
            .find(|e| e.metadata.name == "document-skills:pdf")
            .unwrap();
        assert_eq!(pdf.source_file.as_deref(), Some("skills/pdf/SKILL.md"));
        assert_eq!(pdf.metadata.source, Some(SkillSource::Plugin));
        assert_eq!(pdf.metadata.path, root.join("skills/pdf"));
        assert!(pdf.body.contains("PDF"));
    }

    #[test]
    fn test_scan_with_adapter_skill_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(scan_with_adapter(tmp.path(), PluginFormat::Skill).is_none());
    }

    #[test]
    fn test_scan_with_adapter_claude_code() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/plugin.json"),
            r#"{"name":"my-plugin"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("skills")).unwrap();
        std::fs::write(root.join("skills/do-thing.md"), "Do the thing.").unwrap();

        let result = scan_with_adapter(root, PluginFormat::ClaudeCode);
        assert!(result.is_some());
        let skills = result.unwrap().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].metadata.name, "my-plugin:do-thing");
    }

    // ── GsdOpenCode adapter tests ─────────────────────────────────────────────

    #[test]
    fn test_plugin_format_gsd_opencode_display() {
        assert_eq!(PluginFormat::GsdOpenCode.to_string(), "gsd_opencode");
    }

    #[test]
    fn test_detect_gsd_opencode_format() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".opencode/rules")).unwrap();
        std::fs::write(tmp.path().join(".opencode/rules/gsd-my-rule.md"), "A rule.").unwrap();
        assert_eq!(detect_format(tmp.path()), PluginFormat::GsdOpenCode);
    }

    #[test]
    fn test_gsd_opencode_adapter_not_detected_without_gsd_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        // A native opencode project: has .opencode/rules/*.md but NO gsd-*.md files.
        // Must NOT be detected as GsdOpenCode — native opencode is left untouched.
        std::fs::create_dir_all(tmp.path().join(".opencode/rules")).unwrap();
        std::fs::write(tmp.path().join(".opencode/rules/my-rule.md"), "A rule.").unwrap();
        assert_ne!(detect_format(tmp.path()), PluginFormat::GsdOpenCode);
    }

    #[test]
    fn test_gsd_opencode_adapter_not_detected_without_rules_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // A bare .opencode/ directory (no rules/ subdir) must NOT be detected,
        // even when the file name has a gsd- prefix.
        std::fs::create_dir_all(tmp.path().join(".opencode")).unwrap();
        std::fs::write(tmp.path().join(".opencode/gsd-rule.md"), "A rule.").unwrap();
        assert_ne!(detect_format(tmp.path()), PluginFormat::GsdOpenCode);
    }

    #[test]
    fn test_gsd_opencode_adapter_not_detected_without_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("README.md"), "hello").unwrap();
        assert_ne!(detect_format(tmp.path()), PluginFormat::GsdOpenCode);
    }

    #[test]
    fn test_gsd_opencode_adapter_scan_rules_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join(".opencode/rules")).unwrap();
        // At least one gsd-*.md file is required for detection; both files are
        // scanned once the repo is confirmed as a gsd-opencode repo.
        std::fs::write(
            root.join(".opencode/rules/gsd-rust-idioms.md"),
            "---\nname: rust-idioms\ndescription: Rust idioms to follow\nalwaysApply: true\n---\n\nAlways prefer iterators over loops.",
        )
        .unwrap();
        std::fs::write(
            root.join(".opencode/rules/gsd-testing.md"),
            "---\ndescription: Testing guidelines\n---\n\nWrite tests for all public functions.",
        )
        .unwrap();

        let adapter = GsdOpenCodeAdapter;
        assert!(adapter.detect(root));

        let mut results = adapter.scan_skills(root).unwrap();
        results.sort_by(|a, b| a.metadata.name.cmp(&b.metadata.name));
        assert_eq!(results.len(), 2);

        let rust_idioms = results
            .iter()
            .find(|e| e.metadata.name.ends_with(":rust-idioms"))
            .unwrap();
        assert_eq!(rust_idioms.metadata.description, "Rust idioms to follow");
        assert_eq!(rust_idioms.metadata.compatibility.as_deref(), Some("alwaysApply"));
        assert!(rust_idioms.body.contains("prefer iterators"));
        assert_eq!(rust_idioms.display_name.as_deref(), Some("Rust Idioms"));
        assert_eq!(
            rust_idioms.source_file.as_deref(),
            Some(".opencode/rules/gsd-rust-idioms.md")
        );
        assert_eq!(rust_idioms.metadata.source, Some(SkillSource::Plugin));

        let testing = results
            .iter()
            .find(|e| e.metadata.name.ends_with(":gsd-testing"))
            .unwrap();
        assert_eq!(testing.metadata.description, "Testing guidelines");
        assert!(testing.metadata.compatibility.is_none());
        assert!(testing.body.contains("tests for all public functions"));
    }

    #[test]
    fn test_gsd_opencode_adapter_scans_all_md_when_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join(".opencode/rules")).unwrap();
        // gsd-trigger.md triggers detection; both files must be scanned.
        std::fs::write(root.join(".opencode/rules/gsd-trigger.md"), "Trigger.").unwrap();
        std::fs::write(root.join(".opencode/rules/extra-rule.md"), "Extra rule.").unwrap();

        let adapter = GsdOpenCodeAdapter;
        assert!(adapter.detect(root));

        let results = adapter.scan_skills(root).unwrap();
        assert_eq!(results.len(), 2, "all .md files should be scanned once detected");
    }

    #[test]
    fn test_gsd_opencode_adapter_no_fallback_to_bare_opencode_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // A directory with files directly in .opencode/ (no rules/ subdir) is a
        // native opencode config layout and must NOT be detected as GsdOpenCode.
        std::fs::create_dir_all(root.join(".opencode")).unwrap();
        std::fs::write(
            root.join(".opencode/commit-style.md"),
            "Use conventional commits.",
        )
        .unwrap();

        let adapter = GsdOpenCodeAdapter;
        assert!(!adapter.detect(root));
    }

    #[test]
    fn test_gsd_opencode_adapter_description_fallback_to_body() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join(".opencode/rules")).unwrap();
        // No frontmatter at all – description should come from first body line.
        std::fs::write(
            root.join(".opencode/rules/gsd-no-frontmatter.md"),
            "This is the first line used as description.\n\nMore content.",
        )
        .unwrap();

        let adapter = GsdOpenCodeAdapter;
        let results = adapter.scan_skills(root).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].metadata.description,
            "This is the first line used as description."
        );
    }

    #[test]
    fn test_gsd_opencode_adapter_skips_non_md_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join(".opencode/rules")).unwrap();
        std::fs::write(root.join(".opencode/rules/gsd-rule.md"), "A rule.").unwrap();
        std::fs::write(root.join(".opencode/rules/config.json"), "{}").unwrap();
        std::fs::write(root.join(".opencode/rules/readme.txt"), "notes").unwrap();

        let adapter = GsdOpenCodeAdapter;
        let results = adapter.scan_skills(root).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].metadata.name.ends_with(":gsd-rule"));
    }

    #[test]
    fn test_gsd_opencode_adapter_empty_rules_dir_not_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // An empty .opencode/rules/ has no gsd-*.md files → not detected.
        std::fs::create_dir_all(root.join(".opencode/rules")).unwrap();

        let adapter = GsdOpenCodeAdapter;
        assert!(!adapter.detect(root));
        let results = adapter.scan_skills(root).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_scan_with_adapter_gsd_opencode() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join(".opencode/rules")).unwrap();
        std::fs::write(root.join(".opencode/rules/gsd-style.md"), "Follow style guide.")
            .unwrap();

        let result = scan_with_adapter(root, PluginFormat::GsdOpenCode);
        assert!(result.is_some());
        let skills = result.unwrap().unwrap();
        assert_eq!(skills.len(), 1);
        assert!(skills[0].metadata.name.ends_with(":gsd-style"));
    }

    // ── Additional GsdOpenCode adapter tests ──────────────────────────────────

    #[test]
    fn test_gsd_opencode_slug_to_display_name_kebab() {
        // gsd- prefix is stripped before title-casing
        assert_eq!(GsdOpenCodeAdapter::slug_to_display_name("gsd-rust-idioms"), "Rust Idioms");
        assert_eq!(GsdOpenCodeAdapter::slug_to_display_name("gsd-commit-style"), "Commit Style");
        assert_eq!(GsdOpenCodeAdapter::slug_to_display_name("single"), "Single");
    }

    #[test]
    fn test_gsd_opencode_slug_to_display_name_snake_case() {
        // gsd-opencode repos sometimes use snake_case file names
        assert_eq!(GsdOpenCodeAdapter::slug_to_display_name("rust_idioms"), "Rust Idioms");
        assert_eq!(GsdOpenCodeAdapter::slug_to_display_name("code_review"), "Code Review");
    }

    #[test]
    fn test_gsd_opencode_slug_to_display_name_mixed() {
        assert_eq!(
            GsdOpenCodeAdapter::slug_to_display_name("my-rust_rules"),
            "My Rust Rules"
        );
    }

    #[test]
    fn test_gsd_opencode_adapter_crlf_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join(".opencode/rules")).unwrap();
        // Simulate a Windows-style CRLF line-ending frontmatter block
        let content =
            "---\r\nname: crlf-rule\r\ndescription: A CRLF rule\r\n---\r\n\r\nRule body here.";
        std::fs::write(root.join(".opencode/rules/gsd-crlf-rule.md"), content).unwrap();

        let adapter = GsdOpenCodeAdapter;
        let results = adapter.scan_skills(root).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].metadata.description, "A CRLF rule");
        assert!(results[0].body.contains("Rule body here."));
    }

    #[test]
    fn test_gsd_opencode_adapter_name_override_from_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join(".opencode/rules")).unwrap();
        // The frontmatter `name` overrides the filename stem in the skill identifier
        std::fs::write(
            root.join(".opencode/rules/gsd-file-stem.md"),
            "---\nname: overridden-name\ndescription: Name comes from frontmatter\n---\n\nBody.",
        )
        .unwrap();

        let adapter = GsdOpenCodeAdapter;
        let results = adapter.scan_skills(root).unwrap();
        assert_eq!(results.len(), 1);
        // Skill name should use the frontmatter `name`, not the file stem
        assert!(
            results[0].metadata.name.ends_with(":overridden-name"),
            "expected name ending with ':overridden-name', got '{}'",
            results[0].metadata.name
        );
        // Display name should still be derived from the file stem (gsd- prefix stripped)
        assert_eq!(results[0].display_name.as_deref(), Some("File Stem"));
    }

    #[test]
    fn test_gsd_opencode_adapter_duplicate_name_deduplication() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join(".opencode/rules")).unwrap();
        // Two files with the same `name` in frontmatter — second should be skipped
        std::fs::write(
            root.join(".opencode/rules/gsd-rule-a.md"),
            "---\nname: shared-name\n---\n\nFirst rule.",
        )
        .unwrap();
        std::fs::write(
            root.join(".opencode/rules/gsd-rule-b.md"),
            "---\nname: shared-name\n---\n\nSecond rule (duplicate).",
        )
        .unwrap();

        let adapter = GsdOpenCodeAdapter;
        let results = adapter.scan_skills(root).unwrap();
        // Only one entry should survive deduplication
        assert_eq!(results.len(), 1);
        assert!(results[0].metadata.name.ends_with(":shared-name"));
    }
}
