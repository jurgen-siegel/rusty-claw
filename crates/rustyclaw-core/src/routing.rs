use std::collections::{HashMap, HashSet};

use regex::Regex;

use crate::smart_routing;
use crate::types::{AgentConfig, RoutingResult, TeamConfig, TeamContext, TeammateMention};

/// Find the first team that contains the given agent.
pub fn find_team_for_agent(
    agent_id: &str,
    teams: &HashMap<String, TeamConfig>,
) -> Option<TeamContext> {
    for (team_id, team) in teams {
        if team.agents.iter().any(|a| a == agent_id) {
            return Some(TeamContext {
                team_id: team_id.clone(),
                team: team.clone(),
            });
        }
    }
    None
}

/// Check if a mentioned ID is a valid teammate of the current agent in the given team.
pub fn is_teammate(
    mentioned_id: &str,
    current_agent_id: &str,
    team_id: &str,
    teams: &HashMap<String, TeamConfig>,
    agents: &HashMap<String, AgentConfig>,
) -> bool {
    let Some(team) = teams.get(team_id) else {
        return false;
    };
    mentioned_id != current_agent_id
        && team.agents.iter().any(|a| a == mentioned_id)
        && agents.contains_key(mentioned_id)
}

/// Extract teammate mentions from a response text.
/// Parses tags like `[@agent_id: message]` or `[@agent1,agent2: message]`.
/// Returns a list of (teammate_id, full_message) pairs with shared context prepended.
pub fn extract_teammate_mentions(
    response: &str,
    current_agent_id: &str,
    team_id: &str,
    teams: &HashMap<String, TeamConfig>,
    agents: &HashMap<String, AgentConfig>,
) -> Vec<TeammateMention> {
    let mut results = Vec::new();
    let mut seen = HashSet::new();

    let tag_re = Regex::new(r"\[@(\S+?):\s*([\s\S]*?)\]").unwrap();

    for caps in tag_re.captures_iter(response) {
        // Strip all tags from the response to get shared context
        let shared_context = tag_re.replace_all(response, "").trim().to_string();
        let direct_message = caps[2].trim().to_string();
        let full_message = if !shared_context.is_empty() {
            format!(
                "{}\n\n------\n\nDirected to you:\n{}",
                shared_context, direct_message
            )
        } else {
            direct_message
        };

        // Support comma-separated agent IDs: [@coder,reviewer: message]
        let candidate_ids: Vec<String> = caps[1]
            .to_lowercase()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        for candidate_id in candidate_ids {
            if !seen.contains(&candidate_id)
                && is_teammate(&candidate_id, current_agent_id, team_id, teams, agents)
            {
                results.push(TeammateMention {
                    teammate_id: candidate_id.clone(),
                    message: full_message.clone(),
                });
                seen.insert(candidate_id);
            }
        }
    }

    results
}

/// Extract cross-team mentions from a response text.
/// Parses tags like `[@!agent_id: message]` — the `!` prefix means "any agent, not just teammates."
/// Returns a list of (target_agent_id, message) pairs, excluding self and agents already
/// mentioned as teammates.
pub fn extract_cross_team_mentions(
    response: &str,
    current_agent_id: &str,
    agents: &HashMap<String, AgentConfig>,
    already_mentioned: &HashSet<String>,
) -> Vec<TeammateMention> {
    let mut results = Vec::new();
    let mut seen: HashSet<String> = already_mentioned.clone();

    let tag_re = Regex::new(r"\[@!(\S+?):\s*([\s\S]*?)\]").unwrap();
    // Also strip normal teammate tags for shared context
    let all_tags_re = Regex::new(r"\[@!?\S+?:\s*[\s\S]*?\]").unwrap();

    for caps in tag_re.captures_iter(response) {
        let shared_context = all_tags_re.replace_all(response, "").trim().to_string();
        let direct_message = caps[2].trim().to_string();
        let full_message = if !shared_context.is_empty() {
            format!(
                "{}\n\n------\n\nDirected to you:\n{}",
                shared_context, direct_message
            )
        } else {
            direct_message
        };

        let candidate_ids: Vec<String> = caps[1]
            .to_lowercase()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        for candidate_id in candidate_ids {
            if candidate_id == current_agent_id {
                continue;
            }
            if seen.contains(&candidate_id) {
                continue;
            }
            if !agents.contains_key(&candidate_id) {
                continue;
            }
            // Check if the originating agent has cross_team_handoffs enabled
            if let Some(agent_config) = agents.get(current_agent_id) {
                if !agent_config.cross_team_handoffs {
                    continue;
                }
            }
            results.push(TeammateMention {
                teammate_id: candidate_id.clone(),
                message: full_message.clone(),
            });
            seen.insert(candidate_id);
        }
    }

    results
}

