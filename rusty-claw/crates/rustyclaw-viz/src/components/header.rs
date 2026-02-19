use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct HeaderProps {
    pub team_id: Option<String>,
    pub team_name: Option<String>,
    pub uptime: String,
    pub connected: bool,
}

#[function_component(Header)]
pub fn header(props: &HeaderProps) -> Html {
    let conn_class = if props.connected {
        "status-connected"
    } else {
        "status-disconnected"
    };

    let conn_icon = if props.connected {
        "\u{25CF}"
    } else {
        "\u{25CB}"
    };

    let conn_text = if props.connected {
        "connected"
    } else {
        "disconnected"
    };

    html! {
        <div class="header">
            <div class="header-content">
                <span class="header-icon">{"\u{2726}"}</span>
                <h1>{"Rusty Claw"}</h1>
                <span class="sep">{"\u{2502}"}</span>
                if let Some(ref tid) = props.team_id {
                    <span class="header-team">
                        {format!("@{}", tid)}
                    </span>
                    if let Some(ref name) = props.team_name {
                        <span class="header-team-name">{format!(" ({})", name)}</span>
                    }
                } else {
                    <span class="header-all">{"all teams"}</span>
                }
                <span class="sep">{"\u{2502}"}</span>
                <span class="header-uptime">{format!("up {}", props.uptime)}</span>
                <span class="sep">{"\u{2502}"}</span>
                <span class={conn_class}>{format!("{} {}", conn_icon, conn_text)}</span>
            </div>
            <hr class="divider" />
        </div>
    }
}
