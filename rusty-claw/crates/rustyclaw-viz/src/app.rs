use std::collections::HashMap;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use yew::prelude::*;
use gloo_net::http::Request;
use gloo_timers::callback::Interval;
use wasm_bindgen_futures::spawn_local;

use crate::types::*;
use crate::components::header::Header;
use crate::components::agent_card::AgentCard;
use crate::components::chain_flow::ChainFlow;
use crate::components::activity_log::ActivityLog;
use crate::components::status_bar::StatusBar;
use crate::components::queue_panel::QueuePanel;
use crate::components::send_form::SendForm;
use crate::components::settings_page::SettingsPage;
use crate::components::kanban_board::KanbanBoard;

// ─── App State ──────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
pub struct AppState {
    pub settings: VizSettings,
    pub agent_states: HashMap<String, AgentState>,
    pub arrows: Vec<ChainArrow>,
    pub log_entries: Vec<LogEntry>,
    pub total_processed: u32,
    pub queue_depth: u32,
    pub processor_alive: bool,
    pub connected: bool,
    pub team_filter: Option<String>,
    pub queued_messages: Vec<QueuedMessage>,
    pub kanban_cards: Vec<KanbanCard>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            settings: VizSettings::default(),
            agent_states: HashMap::new(),
            arrows: Vec::new(),
            log_entries: Vec::new(),
            total_processed: 0,
            queue_depth: 0,
            processor_alive: false,
            connected: false,
            team_filter: get_team_filter(),
            queued_messages: Vec::new(),
            kanban_cards: Vec::new(),
        }
    }
}

// ─── Actions ────────────────────────────────────────────────────────────────

pub enum AppAction {
    SetSettings(VizSettings),
    HandleEvent(VizEvent),
    SetConnected(bool),
    SetQueueMessages(Vec<QueuedMessage>),
}

impl Reducible for AppState {
    type Action = AppAction;

    fn reduce(self: Rc<Self>, action: Self::Action) -> Rc<Self> {
        let mut s = (*self).clone();
        match action {
            AppAction::SetSettings(settings) => {
                // Determine which agents to show
                let mut agent_ids: Vec<String> = Vec::new();

                // If filtering by team, show only that team's agents
                if let Some(ref tid) = s.team_filter {
                    if let Some(team) = settings.teams.get(tid) {
                        agent_ids = team.agents.clone();
                    }
                }

                // Otherwise show all team agents, or all agents if no teams
                if agent_ids.is_empty() {
                    for team in settings.teams.values() {
                        for aid in &team.agents {
                            if !agent_ids.contains(aid) {
                                agent_ids.push(aid.clone());
                            }
                        }
                    }
                }
                if agent_ids.is_empty() {
                    agent_ids = settings.agents.keys().cloned().collect();
                }

                let mut states = HashMap::new();
                for id in &agent_ids {
                    if let Some(agent) = settings.agents.get(id) {
                        states.insert(
                            id.clone(),
                            AgentState {
                                id: id.clone(),
                                name: agent.name.clone(),
                                provider: agent.provider.clone(),
                                model: agent.model.clone(),
                                status: AgentStatus::Idle,
                                last_activity: String::new(),
                                response_length: None,
                            },
                        );
                    }
                }
                s.agent_states = states;
                s.settings = settings;
            }
            AppAction::HandleEvent(event) => {
                handle_event(&mut s, &event);
            }
            AppAction::SetConnected(c) => {
                s.connected = c;
            }
            AppAction::SetQueueMessages(msgs) => {
                s.queue_depth = msgs.len() as u32;
                s.queued_messages = msgs;
            }
        }
        Rc::new(s)
    }
}

// ─── Event handler ──────────────────────────────────────────────────────────