/// Extract all agent mentions from a response, regardless of team membership.
/// Matches both `[@agent: msg]` and `[@!agent: msg]` bracket syntax.
/// Used when there's no team context — any known agent is a valid handoff target.
pub fn extract_all_agent_mentions(
    response: &str,
    current_agent_id: &str,
    agents: &HashMap<String, AgentConfig>,
    already_mentioned: &HashSet<String>,
) -> Vec<TeammateMention> {
    let mut results = Vec::new();
    let mut seen: HashSet<String> = already_mentioned.clone();

    // Match both [@agent: msg] and [@!agent: msg]
    let tag_re = Regex::new(r"\[@!?(\S+?):\s*([\s\S]*?)\]").unwrap();

    for caps in tag_re.captures_iter(response) {
        let shared_context = tag_re.replace_all(response, "").trim().to_string();
        let direct_message = caps[2].trim().to_string();
        let full_message = if !shared_context.is_empty() {
            format!(
                "{}\n\n------\n\nDirected to you:\n{}",
                shared_context, direct_message
            )
        } else {
            direct_message
        };

        let candidate_ids: Vec<String> = caps[1]
            .to_lowercase()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        for candidate_id in candidate_ids {
            if candidate_id == current_agent_id {
                continue;
            }
            if seen.contains(&candidate_id) {
                continue;
            }
            if !agents.contains_key(&candidate_id) {
                continue;
            }
            results.push(TeammateMention {
                teammate_id: candidate_id.clone(),
                message: full_message.clone(),
            });
            seen.insert(candidate_id);
        }
    }

    results
}

/// Extract natural @agent handoff mentions from a response.
/// Matches bare `@agent_id:` or `@agent_id —` patterns at the start of a line
/// (without the bracket syntax). This is a fallback for when LLMs use natural
/// language addressing instead of the `[@agent: msg]` bracket syntax.
///
/// Skips agents already captured by bracket-syntax extraction.
pub fn extract_natural_handoffs(
    response: &str,
    current_agent_id: &str,
    agents: &HashMap<String, AgentConfig>,
    already_mentioned: &HashSet<String>,
) -> Vec<TeammateMention> {
    let mut results = Vec::new();
    let mut seen: HashSet<String> = already_mentioned.clone();

    // Match @agent_id at the start of a line, followed by colon, em-dash, en-dash, or hyphen.
    // Handles markdown-wrapped mentions like **@review** or *@review* (LLMs love to bold these).
    let re = Regex::new(r"(?m)^[*_]{0,2}@([\w-]+)[*_]{0,2}\s*[:\u{2014}\u{2013}\-]+[*_]{0,2}\s*")
        .unwrap();

    // Collect all match positions and agent IDs
    let matches: Vec<_> = re.find_iter(response).collect();
    let captures: Vec<_> = re.captures_iter(response).collect();

    if matches.is_empty() {
        return results;
    }

    // Group text segments by agent ID
    let mut agent_texts: HashMap<String, Vec<String>> = HashMap::new();

    for (i, caps) in captures.iter().enumerate() {
        let raw_id = &caps[1];
        // Strip trailing punctuation (commas, periods, semicolons)
        let agent_id = raw_id
            .trim_end_matches(|c: char| c == ',' || c == ';' || c == '.')
            .to_lowercase();

        if agent_id == current_agent_id {
            continue;
        }
        if !agents.contains_key(&agent_id) {
            continue;
        }

        // Capture text from end of this match to start of next match (or end of string)
        let start = matches[i].end();
        let end = if i + 1 < matches.len() {
            matches[i + 1].start()
        } else {
            response.len()
        };

        let text = response[start..end].trim().to_string();
        if !text.is_empty() {
            agent_texts.entry(agent_id).or_default().push(text);
        }
    }

    // Build results, deduplicating against already-mentioned agents
    for (agent_id, texts) in agent_texts {
        if seen.contains(&agent_id) {
            continue;
        }
        let message = texts.join("\n\n");
        results.push(TeammateMention {
            teammate_id: agent_id.clone(),
            message,
        });
        seen.insert(agent_id);
    }

    results
}

