use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use colored::Colorize;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

use rustyclaw_core::config::{get_agents, get_settings, get_teams, Paths};
use rustyclaw_core::types::MessageData;

// ─── Server state ───────────────────────────────────────────────────────────

struct VizServerState {
    tx: broadcast::Sender<String>,
    paths: Paths,
}

// ─── Settings API response ──────────────────────────────────────────────────

#[derive(Serialize)]
struct SettingsResponse {
    teams: HashMap<String, TeamInfo>,
    agents: HashMap<String, AgentInfo>,
}

#[derive(Serialize)]
struct TeamInfo {
    name: String,
    agents: Vec<String>,
    leader_agent: String,
}

#[derive(Serialize)]
struct AgentInfo {
    name: String,
    provider: String,
    model: String,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<VizServerState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<VizServerState>) {
    let mut rx = state.tx.subscribe();
    while let Ok(msg) = rx.recv().await {
        if socket.send(Message::Text(msg.into())).await.is_err() {
            break;
        }
    }
}

async fn get_settings_handler(
    State(state): State<Arc<VizServerState>>,
) -> impl IntoResponse {
    let settings = get_settings(&state.paths.settings_file).unwrap_or_default();
    let agents_map = get_agents(&settings);
    let teams_map = get_teams(&settings);

    let agents: HashMap<String, AgentInfo> = agents_map
        .into_iter()
        .map(|(id, a)| {
            (
                id,
                AgentInfo {
                    name: a.name,
                    provider: a.provider,
                    model: a.model,
                },
            )
        })
        .collect();

    let teams: HashMap<String, TeamInfo> = teams_map
        .into_iter()
        .map(|(id, t)| {
            (
                id,
                TeamInfo {
                    name: t.name,
                    agents: t.agents,
                    leader_agent: t.leader_agent,
                },
            )
        })
        .collect();

    Json(SettingsResponse { teams, agents })
}

async fn get_status_handler(
    State(state): State<Arc<VizServerState>>,
) -> impl IntoResponse {
    let incoming = count_files(&state.paths.queue_incoming);
    let processing = count_files(&state.paths.queue_processing);

    Json(serde_json::json!({
        "queue_incoming": incoming,
        "queue_processing": processing,
    }))
}

fn count_files(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|ext| ext == "json")
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

// ─── Full settings API (with masked tokens) ──────────────────────────────

async fn get_full_settings_handler(
    State(state): State<Arc<VizServerState>>,
) -> impl IntoResponse {
    let mut settings = get_settings(&state.paths.settings_file).unwrap_or_default();

    // Mask sensitive fields
    if let Some(ref mut channels) = settings.channels {
        if let Some(ref mut discord) = channels.discord {
            if discord.bot_token.is_some() {
                discord.bot_token = Some("********".to_string());
            }
        }
        if let Some(ref mut telegram) = channels.telegram {
            if telegram.bot_token.is_some() {
                telegram.bot_token = Some("********".to_string());
            }
        }
    }

    Json(settings)
}

// ─── Queue API ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct QueueMessagesResponse {
    incoming: Vec<QueuedMessage>,
    processing: Vec<QueuedMessage>,
}

#[derive(Serialize)]
struct QueuedMessage {
    message_id: String,
    channel: String,
    sender: String,
    message: String,
    agent: Option<String>,
    timestamp: u64,
    status: String,
}

fn read_queue_dir(dir: &std::path::Path, status: &str) -> Vec<QueuedMessage> {
    let mut messages = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return messages,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e == "json").unwrap_or(false) {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(msg) = serde_json::from_str::<MessageData>(&content) {
                    let preview = if msg.message.len() > 200 {
                        format!("{}...", &msg.message[..200])
                    } else {
                        msg.message.clone()
                    };
                    messages.push(QueuedMessage {
                        message_id: msg.message_id,
                        channel: msg.channel,
                        sender: msg.sender,
                        message: preview,
                        agent: msg.agent,
                        timestamp: msg.timestamp,
                        status: status.to_string(),
                    });
                }
            }
        }
    }
    messages.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    messages
}

async fn get_queue_handler(
    State(state): State<Arc<VizServerState>>,
) -> impl IntoResponse {
    let incoming = read_queue_dir(&state.paths.queue_incoming, "incoming");
    let processing = read_queue_dir(&state.paths.queue_processing, "processing");
    Json(QueueMessagesResponse { incoming, processing })
}

// ─── Send message API ──────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SendMessageRequest {
    message: String,
    agent: Option<String>,
}