fn handle_event(s: &mut AppState, event: &VizEvent) {
    let get_str = |key: &str| -> String {
        event
            .data
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    match event.event_type.as_str() {
        "processor_start" => {
            s.processor_alive = true;
            add_log(s, "\u{26A1}", "Queue processor started", "log-green");
            // Settings may have changed — the WASM app re-fetches on connect
        }

        "message_received" => {
            let channel = get_str("channel");
            let sender = get_str("sender");
            let message = get_str("message");
            add_log(
                s,
                "\u{2709}",
                &format!("[{}] {}: {}", channel, sender, truncate(&message, 50)),
                "log-white",
            );

            // Kanban: create card
            let message_id = get_str("messageId");
            if !message_id.is_empty() {
                let now = js_sys::Date::now();
                s.kanban_cards.push(KanbanCard {
                    id: message_id,
                    current_agent: String::new(),
                    message_preview: truncate(&message, 80),
                    channel,
                    sender,
                    status: KanbanCardStatus::Queued,
                    entered_column_at: now,
                    created_at: now,
                    handoff_trail: Vec::new(),
                    done_at: None,
                    response_length: None,
                });
            }
        }

        "agent_routed" => {
            let aid = get_str("agentId");
            if let Some(agent) = s.agent_states.get_mut(&aid) {
                agent.status = AgentStatus::Active;
                agent.last_activity = "Routing...".to_string();
            }
            let is_team = event
                .data
                .get("isTeamRouted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if is_team {
                add_log(
                    s,
                    "\u{2691}",
                    &format!("Routed to @{} (via team)", aid),
                    "log-cyan",
                );
            } else {
                add_log(s, "\u{2192}", &format!("Routed to @{}", aid), "log-cyan");
            }

            // Kanban: assign unassigned card to this agent
            let now = js_sys::Date::now();
            if let Some(card) = s.kanban_cards.iter_mut().rev()
                .find(|c| c.current_agent.is_empty() && c.status == KanbanCardStatus::Queued)
            {
                card.current_agent = aid.clone();
                card.entered_column_at = now;
                card.handoff_trail.push(HandoffStep {
                    agent_id: aid,
                    entered_at: now,
                });
            }
        }

        "team_chain_start" => {
            let team_name = get_str("teamName");
            let agents_str = event
                .data
                .get("agents")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| format!("@{}", s))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            add_log(
                s,
                "\u{26D3}",
                &format!("Conversation started: {} [{}]", team_name, agents_str),
                "log-magenta",
            );
            s.arrows.clear();
        }

        "chain_step_start" => {
            let aid = get_str("agentId");
            let from = get_str("fromAgent");
            if let Some(agent) = s.agent_states.get_mut(&aid) {
                agent.status = AgentStatus::Active;
                agent.last_activity = if from.is_empty() {
                    "Processing".to_string()
                } else {
                    format!("From @{}", from)
                };
            }

            // Kanban: mark card as active
            if let Some(card) = s.kanban_cards.iter_mut().rev()
                .find(|c| c.current_agent == aid && c.status != KanbanCardStatus::Done)
            {
                card.status = KanbanCardStatus::Active;
            }
        }

        "chain_step_done" => {
            let aid = get_str("agentId");
            let resp_len = event
                .data
                .get("responseLength")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            if let Some(agent) = s.agent_states.get_mut(&aid) {
                agent.status = AgentStatus::Done;
                agent.response_length = Some(resp_len);
            }
            add_log(
                s,
                "\u{1F4AC}",
                &format!("@{}: ({} chars)", aid, resp_len),
                "log-white",
            );

            // Kanban: mark card as step done
            if let Some(card) = s.kanban_cards.iter_mut().rev()
                .find(|c| c.current_agent == aid && c.status == KanbanCardStatus::Active)
            {
                card.status = KanbanCardStatus::StepDone;
                card.response_length = Some(resp_len);
            }
        }

        "chain_handoff" => {
            let from = get_str("fromAgent");
            let to = get_str("toAgent");
            s.arrows.push(ChainArrow {
                from: from.clone(),
                to: to.clone(),
            });
            if let Some(agent) = s.agent_states.get_mut(&to) {
                agent.status = AgentStatus::Waiting;
                agent.last_activity = format!("Handoff from @{}", from);
            }
            add_log(
                s,
                "\u{2192}",
                &format!("@{} \u{2192} @{}", from, to),
                "log-yellow",
            );

            // Kanban: move card from one agent to another
            let now = js_sys::Date::now();
            if let Some(card) = s.kanban_cards.iter_mut().rev()
                .find(|c| c.current_agent == from &&
                    (c.status == KanbanCardStatus::StepDone || c.status == KanbanCardStatus::Active))
            {
                card.current_agent = to.clone();
                card.status = KanbanCardStatus::Queued;
                card.entered_column_at = now;
                card.response_length = None;
                card.handoff_trail.push(HandoffStep {
                    agent_id: to,
                    entered_at: now,
                });
            }
        }

        "team_chain_end" => {
            if let Some(agents) = event.data.get("agents").and_then(|v| v.as_array()) {
                let agent_strs: Vec<String> = agents
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect();
                for aid in &agent_strs {
                    if let Some(agent) = s.agent_states.get_mut(aid) {
                        agent.status = AgentStatus::Done;
                    }
                }
            }
            add_log(s, "\u{2714}", "Conversation complete", "log-green");
        }

        "cross_team_handoff" => {
            let from = get_str("fromAgent");
            let to = get_str("toAgent");
            s.arrows.push(ChainArrow {
                from: from.clone(),
                to: to.clone(),
            });
            if let Some(agent) = s.agent_states.get_mut(&to) {
                agent.status = AgentStatus::Waiting;
                agent.last_activity = format!("Cross-team from @{}", from);
            }
            add_log(
                s,
                "\u{2192}",
                &format!("@{} \u{21C0} @{} (cross-team)", from, to),
                "log-magenta",
            );

            // Kanban: move card cross-team
            let now = js_sys::Date::now();
            if let Some(card) = s.kanban_cards.iter_mut().rev()
                .find(|c| c.current_agent == from &&
                    (c.status == KanbanCardStatus::StepDone || c.status == KanbanCardStatus::Active))
            {
                card.current_agent = to.clone();
                card.status = KanbanCardStatus::Queued;
                card.entered_column_at = now;
                card.response_length = None;
                card.handoff_trail.push(HandoffStep {
                    agent_id: to,
                    entered_at: now,
                });
            }
        }

        "multi_dispatch" => {
            let agents_str = event
                .data
                .get("agents")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| format!("@{}", s))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            add_log(
                s,
                "\u{26A1}",
                &format!("Multi-dispatch: [{}]", agents_str),
                "log-cyan",
            );
        }

        "response_ready" => {
            s.total_processed += 1;
            // Reset agents to idle after response sent
            for agent in s.agent_states.values_mut() {
                if agent.status == AgentStatus::Done || agent.status == AgentStatus::Error {
                    agent.status = AgentStatus::Idle;
                    agent.last_activity = String::new();
                }
            }
            s.arrows.clear();

            // Kanban: move card to Done
            let message_id = get_str("messageId");
            let aid = get_str("agentId");
            let now = js_sys::Date::now();

            let card = if !message_id.is_empty() {
                s.kanban_cards.iter_mut()
                    .find(|c| c.id == message_id && c.status != KanbanCardStatus::Done)
            } else {
                None
            };
            let card = card.or_else(|| {
                if !aid.is_empty() {
                    s.kanban_cards.iter_mut().rev()
                        .find(|c| c.current_agent == aid && c.status != KanbanCardStatus::Done)
                } else {
                    s.kanban_cards.iter_mut().rev()
                        .find(|c| c.status != KanbanCardStatus::Done)
                }
            });
            if let Some(card) = card {
                card.current_agent = String::new();
                card.status = KanbanCardStatus::Done;
                card.done_at = Some(now);
                card.entered_column_at = now;
            }
        }

        _ => {}
    }

    // Kanban cleanup: remove expired Done cards (30s) and stale cards (5min)
    let now = js_sys::Date::now();
    s.kanban_cards.retain(|card| {
        if let Some(done_at) = card.done_at {
            now - done_at < 30_000.0
        } else {
            now - card.created_at < 300_000.0
        }
    });
}