/// Get the reset flag path for a specific agent.
pub fn get_agent_reset_flag(agent_id: &str, workspace_path: &std::path::Path) -> std::path::PathBuf {
    workspace_path.join(agent_id).join("reset_flag")
}

/// Detect if message starts with multiple @agent mentions (for parallel dispatch).
/// Only considers @mentions that appear as a prefix — before any non-@ text.
/// e.g. "@coder @tester fix this" → ["coder", "tester"]
/// e.g. "@coder do X then pass to @review" → ["coder"] (review is inline, not a target)
/// Returns empty if all prefix agents are in the same team (team chain handles that).
pub fn detect_multiple_agents(
    message: &str,
    agents: &HashMap<String, AgentConfig>,
    teams: &HashMap<String, TeamConfig>,
) -> Vec<String> {
    let mut valid_agents: Vec<String> = Vec::new();

    // Only consider @mentions at the start of the message (prefix tokens)
    for token in message.split_whitespace() {
        if let Some(id) = token.strip_prefix('@') {
            let agent_id = id.to_lowercase();
            if agents.contains_key(&agent_id) && !valid_agents.contains(&agent_id) {
                valid_agents.push(agent_id);
            }
        } else {
            break; // Stop at first non-@ token
        }
    }

    // If multiple agents are all in the same team, don't trigger multi-dispatch
    if valid_agents.len() > 1 {
        for team in teams.values() {
            if valid_agents.iter().all(|a| team.agents.contains(a)) {
                return Vec::new(); // Same team — chain will handle collaboration
            }
        }
    }

    valid_agents
}

