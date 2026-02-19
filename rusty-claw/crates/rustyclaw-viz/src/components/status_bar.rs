use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct StatusBarProps {
    pub queue_depth: u32,
    pub total_processed: u32,
    pub processor_alive: bool,
    pub connected: bool,
}

#[function_component(StatusBar)]
pub fn status_bar(props: &StatusBarProps) -> Html {
    let processor_class = if props.processor_alive {
        "processor-online"
    } else {
        "processor-idle"
    };

    let queue_class = if props.queue_depth > 0 {
        "queue-busy"
    } else {
        "queue-empty"
    };

    let proc_icon = if props.processor_alive { "\u{25CF}" } else { "\u{25CB}" };
    let proc_label = if props.processor_alive { "Processor Online" } else { "Processor Idle" };
    let ws_icon = if props.connected { "\u{25CF}" } else { "\u{25CB}" };
    let ws_class = if props.connected { "ws-connected" } else { "ws-disconnected" };

    html! {
        <div class="status-bar">
            <div class="status-content">
                <span class={processor_class}>
                    {format!("{} {}", proc_icon, proc_label)}
                </span>
                <span class="sep">{"\u{2502}"}</span>
                <span>
                    {"Queue: "}
                    <span class={queue_class}>{props.queue_depth}</span>
                </span>
                <span class="sep">{"\u{2502}"}</span>
                <span>
                    {"Processed: "}
                    <span class="processed-count">{props.total_processed}</span>
                </span>
                <span class="sep">{"\u{2502}"}</span>
                <span class={ws_class}>
                    {format!("{} WS", ws_icon)}
                </span>
            </div>
        </div>
    }
}
