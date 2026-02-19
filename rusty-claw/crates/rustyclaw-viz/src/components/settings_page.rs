use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;
use gloo_net::http::Request;

use crate::types::{
    FullSettings, FullAgentConfig, FullChannelsConfig, FullModelsConfig,
    FullMonitoringConfig, FullTeamConfig, FullWorkspaceConfig,
};

// ─── Main settings page ─────────────────────────────────────────────────────

#[function_component(SettingsPage)]
pub fn settings_page() -> Html {
    let settings = use_state(|| None::<FullSettings>);
    let loading = use_state(|| true);
    let error = use_state(|| None::<String>);

    {
        let settings = settings.clone();
        let loading = loading.clone();
        let error = error.clone();
        use_effect_with((), move |_| {
            spawn_local(async move {
                match Request::get("/api/settings/full").send().await {
                    Ok(resp) if resp.ok() => {
                        match resp.json::<FullSettings>().await {
                            Ok(s) => settings.set(Some(s)),
                            Err(e) => error.set(Some(format!("Parse error: {}", e))),
                        }
                    }
                    Ok(resp) => error.set(Some(format!("HTTP {}", resp.status()))),
                    Err(e) => error.set(Some(format!("Network error: {}", e))),
                }
                loading.set(false);
            });
            || {}
        });
    }

    if *loading {
        return html! { <div class="settings-loading">{"Loading settings..."}</div> };
    }
    if let Some(ref err) = *error {
        return html! { <div class="settings-error">{err}</div> };
    }
    let s = match &*settings {
        Some(s) => s,
        None => return html! { <div class="settings-empty">{"No settings loaded"}</div> },
    };

    html! {
        <div class="settings-page">
            {render_workspace(&s.workspace)}
            {render_models(&s.models)}
            {render_channels(&s.channels)}
            {render_agents(&s.agents)}
            {render_teams(&s.teams)}
            {render_monitoring(&s.monitoring)}
        </div>
    }
}

// ─── Section wrapper ─────────────────────────────────────────────────────────

fn render_section(title: &str, body: Html) -> Html {
    html! {
        <div class="settings-section">
            <h3>{title}</h3>
            <hr class="divider" />
            <div class="settings-section-body">
                {body}
            </div>
        </div>
    }
}

fn render_field(label: &str, value: &str, css: &str) -> Html {
    let css_class = if css.is_empty() {
        classes!("settings-field-value")
    } else {
        classes!("settings-field-value", css.to_string())
    };
    html! {
        <div class="settings-field">
            <span class="settings-field-label">{label}</span>
            <span class={css_class}>{value}</span>
        </div>
    }
}

fn opt_display(val: &Option<String>, placeholder: &str) -> (String, &'static str) {
    match val {
        Some(v) if !v.is_empty() => (v.clone(), ""),
        _ => (placeholder.to_string(), "empty"),
    }
}

// ─── Section renderers ──────────────────────────────────────────────────────

fn render_workspace(ws: &Option<FullWorkspaceConfig>) -> Html {
    let body = match ws {
        Some(w) => {
            let (name, nc) = opt_display(&w.name, "not set");
            let (path, pc) = opt_display(&w.path, "not set");
            html! {
                <>
                    {render_field("Name", &name, nc)}
                    {render_field("Path", &path, pc)}
                </>
            }
        }
        None => html! { {render_field("Status", "not configured", "empty")} },
    };
    render_section("Workspace", body)
}

fn render_models(models: &Option<FullModelsConfig>) -> Html {
    let body = match models {
        Some(m) => {
            let (prov, pc) = opt_display(&m.provider, "auto-detect");
            let anthropic = m.anthropic.as_ref()
                .and_then(|a| a.model.clone())
                .unwrap_or_default();
            let openai = m.openai.as_ref()
                .and_then(|a| a.model.clone())
                .unwrap_or_default();
            let opencode = m.opencode.as_ref()
                .and_then(|a| a.model.clone())
                .unwrap_or_default();
            html! {
                <>
                    {render_field("Provider", &prov, pc)}
                    if !anthropic.is_empty() {
                        {render_field("Anthropic model", &anthropic, "")}
                    }
                    if !openai.is_empty() {
                        {render_field("OpenAI model", &openai, "")}
                    }
                    if !opencode.is_empty() {
                        {render_field("OpenCode model", &opencode, "")}
                    }
                </>
            }
        }
        None => html! { {render_field("Status", "not configured", "empty")} },
    };
    render_section("Models", body)
}