fn add_log(s: &mut AppState, icon: &str, text: &str, css_class: &str) {
    let now = js_sys::Date::new_0();
    let time = format!(
        "{:02}:{:02}:{:02}",
        now.get_hours(),
        now.get_minutes(),
        now.get_seconds()
    );
    s.log_entries.push(LogEntry {
        time,
        icon: icon.to_string(),
        text: text.to_string(),
        css_class: css_class.to_string(),
    });
    if s.log_entries.len() > 50 {
        let drain_count = s.log_entries.len() - 50;
        s.log_entries.drain(..drain_count);
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}\u{2026}", &s[..max.saturating_sub(1)])
    }
}

fn get_ws_url() -> String {
    let window = web_sys::window().expect("no window");
    let location = window.location();
    let protocol = location.protocol().unwrap_or_default();
    let host = location.host().unwrap_or_default();
    let ws_protocol = if protocol == "https:" {
        "wss:"
    } else {
        "ws:"
    };
    format!("{}//{}/ws", ws_protocol, host)
}

fn get_team_filter() -> Option<String> {
    let window = web_sys::window()?;
    let search = window.location().search().ok()?;
    let params = web_sys::UrlSearchParams::new_with_str(&search).ok()?;
    params.get("team")
}

fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

// ─── Main App Component ────────────────────────────────────────────────────