fn random_suffix() -> String {
    let mut rng = rand::thread_rng();
    (0..6)
        .map(|_| {
            let idx = rng.gen_range(0..36u8);
            if idx < 10 { (b'0' + idx) as char } else { (b'a' + idx - 10) as char }
        })
        .collect()
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

async fn post_send_handler(
    State(state): State<Arc<VizServerState>>,
    Json(body): Json<SendMessageRequest>,
) -> impl IntoResponse {
    if body.message.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Message cannot be empty" })),
        );
    }

    let _ = std::fs::create_dir_all(&state.paths.queue_incoming);

    let now = now_millis();
    let message_id = format!("web-{}-{}", now, random_suffix());

    let msg = MessageData {
        channel: "web".to_string(),
        sender: "web-user".to_string(),
        sender_id: Some("web".to_string()),
        message: body.message,
        timestamp: now,
        message_id: message_id.clone(),
        agent: body.agent,
        files: None,
        conversation_id: None,
        from_agent: None,
    };

    let json = match serde_json::to_string_pretty(&msg) {
        Ok(j) => j,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Failed to serialize message" })),
            )
        }
    };

    let filepath = state.paths.queue_incoming.join(format!("{}.json", message_id));
    match std::fs::write(&filepath, &json) {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "ok": true, "messageId": message_id })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "Failed to write message file" })),
        ),
    }
}

// ─── File watcher for events directory ──────────────────────────────────────

fn start_event_watcher(
    events_dir: PathBuf,
    tx: broadcast::Sender<String>,
) -> Result<notify::RecommendedWatcher> {
    std::fs::create_dir_all(&events_dir)?;

    let events_dir_clone = events_dir.clone();
    let tx_watcher = tx.clone();
    let mut watcher = notify::recommended_watcher(
        move |res: std::result::Result<Event, notify::Error>| {
            if let Ok(event) = res {
                if matches!(event.kind, EventKind::Create(_)) {
                    for path in &event.paths {
                        if path.extension().map(|e| e == "json").unwrap_or(false) {
                            // Small delay to ensure file is fully written
                            std::thread::sleep(std::time::Duration::from_millis(50));
                            if let Ok(content) = std::fs::read_to_string(path) {
                                let _ = tx_watcher.send(content.trim().to_string());
                            }
                            // Clean up old event files
                            let _ = std::fs::remove_file(path);
                        }
                    }
                }
            }
        },
    )?;

    watcher.watch(&events_dir, RecursiveMode::NonRecursive)?;

    // Also process any existing recent events
    if let Ok(entries) = std::fs::read_dir(&events_dir_clone) {
        let cutoff = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(30))
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                let modified = entry.metadata().and_then(|m| m.modified()).ok();
                if modified.map(|m| m >= cutoff).unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let _ = tx.send(content.trim().to_string());
                    }
                }
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    Ok(watcher)
}

// ─── Public API ─────────────────────────────────────────────────────────────

/// Start the viz server. This blocks until interrupted.
pub fn start_viz_server(paths: &Paths, port: u16, static_dir: Option<&str>) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async {
        let (tx, _rx) = broadcast::channel::<String>(256);

        let state = Arc::new(VizServerState {
            tx: tx.clone(),
            paths: paths.clone(),
        });

        // Start event file watcher
        let _watcher = start_event_watcher(paths.events_dir.clone(), tx)?;

        // Build router
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any);

        let mut app = Router::new()
            .route("/ws", get(ws_handler))
            .route("/api/settings", get(get_settings_handler))
            .route("/api/status", get(get_status_handler))
            .route("/api/settings/full", get(get_full_settings_handler))
            .route("/api/queue", get(get_queue_handler))
            .route("/api/send", post(post_send_handler))
            .layer(cors)
            .with_state(state);

        // Serve static files if a directory is provided
        if let Some(dir) = static_dir {
            let serve_dir = ServeDir::new(dir);
            app = app.fallback_service(serve_dir);
        }

        let bind_addr = format!("0.0.0.0:{}", port);
        let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

        println!(
            "{}",
            format!("Viz server running at http://localhost:{}", port)
                .green()
                .bold()
        );
        if static_dir.is_some() {
            println!(
                "  Open {} in your browser",
                format!("http://localhost:{}", port).bright_white()
            );
        } else {
            println!(
                "  {}",
                "No static dir specified — only WebSocket and API available"
                    .dimmed()
            );
        }
        println!(
            "  WebSocket: {}",
            format!("ws://localhost:{}/ws", port).bright_white()
        );
        println!("  {}", "(Ctrl+C to stop)".dimmed());

        axum::serve(listener, app).await?;

        Ok::<_, anyhow::Error>(())
    })
}
