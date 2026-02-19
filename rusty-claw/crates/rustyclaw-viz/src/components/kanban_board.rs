use yew::prelude::*;

use crate::types::{AgentState, KanbanCard, KanbanCardStatus};

#[derive(Properties, PartialEq)]
pub struct KanbanBoardProps {
    pub cards: Vec<KanbanCard>,
    pub agents: Vec<AgentState>,
}

/// Format elapsed time since a timestamp (ms epoch)
fn elapsed_str(since: f64) -> String {
    let secs = ((js_sys::Date::now() - since) / 1000.0) as u64;
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn status_label(status: &KanbanCardStatus) -> &'static str {
    match status {
        KanbanCardStatus::Queued => "Queued",
        KanbanCardStatus::Active => "Processing",
        KanbanCardStatus::StepDone => "Step done",
        KanbanCardStatus::Done => "Done",
    }
}

fn render_card(card: &KanbanCard) -> Html {
    let status_css = card.status.css_class();
    let fade_class = if let Some(done_at) = card.done_at {
        let elapsed = js_sys::Date::now() - done_at;
        if elapsed > 20_000.0 { "kanban-card-fading" } else { "" }
    } else {
        ""
    };

    let trail_html = if card.handoff_trail.len() > 1 {
        let trail_str = card
            .handoff_trail
            .iter()
            .map(|h| format!("@{}", h.agent_id))
            .collect::<Vec<_>>()
            .join(" \u{2192} ");
        html! { <div class="kanban-card-trail">{trail_str}</div> }
    } else {
        html! {}
    };

    let chars_html = if let Some(len) = card.response_length {
        html! { <span class="kanban-card-chars">{format!("{} chars", len)}</span> }
    } else {
        html! {}
    };

    html! {
        <div class={classes!("kanban-card", format!("kanban-status-{}", status_css), fade_class)}>
            <div class="kanban-card-status-line">
                <span class={classes!("kanban-status-dot", format!("dot-{}", status_css))}></span>
                <span class="kanban-status-text">{status_label(&card.status)}</span>
                {chars_html}
            </div>
            <div class="kanban-card-preview">{&card.message_preview}</div>
            <div class="kanban-card-meta">
                <span class="kanban-card-channel">{format!("#{}", card.channel)}</span>
                <span class="kanban-card-sender">{&card.sender}</span>
                <span class="kanban-card-elapsed">{elapsed_str(card.entered_column_at)}</span>
            </div>
            {trail_html}
        </div>
    }
}

#[function_component(KanbanBoard)]
pub fn kanban_board(props: &KanbanBoardProps) -> Html {
    let agents = &props.agents;
    let cards = &props.cards;

    // Build agent columns
    let agent_columns = agents.iter().map(|agent| {
        let agent_cards: Vec<&KanbanCard> = cards
            .iter()
            .filter(|c| c.current_agent == agent.id && c.status != KanbanCardStatus::Done)
            .collect();
        let count = agent_cards.len();

        html! {
            <div class="kanban-column">
                <div class="kanban-column-header">
                    <span class="kanban-column-id">{format!("@{}", agent.id)}</span>
                    <span class="kanban-column-name">{&agent.name}</span>
                    if count > 0 {
                        <span class="kanban-column-badge">{count}</span>
                    }
                </div>
                <div class="kanban-column-body">
                    if agent_cards.is_empty() {
                        <div class="kanban-empty">{"No tasks"}</div>
                    } else {
                        { for agent_cards.iter().map(|c| render_card(c)) }
                    }
                </div>
            </div>
        }
    });

    // Done column
    let done_cards: Vec<&KanbanCard> = cards
        .iter()
        .filter(|c| c.status == KanbanCardStatus::Done)
        .collect();
    let done_count = done_cards.len();

    html! {
        <div class="kanban-board">
            <div class="kanban-columns">
                { for agent_columns }
                <div class="kanban-column kanban-column-done">
                    <div class="kanban-column-header">
                        <span class="kanban-column-id">{"\u{2713} Done"}</span>
                        if done_count > 0 {
                            <span class="kanban-column-badge kanban-badge-done">{done_count}</span>
                        }
                    </div>
                    <div class="kanban-column-body">
                        if done_cards.is_empty() {
                            <div class="kanban-empty">{"No completed tasks"}</div>
                        } else {
                            { for done_cards.iter().map(|c| render_card(c)) }
                        }
                    </div>
                </div>
            </div>
        </div>
    }
}
