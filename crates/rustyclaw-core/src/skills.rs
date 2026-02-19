use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::types::SkillOverride;

/// Parsed metadata from a SKILL.md frontmatter.
#[derive(Debug, Clone)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub requires_bins: Vec<String>,
    pub requires_env: Vec<String>,
}

/// A discovered skill with its metadata and instructions.
#[derive(Debug, Clone)]
pub struct Skill {
    pub meta: SkillMeta,
    pub instructions: String,
    pub source_dir: PathBuf,
}

/// Discover skills from multiple directories.
/// Later directories take precedence on name collisions.
pub fn discover_skills(dirs: &[&Path]) -> Vec<Skill> {
    let mut skills_map: HashMap<String, Skill> = HashMap::new();

    for dir in dirs {
        if !dir.exists() {
            continue;
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let skill_md = path.join("SKILL.md");
            if !skill_md.exists() {
                continue;
            }
            let content = match std::fs::read_to_string(&skill_md) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Some((meta, instructions)) = parse_skill_frontmatter(&content) {
                skills_map.insert(
                    meta.name.clone(),
                    Skill {
                        meta,
                        instructions,
                        source_dir: path,
                    },
                );
            }
        }
    }

    let mut skills: Vec<Skill> = skills_map.into_values().collect();
    skills.sort_by(|a, b| a.meta.name.cmp(&b.meta.name));
    skills
}

/// Parse YAML frontmatter from a SKILL.md file.
///
/// Expected format:
/// ```text
/// ---
/// name: github
/// description: Interact with GitHub
/// requires:
///   bins:
///     - gh
///   env:
///     - GITHUB_TOKEN
/// ---
///
/// # Instructions markdown here...
/// ```
pub fn parse_skill_frontmatter(content: &str) -> Option<(SkillMeta, String)> {
    let trimmed = content.trim();
    if !trimmed.starts_with("---") {
        return None;
    }

    // Find the closing ---
    let rest = &trimmed[3..];
    let end_idx = rest.find("\n---")?;
    let frontmatter = &rest[..end_idx];
    let body = rest[end_idx + 4..].trim();

    // Simple YAML parsing (no dependency on a YAML crate)
    let mut name = String::new();
    let mut description = String::new();
    let mut requires_bins = Vec::new();
    let mut requires_env = Vec::new();

    let mut in_bins = false;
    let mut in_env = false;

    for line in frontmatter.lines() {
        let stripped = line.trim();

        // Detect section changes
        if stripped.starts_with("name:") {
            name = stripped.trim_start_matches("name:").trim().to_string();
            in_bins = false;
            in_env = false;
            continue;
        }
        if stripped.starts_with("description:") {
            description = stripped
                .trim_start_matches("description:")
                .trim()
                .trim_matches('"')
                .to_string();
            in_bins = false;
            in_env = false;
            continue;
        }
        if stripped == "requires:" {
            in_bins = false;
            in_env = false;
            continue;
        }
        if stripped == "bins:" {
            in_bins = true;
            in_env = false;
            continue;
        }
        if stripped == "env:" {
            in_bins = false;
            in_env = true;
            continue;
        }

        // Collect list items
        if stripped.starts_with("- ") {
            let value = stripped.trim_start_matches("- ").trim().to_string();
            if in_bins {
                requires_bins.push(value);
            } else if in_env {
                requires_env.push(value);
            }
            continue;
        }

        // Any other key resets list context
        if stripped.contains(':') && !stripped.starts_with('-') {
            in_bins = false;
            in_env = false;
        }
    }

    if name.is_empty() {
        return None;
    }

    Some((
        SkillMeta {
            name,
            description,
            requires_bins,
            requires_env,
        },
        body.to_string(),
    ))
}

/// Check if a skill is eligible based on binary availability, env vars, and overrides.
pub fn is_skill_eligible(
    skill: &Skill,
    overrides: &HashMap<String, SkillOverride>,
) -> bool {
    // Check explicit override
    if let Some(ov) = overrides.get(&skill.meta.name) {
        if !ov.enabled {
            return false;
        }
    }

    // Check required binaries
    for bin in &skill.meta.requires_bins {
        if !check_bin_available(bin) {
            return false;
        }
    }

    // Check required env vars
    for var in &skill.meta.requires_env {
        if std::env::var(var).is_err() {
            return false;
        }
    }

    true
}