/// Parse @agent_id or @team_id prefix from a message.
/// Returns RoutingResult with agent_id, stripped message, and isTeam flag.
/// Returns agent_id "error" with easter egg message if multiple agents detected across teams.
pub fn parse_agent_routing(
    raw_message: &str,
    agents: &HashMap<String, AgentConfig>,
    teams: &HashMap<String, TeamConfig>,
) -> RoutingResult {
    // Multi-agent dispatch: route to multiple agents in parallel
    let mentioned_agents = detect_multiple_agents(raw_message, agents, teams);
    if mentioned_agents.len() > 1 {
        // Strip only the leading @agent prefix tokens, keep inline @mentions intact
        let tokens: Vec<&str> = raw_message.split_whitespace().collect();
        let first_non_at = tokens
            .iter()
            .position(|t| !t.starts_with('@'))
            .unwrap_or(tokens.len());
        let message = tokens[first_non_at..].join(" ");
        let message = if message.is_empty() {
            raw_message.to_string()
        } else {
            message
        };
        return RoutingResult {
            agent_id: mentioned_agents[0].clone(),
            message,
            is_team: false,
            multi_agents: mentioned_agents,
        };
    }

    // Match @prefix pattern
    let prefix_re = Regex::new(r"^@(\S+)\s+([\s\S]*)$").unwrap();
    if let Some(caps) = prefix_re.captures(raw_message) {
        let candidate_id = caps[1].to_lowercase();
        let message = caps[2].to_string();

        // Check agent IDs
        if agents.contains_key(&candidate_id) {
            return RoutingResult {
                agent_id: candidate_id,
                message,
                is_team: false,
                multi_agents: Vec::new(),
            };
        }

        // Check team IDs — resolve to leader agent
        if let Some(team) = teams.get(&candidate_id) {
            return RoutingResult {
                agent_id: team.leader_agent.clone(),
                message,
                is_team: true,
                multi_agents: Vec::new(),
            };
        }

        // Match by agent name (case-insensitive)
        for (id, config) in agents {
            if config.name.to_lowercase() == candidate_id {
                return RoutingResult {
                    agent_id: id.clone(),
                    message,
                    is_team: false,
                    multi_agents: Vec::new(),
                };
            }
        }

        // Match by team name (case-insensitive)
        for config in teams.values() {
            if config.name.to_lowercase() == candidate_id {
                return RoutingResult {
                    agent_id: config.leader_agent.clone(),
                    message,
                    is_team: true,
                    multi_agents: Vec::new(),
                };
            }
        }
    }

    // Smart routing: try to match agent by message content keywords
    if let Some(matched_agent) = smart_routing::match_agent_by_content(raw_message, agents) {
        return RoutingResult {
            agent_id: matched_agent,
            message: raw_message.to_string(),
            is_team: false,
            multi_agents: Vec::new(),
        };
    }

    // Default: no routing prefix
    RoutingResult {
        agent_id: "default".to_string(),
        message: raw_message.to_string(),
        is_team: false,
        multi_agents: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agents() -> HashMap<String, AgentConfig> {
        let mut agents = HashMap::new();
        agents.insert(
            "coder".to_string(),
            AgentConfig {
                name: "Coder".to_string(),
                provider: "anthropic".to_string(),
                model: "sonnet".to_string(),
                working_directory: "/tmp/coder".to_string(),
                reset_policy: String::new(),
                reset_hour: None,
                idle_timeout_minutes: None,
                context_window: None,
                fallbacks: None,
                cross_team_handoffs: true,
                route_patterns: None,
                route_priority: 0,
            },
        );
        agents.insert(
            "reviewer".to_string(),
            AgentConfig {
                name: "Reviewer".to_string(),
                provider: "anthropic".to_string(),
                model: "opus".to_string(),
                working_directory: "/tmp/reviewer".to_string(),
                reset_policy: String::new(),
                reset_hour: None,
                idle_timeout_minutes: None,
                context_window: None,
                fallbacks: None,
                cross_team_handoffs: true,
                route_patterns: None,
                route_priority: 0,
            },
        );
        agents.insert(
            "tester".to_string(),
            AgentConfig {
                name: "Tester".to_string(),
                provider: "openai".to_string(),
                model: "gpt-5.3-codex".to_string(),
                working_directory: "/tmp/tester".to_string(),
                reset_policy: String::new(),
                reset_hour: None,
                idle_timeout_minutes: None,
                context_window: None,
                fallbacks: None,
                cross_team_handoffs: true,
                route_patterns: None,
                route_priority: 0,
            },
        );
        agents
    }

    fn sample_teams() -> HashMap<String, TeamConfig> {
        let mut teams = HashMap::new();
        teams.insert(
            "dev".to_string(),
            TeamConfig {
                name: "Development Team".to_string(),
                agents: vec![
                    "coder".to_string(),
                    "reviewer".to_string(),
                ],
                leader_agent: "coder".to_string(),
                description: None,
            },
        );
        teams
    }

    #[test]
    fn test_parse_agent_routing_at_mention() {
        let agents = sample_agents();
        let teams = HashMap::new();
        let result = parse_agent_routing("@coder fix the bug", &agents, &teams);
        assert_eq!(result.agent_id, "coder");
        assert_eq!(result.message, "fix the bug");
        assert!(!result.is_team);
    }

    #[test]
    fn test_parse_agent_routing_team_mention() {
        let agents = sample_agents();
        let teams = sample_teams();
        let result = parse_agent_routing("@dev fix the auth bug", &agents, &teams);
        assert_eq!(result.agent_id, "coder"); // leader
        assert_eq!(result.message, "fix the auth bug");
        assert!(result.is_team);
    }

    #[test]
    fn test_parse_agent_routing_default() {
        let agents = sample_agents();
        let teams = HashMap::new();
        let result = parse_agent_routing("hello world", &agents, &teams);
        assert_eq!(result.agent_id, "default");
        assert_eq!(result.message, "hello world");
    }

    #[test]
    fn test_parse_agent_routing_by_name() {
        let agents = sample_agents();
        let teams = HashMap::new();
        let result = parse_agent_routing("@Coder fix it", &agents, &teams);
        assert_eq!(result.agent_id, "coder");
        assert_eq!(result.message, "fix it");
    }

    #[test]
    fn test_parse_agent_routing_unknown_prefix() {
        let agents = sample_agents();
        let teams = HashMap::new();
        let result = parse_agent_routing("@unknown do something", &agents, &teams);
        assert_eq!(result.agent_id, "default");
    }

    #[test]
    fn test_find_team_for_agent() {
        let teams = sample_teams();
        let ctx = find_team_for_agent("coder", &teams);
        assert!(ctx.is_some());
        assert_eq!(ctx.unwrap().team_id, "dev");
    }

    #[test]
    fn test_find_team_for_agent_not_found() {
        let teams = sample_teams();
        assert!(find_team_for_agent("tester", &teams).is_none());
    }

    #[test]
    fn test_is_teammate() {
        let agents = sample_agents();
        let teams = sample_teams();
        assert!(is_teammate("reviewer", "coder", "dev", &teams, &agents));
        assert!(!is_teammate("coder", "coder", "dev", &teams, &agents)); // self
        assert!(!is_teammate("tester", "coder", "dev", &teams, &agents)); // not in team
    }

    #[test]
    fn test_extract_teammate_mentions_single() {
        let agents = sample_agents();
        let teams = sample_teams();
        let mentions = extract_teammate_mentions(
            "Done with the fix. [@reviewer: please check my changes]",
            "coder",
            "dev",
            &teams,
            &agents,
        );
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].teammate_id, "reviewer");
        assert!(mentions[0].message.contains("please check my changes"));
        assert!(mentions[0].message.contains("Done with the fix."));
    }

    #[test]
    fn test_extract_teammate_mentions_comma_separated() {
        let agents = sample_agents();
        let teams = sample_teams();

        // From coder's perspective, mention reviewer via comma-separated syntax
        let mentions = extract_teammate_mentions(
            "[@reviewer: please review this code]",
            "coder",
            "dev",
            &teams,
            &agents,
        );
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].teammate_id, "reviewer");

        // Test actual comma-separated: coder mentions reviewer (self-mention is excluded)
        let mentions = extract_teammate_mentions(
            "[@coder,reviewer: status update please]",
            "coder",
            "dev",
            &teams,
            &agents,
        );
        // coder is self (excluded), reviewer is teammate
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].teammate_id, "reviewer");
    }

    #[test]
    fn test_extract_teammate_mentions_no_duplicates() {
        let agents = sample_agents();
        let teams = sample_teams();
        let mentions = extract_teammate_mentions(
            "[@reviewer: first task] [@reviewer: second task]",
            "coder",
            "dev",
            &teams,
            &agents,
        );
        // Should deduplicate — only one mention per teammate
        assert_eq!(mentions.len(), 1);
    }

    #[test]
    fn test_detect_multiple_agents_same_team() {
        let agents = sample_agents();
        let teams = sample_teams();
        let result = detect_multiple_agents("@coder @reviewer fix this", &agents, &teams);
        assert!(result.is_empty()); // Same team, no easter egg
    }

    #[test]
    fn test_detect_multiple_agents_cross_team() {
        let agents = sample_agents();
        let teams = sample_teams();
        // tester is not in any team with coder
        let result = detect_multiple_agents("@coder @tester fix this", &agents, &teams);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_multi_dispatch_multiple_agents() {
        let agents = sample_agents();
        let teams = sample_teams();
        let result = parse_agent_routing("@coder @tester fix everything", &agents, &teams);
        assert_eq!(result.multi_agents.len(), 2);
        assert!(result.multi_agents.contains(&"coder".to_string()));
        assert!(result.multi_agents.contains(&"tester".to_string()));
        assert_eq!(result.message, "fix everything");
    }

    #[test]
    fn test_multi_dispatch_same_team_no_dispatch() {
        let agents = sample_agents();
        let teams = sample_teams();
        // coder and reviewer are in same team — should route to first (prefix), not multi-dispatch
        let result = parse_agent_routing("@coder @reviewer fix this", &agents, &teams);
        assert!(result.multi_agents.is_empty());
        assert_eq!(result.agent_id, "coder");
    }

    #[test]
    fn test_inline_mention_no_multi_dispatch() {
        let agents = sample_agents();
        let teams = sample_teams();
        // @tester appears inline (after non-@ text), not as a prefix dispatch target
        let result =
            parse_agent_routing("@coder create a todo app then pass to @tester", &agents, &teams);
        assert!(result.multi_agents.is_empty());
        assert_eq!(result.agent_id, "coder");
        assert!(result.message.contains("pass to @tester")); // inline mention preserved
    }

    #[test]
    fn test_inline_mentions_complex_pipeline() {
        let agents = sample_agents();
        let teams = sample_teams();
        // Only @coder is a prefix mention; @reviewer and @tester are inline handoff targets
        let result = parse_agent_routing(
            "@coder write code then @reviewer reviews it then @tester tests it",
            &agents,
            &teams,
        );
        assert!(result.multi_agents.is_empty());
        assert_eq!(result.agent_id, "coder");
        assert!(result.message.contains("@reviewer"));
        assert!(result.message.contains("@tester"));
    }

    #[test]
    fn test_extract_cross_team_mentions() {
        let agents = sample_agents();
        let already: HashSet<String> = HashSet::new();
        let mentions = extract_cross_team_mentions(
            "Done. [@!tester: please run the test suite]",
            "coder",
            &agents,
            &already,
        );
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].teammate_id, "tester");
        assert!(mentions[0].message.contains("please run the test suite"));
    }

    #[test]
    fn test_cross_team_skips_self() {
        let agents = sample_agents();
        let already: HashSet<String> = HashSet::new();
        let mentions = extract_cross_team_mentions(
            "[@!coder: talk to myself]",
            "coder",
            &agents,
            &already,
        );
        assert!(mentions.is_empty());
    }

    #[test]
    fn test_cross_team_skips_already_mentioned() {
        let agents = sample_agents();
        let mut already: HashSet<String> = HashSet::new();
        already.insert("tester".to_string());
        let mentions = extract_cross_team_mentions(
            "[@!tester: duplicate]",
            "coder",
            &agents,
            &already,
        );
        assert!(mentions.is_empty());
    }

    #[test]
    fn test_cross_team_disabled() {
        let mut agents = sample_agents();
        agents.get_mut("coder").unwrap().cross_team_handoffs = false;
        let already: HashSet<String> = HashSet::new();
        let mentions = extract_cross_team_mentions(
            "[@!tester: please test]",
            "coder",
            &agents,
            &already,
        );
        assert!(mentions.is_empty());
    }

    #[test]
    fn test_smart_routing_keyword_match() {
        let mut agents = sample_agents();
        agents.get_mut("coder").unwrap().route_patterns =
            Some(vec!["code".to_string(), "bug".to_string(), "fix".to_string()]);
        let teams = HashMap::new();
        let result = parse_agent_routing("please fix the bug", &agents, &teams);
        assert_eq!(result.agent_id, "coder");
    }

    #[test]
    fn test_smart_routing_no_match_defaults() {
        let agents = sample_agents();
        let teams = HashMap::new();
        // No route_patterns configured, should default
        let result = parse_agent_routing("hello world", &agents, &teams);
        assert_eq!(result.agent_id, "default");
    }

    #[test]
    fn test_natural_handoff_colon() {
        let agents = sample_agents();
        let already: HashSet<String> = HashSet::new();
        let mentions = extract_natural_handoffs(
            "Done with my work.\n@reviewer: please check the code for bugs",
            "coder",
            &agents,
            &already,
        );
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].teammate_id, "reviewer");
        assert!(mentions[0].message.contains("please check the code"));
    }

    #[test]
    fn test_natural_handoff_em_dash() {
        let agents = sample_agents();
        let already: HashSet<String> = HashSet::new();
        let mentions = extract_natural_handoffs(
            "@reviewer \u{2014} your turn. Here's the code.",
            "coder",
            &agents,
            &already,
        );
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].teammate_id, "reviewer");
        assert!(mentions[0].message.contains("your turn"));
    }

    #[test]
    fn test_natural_handoff_skips_inline() {
        let agents = sample_agents();
        let already: HashSet<String> = HashSet::new();
        // @reviewer NOT at start of line — should not match
        let mentions = extract_natural_handoffs(
            "I asked @reviewer about this already.",
            "coder",
            &agents,
            &already,
        );
        assert!(mentions.is_empty());
    }

    #[test]
    fn test_natural_handoff_skips_already_mentioned() {
        let agents = sample_agents();
        let mut already: HashSet<String> = HashSet::new();
        already.insert("reviewer".to_string());
        let mentions = extract_natural_handoffs(
            "@reviewer: please check this",
            "coder",
            &agents,
            &already,
        );
        assert!(mentions.is_empty());
    }

    #[test]
    fn test_natural_handoff_multiple_agents() {
        let agents = sample_agents();
        let already: HashSet<String> = HashSet::new();
        let mentions = extract_natural_handoffs(
            "@reviewer: check the code\n\n@tester: run the test suite",
            "coder",
            &agents,
            &already,
        );
        assert_eq!(mentions.len(), 2);
        let ids: HashSet<String> = mentions.iter().map(|m| m.teammate_id.clone()).collect();
        assert!(ids.contains("reviewer"));
        assert!(ids.contains("tester"));
    }

    #[test]
    fn test_natural_handoff_merges_duplicate_agent() {
        let agents = sample_agents();
        let already: HashSet<String> = HashSet::new();
        let mentions = extract_natural_handoffs(
            "@reviewer: first task\n@reviewer: second task",
            "coder",
            &agents,
            &already,
        );
        // Should be deduplicated — one mention with merged text
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].teammate_id, "reviewer");
    }

    #[test]
    fn test_natural_handoff_markdown_bold() {
        let agents = sample_agents();
        let already: HashSet<String> = HashSet::new();
        let mentions = extract_natural_handoffs(
            "**@reviewer** \u{2014} the code is ready. Please check it.",
            "coder",
            &agents,
            &already,
        );
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].teammate_id, "reviewer");
        assert!(mentions[0].message.contains("the code is ready"));
    }

    #[test]
    fn test_natural_handoff_markdown_bold_colon() {
        let agents = sample_agents();
        let already: HashSet<String> = HashSet::new();
        let mentions = extract_natural_handoffs(
            "**@reviewer:** Please review this code.",
            "coder",
            &agents,
            &already,
        );
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].teammate_id, "reviewer");
    }

    #[test]
    fn test_all_agent_mentions_bracket_syntax() {
        let agents = sample_agents();
        let already = HashSet::new();
        let mentions = extract_all_agent_mentions(
            "Here's the code. [@reviewer: Please review this]",
            "coder",
            &agents,
            &already,
        );
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].teammate_id, "reviewer");
        assert!(mentions[0].message.contains("Please review this"));
    }

    #[test]
    fn test_all_agent_mentions_bang_syntax() {
        let agents = sample_agents();
        let already = HashSet::new();
        let mentions = extract_all_agent_mentions(
            "Done coding. [@!reviewer: Check this out]",
            "coder",
            &agents,
            &already,
        );
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].teammate_id, "reviewer");
    }

    #[test]
    fn test_all_agent_mentions_mixed() {
        let agents = sample_agents();
        let already = HashSet::new();
        let mentions = extract_all_agent_mentions(
            "[@reviewer: review this] [@!tester: test this]",
            "coder",
            &agents,
            &already,
        );
        assert_eq!(mentions.len(), 2);
        let ids: HashSet<String> = mentions.iter().map(|m| m.teammate_id.clone()).collect();
        assert!(ids.contains("reviewer"));
        assert!(ids.contains("tester"));
    }

    #[test]
    fn test_all_agent_mentions_skips_self() {
        let agents = sample_agents();
        let already = HashSet::new();
        let mentions = extract_all_agent_mentions(
            "[@coder: talking to myself]",
            "coder",
            &agents,
            &already,
        );
        assert_eq!(mentions.len(), 0);
    }

    #[test]
    fn test_all_agent_mentions_skips_unknown() {
        let agents = sample_agents();
        let already = HashSet::new();
        let mentions = extract_all_agent_mentions(
            "[@unknown_agent: hello]",
            "coder",
            &agents,
            &already,
        );
        assert_eq!(mentions.len(), 0);
    }

    #[test]
    fn test_all_agent_mentions_skips_already_mentioned() {
        let agents = sample_agents();
        let mut already = HashSet::new();
        already.insert("reviewer".to_string());
        let mentions = extract_all_agent_mentions(
            "[@reviewer: review this]",
            "coder",
            &agents,
            &already,
        );
        assert_eq!(mentions.len(), 0);
    }

    #[test]
    fn test_get_agent_reset_flag() {
        let flag = get_agent_reset_flag("coder", std::path::Path::new("/workspace"));
        assert_eq!(flag, std::path::PathBuf::from("/workspace/coder/reset_flag"));
    }
}