fn render_channels(channels: &Option<FullChannelsConfig>) -> Html {
    let body = match channels {
        Some(ch) => {
            let enabled = ch.enabled.as_ref()
                .map(|v| v.join(", "))
                .unwrap_or_default();
            let discord_token = ch.discord.as_ref()
                .and_then(|d| d.bot_token.clone())
                .unwrap_or_default();
            let telegram_token = ch.telegram.as_ref()
                .and_then(|t| t.bot_token.clone())
                .unwrap_or_default();
            html! {
                <>
                    if !enabled.is_empty() {
                        {render_field("Enabled", &enabled, "")}
                    }
                    <div class="settings-subsection">
                        <h4>{"Discord"}</h4>
                        if discord_token.is_empty() {
                            {render_field("Bot token", "not set", "empty")}
                        } else {
                            {render_field("Bot token", &discord_token, "masked")}
                        }
                    </div>
                    <div class="settings-subsection">
                        <h4>{"Telegram"}</h4>
                        if telegram_token.is_empty() {
                            {render_field("Bot token", "not set", "empty")}
                        } else {
                            {render_field("Bot token", &telegram_token, "masked")}
                        }
                    </div>
                </>
            }
        }
        None => html! { {render_field("Status", "not configured", "empty")} },
    };
    render_section("Channels", body)
}

fn render_agents(agents: &Option<std::collections::HashMap<String, FullAgentConfig>>) -> Html {
    let body = match agents {
        Some(map) if !map.is_empty() => {
            let mut ids: Vec<&String> = map.keys().collect();
            ids.sort();
            html! {
                <table class="settings-table">
                    <thead>
                        <tr>
                            <th>{"ID"}</th>
                            <th>{"Name"}</th>
                            <th>{"Provider"}</th>
                            <th>{"Model"}</th>
                            <th>{"Working Dir"}</th>
                        </tr>
                    </thead>
                    <tbody>
                        { for ids.iter().map(|id| {
                            let a = &map[*id];
                            html! {
                                <tr>
                                    <td>{id}</td>
                                    <td>{&a.name}</td>
                                    <td>{&a.provider}</td>
                                    <td>{&a.model}</td>
                                    <td>{&a.working_directory}</td>
                                </tr>
                            }
                        })}
                    </tbody>
                </table>
            }
        }
        _ => html! { <span class="settings-field-value empty">{"No agents configured"}</span> },
    };
    render_section("Agents", body)
}

fn render_teams(teams: &Option<std::collections::HashMap<String, FullTeamConfig>>) -> Html {
    let body = match teams {
        Some(map) if !map.is_empty() => {
            let mut ids: Vec<&String> = map.keys().collect();
            ids.sort();
            html! {
                <table class="settings-table">
                    <thead>
                        <tr>
                            <th>{"ID"}</th>
                            <th>{"Name"}</th>
                            <th>{"Agents"}</th>
                            <th>{"Leader"}</th>
                        </tr>
                    </thead>
                    <tbody>
                        { for ids.iter().map(|id| {
                            let t = &map[*id];
                            let agents_str = t.agents.iter()
                                .map(|a| format!("@{}", a))
                                .collect::<Vec<_>>()
                                .join(", ");
                            html! {
                                <tr>
                                    <td>{id}</td>
                                    <td>{&t.name}</td>
                                    <td>{agents_str}</td>
                                    <td>{format!("@{}", &t.leader_agent)}</td>
                                </tr>
                            }
                        })}
                    </tbody>
                </table>
            }
        }
        _ => html! { <span class="settings-field-value empty">{"No teams configured"}</span> },
    };
    render_section("Teams", body)
}

fn render_monitoring(mon: &Option<FullMonitoringConfig>) -> Html {
    let body = match mon {
        Some(m) => {
            let interval = m.heartbeat_interval
                .map(|i| format!("{}s", i))
                .unwrap_or_else(|| "3600s (default)".to_string());
            html! { {render_field("Heartbeat interval", &interval, "")} }
        }
        None => html! { {render_field("Status", "not configured (defaults apply)", "empty")} },
    };
    render_section("Monitoring", body)
}