/// Check if a binary is available on PATH.
pub fn check_bin_available(bin: &str) -> bool {
    std::process::Command::new("which")
        .arg(bin)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Format eligible skills into a context string for injection.
pub fn format_skills_for_context(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut parts = Vec::new();
    for skill in skills {
        parts.push(format!(
            "### {} — {}\n\n{}",
            skill.meta.name, skill.meta.description, skill.instructions
        ));
    }

    parts.join("\n\n---\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_skill_frontmatter() {
        let content = r#"---
name: github
description: "Interact with GitHub using the gh CLI"
requires:
  bins:
    - gh
  env:
    - GITHUB_TOKEN
---

# GitHub Skill

Use `gh` to manage repos and PRs.
"#;
        let (meta, body) = parse_skill_frontmatter(content).unwrap();
        assert_eq!(meta.name, "github");
        assert_eq!(meta.description, "Interact with GitHub using the gh CLI");
        assert_eq!(meta.requires_bins, vec!["gh"]);
        assert_eq!(meta.requires_env, vec!["GITHUB_TOKEN"]);
        assert!(body.contains("GitHub Skill"));
    }

    #[test]
    fn test_parse_skill_no_requires() {
        let content = r#"---
name: notes
description: Take notes
---

Just a simple skill.
"#;
        let (meta, _body) = parse_skill_frontmatter(content).unwrap();
        assert_eq!(meta.name, "notes");
        assert!(meta.requires_bins.is_empty());
        assert!(meta.requires_env.is_empty());
    }

    #[test]
    fn test_parse_skill_no_frontmatter() {
        let content = "# Just markdown, no frontmatter";
        assert!(parse_skill_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_skill_no_name() {
        let content = "---\ndescription: missing name\n---\nbody";
        assert!(parse_skill_frontmatter(content).is_none());
    }

    #[test]
    fn test_discover_skills() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");

        // Create two skill directories
        let gh_dir = skills_dir.join("github");
        std::fs::create_dir_all(&gh_dir).unwrap();
        std::fs::write(
            gh_dir.join("SKILL.md"),
            "---\nname: github\ndescription: GitHub CLI\nrequires:\n  bins:\n    - gh\n---\nUse gh.",
        )
        .unwrap();

        let notes_dir = skills_dir.join("notes");
        std::fs::create_dir_all(&notes_dir).unwrap();
        std::fs::write(
            notes_dir.join("SKILL.md"),
            "---\nname: notes\ndescription: Note taking\n---\nTake notes.",
        )
        .unwrap();

        let skills = discover_skills(&[&skills_dir]);
        assert_eq!(skills.len(), 2);

        let names: Vec<&str> = skills.iter().map(|s| s.meta.name.as_str()).collect();
        assert!(names.contains(&"github"));
        assert!(names.contains(&"notes"));
    }

    #[test]
    fn test_discover_skills_precedence() {
        let tmp = TempDir::new().unwrap();
        let dir1 = tmp.path().join("project/skills");
        let dir2 = tmp.path().join("user/skills");

        // Same skill name in both directories
        let d1 = dir1.join("tool");
        std::fs::create_dir_all(&d1).unwrap();
        std::fs::write(
            d1.join("SKILL.md"),
            "---\nname: tool\ndescription: Project version\n---\nProject.",
        )
        .unwrap();

        let d2 = dir2.join("tool");
        std::fs::create_dir_all(&d2).unwrap();
        std::fs::write(
            d2.join("SKILL.md"),
            "---\nname: tool\ndescription: User version\n---\nUser.",
        )
        .unwrap();

        // dir2 is later, so it should win
        let skills = discover_skills(&[&dir1, &dir2]);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].meta.description, "User version");
    }

    #[test]
    fn test_skill_eligibility_disabled() {
        let skill = Skill {
            meta: SkillMeta {
                name: "test".to_string(),
                description: "test".to_string(),
                requires_bins: vec![],
                requires_env: vec![],
            },
            instructions: String::new(),
            source_dir: PathBuf::new(),
        };

        let mut overrides = HashMap::new();
        overrides.insert("test".to_string(), SkillOverride { enabled: false });

        assert!(!is_skill_eligible(&skill, &overrides));
    }

    #[test]
    fn test_skill_eligibility_missing_env() {
        let skill = Skill {
            meta: SkillMeta {
                name: "test".to_string(),
                description: "test".to_string(),
                requires_bins: vec![],
                requires_env: vec!["RUSTYCLAW_NONEXISTENT_VAR_12345".to_string()],
            },
            instructions: String::new(),
            source_dir: PathBuf::new(),
        };

        assert!(!is_skill_eligible(&skill, &HashMap::new()));
    }

    #[test]
    fn test_check_bin_available() {
        // "ls" should always be available on Linux
        assert!(check_bin_available("ls"));
        // This shouldn't exist
        assert!(!check_bin_available("rustyclaw_nonexistent_binary_12345"));
    }

    #[test]
    fn test_format_skills_empty() {
        assert!(format_skills_for_context(&[]).is_empty());
    }

    #[test]
    fn test_format_skills_nonempty() {
        let skills = vec![Skill {
            meta: SkillMeta {
                name: "github".to_string(),
                description: "GitHub CLI".to_string(),
                requires_bins: vec![],
                requires_env: vec![],
            },
            instructions: "Use gh to manage repos.".to_string(),
            source_dir: PathBuf::new(),
        }];

        let result = format_skills_for_context(&skills);
        assert!(result.contains("### github"));
        assert!(result.contains("GitHub CLI"));
        assert!(result.contains("Use gh to manage repos."));
    }
}