#[function_component(App)]
pub fn app() -> Html {
    let state = use_reducer(AppState::default);
    let start_time = use_state(|| js_sys::Date::now());
    let active_tab = use_state(|| Tab::Dashboard);

    // Force re-render every 500ms for animation dots and uptime
    let update = use_force_update();
    {
        use_effect_with((), move |_| {
            let interval = Interval::new(500, move || {
                update.force_update();
            });
            move || drop(interval)
        });
    }

    // Fetch settings on mount
    {
        let state = state.clone();
        use_effect_with((), move |_| {
            spawn_local(async move {
                if let Ok(resp) = Request::get("/api/settings").send().await {
                    if let Ok(settings) = resp.json::<VizSettings>().await {
                        state.dispatch(AppAction::SetSettings(settings));
                    }
                }
            });
            || {}
        });
    }

    // WebSocket connection
    {
        let state = state.clone();
        use_effect_with((), move |_| {
            let ws_url = get_ws_url();
            let ws = web_sys::WebSocket::new(&ws_url).ok();

            if let Some(ref ws) = ws {
                // onmessage — handle incoming events
                let s = state.clone();
                let onmessage = Closure::wrap(Box::new(move |e: web_sys::MessageEvent| {
                    if let Some(text) = e.data().as_string() {
                        if let Ok(event) = serde_json::from_str::<VizEvent>(&text) {
                            s.dispatch(AppAction::HandleEvent(event));
                        }
                    }
                })
                    as Box<dyn FnMut(web_sys::MessageEvent)>);
                ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
                onmessage.forget();

                // onopen
                let s = state.clone();
                let onopen = Closure::wrap(Box::new(move |_: web_sys::Event| {
                    s.dispatch(AppAction::SetConnected(true));
                }) as Box<dyn FnMut(web_sys::Event)>);
                ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
                onopen.forget();

                // onclose
                let s = state.clone();
                let onclose = Closure::wrap(Box::new(move |_: web_sys::CloseEvent| {
                    s.dispatch(AppAction::SetConnected(false));
                }) as Box<dyn FnMut(web_sys::CloseEvent)>);
                ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
                onclose.forget();
            }

            // Cleanup: close WebSocket on unmount
            move || {
                if let Some(ws) = ws {
                    let _ = ws.close();
                }
            }
        });
    }

    // Poll queue messages every 3 seconds
    {
        let state = state.clone();
        use_effect_with((), move |_| {
            let interval = Interval::new(3_000, move || {
                let state = state.clone();
                spawn_local(async move {
                    if let Ok(resp) = Request::get("/api/queue").send().await {
                        if let Ok(queue_resp) = resp.json::<QueueMessagesResponse>().await {
                            let mut all = queue_resp.incoming;
                            all.extend(queue_resp.processing);
                            state.dispatch(AppAction::SetQueueMessages(all));
                        }
                    }
                });
            });
            move || drop(interval)
        });
    }

    // ─── Derive view data ───────────────────────────────────────────────

    let uptime_secs = ((js_sys::Date::now() - *start_time) / 1000.0) as u64;
    let uptime = format_uptime(uptime_secs);

    let team_filter = &state.team_filter;
    let (team_name, leader_agent) = match team_filter {
        Some(ref tid) => {
            let team = state.settings.teams.get(tid);
            (
                team.map(|t| t.name.clone()),
                team.map(|t| t.leader_agent.clone())
                    .unwrap_or_default(),
            )
        }
        None => (None, String::new()),
    };

    let agents: Vec<AgentState> = {
        let mut list: Vec<_> = state.agent_states.values().cloned().collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    };

    let visible_entries: Vec<LogEntry> = state
        .log_entries
        .iter()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .cloned()
        .collect();

    // ─── Tab callbacks ─────────────────────────────────────────────────

    let on_dashboard = {
        let active_tab = active_tab.clone();
        Callback::from(move |_: MouseEvent| active_tab.set(Tab::Dashboard))
    };
    let on_kanban = {
        let active_tab = active_tab.clone();
        Callback::from(move |_: MouseEvent| active_tab.set(Tab::Kanban))
    };
    let on_queue = {
        let active_tab = active_tab.clone();
        Callback::from(move |_: MouseEvent| active_tab.set(Tab::Queue))
    };
    let on_settings = {
        let active_tab = active_tab.clone();
        Callback::from(move |_: MouseEvent| active_tab.set(Tab::Settings))
    };

    // ─── Render ─────────────────────────────────────────────────────────

    html! {
        <div class="app">
            <Header
                team_id={team_filter.clone()}
                team_name={team_name}
                uptime={uptime}
                connected={state.connected}
            />

            <nav class="tab-bar">
                <button
                    class={classes!("tab-button", (*active_tab == Tab::Dashboard).then_some("tab-active"))}
                    onclick={on_dashboard}
                >
                    {"Dashboard"}
                </button>
                <button
                    class={classes!("tab-button", (*active_tab == Tab::Kanban).then_some("tab-active"))}
                    onclick={on_kanban}
                >
                    {"Board"}
                    {
                        {
                            let active_count = state.kanban_cards.iter()
                                .filter(|c| c.status != KanbanCardStatus::Done)
                                .count();
                            if active_count > 0 {
                                html! { <span class="tab-badge">{active_count}</span> }
                            } else {
                                html! {}
                            }
                        }
                    }
                </button>
                <button
                    class={classes!("tab-button", (*active_tab == Tab::Queue).then_some("tab-active"))}
                    onclick={on_queue}
                >
                    {"Queue"}
                    if state.queue_depth > 0 {
                        <span class="tab-badge">{state.queue_depth}</span>
                    }
                </button>
                <button
                    class={classes!("tab-button", (*active_tab == Tab::Settings).then_some("tab-active"))}
                    onclick={on_settings}
                >
                    {"Settings"}
                </button>
            </nav>

            <div class="tab-content">
                if *active_tab == Tab::Dashboard {
                    if state.settings.teams.is_empty() && state.agent_states.is_empty() {
                        <div class="no-teams">
                            <p class="warning">{"No teams configured."}</p>
                            <p class="hint">{"Create a team with: rustyclaw team add"}</p>
                        </div>
                    } else {
                        <div class="agents-grid">
                            { for agents.iter().map(|agent| {
                                let is_leader = agent.id == leader_agent;
                                html! {
                                    <AgentCard agent={agent.clone()} is_leader={is_leader} />
                                }
                            })}
                        </div>

                        <ChainFlow arrows={state.arrows.clone()} />

                        // Teams legend when viewing all teams
                        if team_filter.is_none() && !state.settings.teams.is_empty() {
                            <div class="teams-legend">
                                <h3>{"Teams"}</h3>
                                { for state.settings.teams.iter().map(|(id, team)| {
                                    let agents_str = team.agents.iter()
                                        .map(|a| format!("@{}", a))
                                        .collect::<Vec<_>>()
                                        .join(", ");
                                    html! {
                                        <div class="team-entry">
                                            <span class="team-id">{format!("@{}", id)}</span>
                                            <span class="team-name">{" "}{&team.name}{" "}</span>
                                            <span class="team-agents">{format!("[{}]", agents_str)}</span>
                                            <span class="team-leader">{format!(" \u{2605} @{}", team.leader_agent)}</span>
                                        </div>
                                    }
                                })}
                            </div>
                        }
                    }

                    <ActivityLog entries={visible_entries} />
                }

                if *active_tab == Tab::Kanban {
                    <KanbanBoard
                        cards={state.kanban_cards.clone()}
                        agents={agents.clone()}
                    />
                }

                if *active_tab == Tab::Queue {
                    <SendForm agents={state.settings.agents.clone()} />
                    <QueuePanel messages={state.queued_messages.clone()} />
                    if state.queued_messages.is_empty() {
                        <div class="queue-empty-hint">
                            {"No messages in queue. Send one above!"}
                        </div>
                    }
                }

                if *active_tab == Tab::Settings {
                    <SettingsPage />
                }
            </div>

            <StatusBar
                queue_depth={state.queue_depth}
                total_processed={state.total_processed}
                processor_alive={state.processor_alive}
                connected={state.connected}
            />
        </div>
    }
}
