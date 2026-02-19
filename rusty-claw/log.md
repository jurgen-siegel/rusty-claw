# Rusty Claw — Development Log

Port of Rusty Claw (TypeScript/Bash multi-agent AI assistant) to Rust + WASM.

---

## Phase 1: Scaffold + Core Library

### Workspace Setup
- Created Cargo workspace with 7 crates: `rustyclaw-core`, `rustyclaw-queue`, `rustyclaw-discord`, `rustyclaw-telegram`, `rustyclaw-cli`, `rustyclaw-heartbeat`, `rustyclaw-viz`
- Configured workspace dependencies for consistent versioning across crates
- Added `default-members` to exclude `rustyclaw-viz` from native builds (WASM-only)
- Release profile: LTO, strip, single codegen unit

### rustyclaw-core (39 tests)
Shared library used by all binaries.

| Module | Purpose |
|---|---|
| `types.rs` | All data structures: `AgentConfig`, `TeamConfig`, `Settings`, `MessageData`, `ResponseData`, `QueueFile`, `Conversation`, `TeamContext`, `RoutingResult`, `TeammateMention`. Full serde support with `#[serde(rename)]` for camelCase JSON compatibility. |
| `config.rs` | Path resolution (`RUSTYCLAW_HOME` env → local `.rustyclaw/` → `~/.rustyclaw/`), `get_settings()`, `get_agents()`, `get_teams()`, `get_workspace_path()`. Fallback to default agent from legacy `models` section. |
| `models.rs` | Static model ID maps for Claude (`CLAUDE_MODEL_IDS`), Codex (`CODEX_MODEL_IDS`), OpenCode (`OPENCODE_MODEL_IDS`). Shortname resolution (e.g. "sonnet" → "claude-sonnet-4-5"). |
| `routing.rs` | `parse_agent_routing()` — handles `@agent`, `@team`, agent name, default fallback. `find_team_for_agent()`, `extract_teammate_mentions()` (single + comma-separated), `detect_multiple_agents()`, `get_agent_reset_flag()`. |
| `pairing.rs` | Full sender allowlist lifecycle: `load_pairing_state()`, `save_pairing_state()` (atomic write via tmp+rename), `ensure_sender_paired()`, `approve_pairing_code()`. 8-char pairing codes with confusing-char exclusion. |
| `logging.rs` | `log()` — timestamped console + file append. `emit_event()` — structured JSON events for visualizer with unique filenames. |
| `agent_setup.rs` | `copy_dir_sync()`, `ensure_agent_directory()` (copies templates: `.claude/`, `heartbeat.md`, `AGENTS.md`, `SOUL.md`, skills symlinks), `update_agent_teammates()` (updates `<!-- TEAMMATES_START -->` blocks in AGENTS.md + CLAUDE.md). |

---

## Phase 2: Queue Processor (10 tests)

### rustyclaw-queue
The heart of the system — processes messages through AI agents.

| Module | Purpose |
|---|---|
| `invoke.rs` | `run_command()` — spawns CLI subprocesses via `tokio::process::Command`. `invoke_agent()` for all 3 providers (Claude, Codex, OpenCode). `parse_codex_output()` and `parse_opencode_output()` for JSONL parsing. |
| `conversation.rs` | Team conversation tracking: `create_conversation()`, `enqueue_internal_message()`, `collect_files()`, `complete_conversation()`, `handle_long_response()`, `save_chat_history()`. Max 50 messages per conversation, 4000-char long response threshold. |
| `processor.rs` | Main queue loop with per-agent `tokio::sync::mpsc::channel` for sequential per-agent processing. `peek_agent_id()`, `process_message()`, `recover_orphaned_files()`, `list_queue_files()`. File watching via `notify` with polling fallback. |

**Architecture:** Each agent gets a dedicated tokio task with its own mpsc channel. Different agents process in parallel; messages to the same agent are processed sequentially.

---

## Phase 3: Channel Clients

### rustyclaw-discord (Serenity 0.12)
- `#[async_trait] impl EventHandler` for DM handling
- Commands: `/agent`, `/team`, `/reset @id`
- Pairing check via `ensure_sender_paired()`
- Attachment download via `reqwest` to `FILES_DIR`
- Outgoing queue polling via `Arc<Http>` (1s interval)
- Message splitting at 2000 chars
- Typing indicator refresh every 8 seconds
- 10-minute pending message cleanup

### rustyclaw-telegram (Teloxide 0.13)
- `teloxide::repl` handler for private chat messages
- All media type downloads: photo (largest size), document, audio, voice, video, video_note, sticker
- MIME-to-extension mapping for proper file naming
- Message splitting at 4096 chars
- Typing indicator every 4 seconds
- Uses `reply_parameters(ReplyParameters::new(msg.id))` (teloxide 0.13 API)

### rustyclaw-heartbeat
- `tokio::time::sleep` loop with configurable interval (default 3600s)
- Per-agent: reads `heartbeat.md` or uses default prompt
- Writes heartbeat messages to `QUEUE_INCOMING`
- Waits 10s, checks `QUEUE_OUTGOING` for responses, logs summaries

---

## Phase 4: CLI

### rustyclaw-cli (clap 4 + dialoguer)
Full replacement for all bash scripts (`tinyclaw.sh` + `lib/*.sh`).

| Module | Purpose |
|---|---|
| `main.rs` | Clap command tree with all subcommands. `resolve_paths()` from `RUSTYCLAW_SCRIPT_DIR` env or exe parent. |
| `daemon.rs` | Tmux session lifecycle: `start()` creates session with panes for queue, channels, heartbeat, logs. `stop()` kills session + lingering processes. `restart()` with tmux-inside detection. `status()` shows daemon, agents, teams, queue depth, last activity. |
| `agents.rs` | Agent CRUD: `list_agents()`, `add_agent()` (interactive with provider/model selection), `remove_agent()` (with team membership check), `show_agent()`, `reset_agents()` (flag file), `set_provider()`, `set_model()`. Atomic settings save via tmp+rename. |
| `teams.rs` | Team CRUD: `list_teams()`, `add_team()` (interactive with MultiSelect for members, leader selection, namespace collision check), `remove_team()`, `show_team()`. Auto-updates AGENTS.md for all team members. |
| `pairing_cmd.rs` | Pairing CLI: `list_pending()`, `list_approved()`, `list_all()`, `approve()`, `unpair()`. Formatted output with timestamps. |
| `messaging.rs` | `send_message()` — writes MessageData JSON to `queue/incoming`. `view_logs()` — `tail -f` on log files by target (queue, discord, telegram, heartbeat, all). |
| `viz_server.rs` | Axum WebSocket server for browser visualizer. Watches `events/` with `notify`, broadcasts to WebSocket clients. REST endpoints: `/api/settings`, `/api/status`. Static file serving for WASM dist. |
| `setup.rs` | Interactive setup wizard: channel selection + tokens, provider/model selection, workspace config, default agent + optional additional agents. Writes `settings.json`, creates all directories, copies templates. |

**Command tree:**
```
rustyclaw setup
rustyclaw start|stop|restart|status|attach
rustyclaw send <message>
rustyclaw logs [target]
rustyclaw reset <agent_id...>
rustyclaw provider [name] [--model model]
rustyclaw model [name]
rustyclaw agent list|add|remove|show|reset
rustyclaw team list|add|remove|show|visualize
rustyclaw pairing pending|approved|list|approve|unpair
```

---

## Phase 5: WASM Visualizer

### rustyclaw-viz (Yew 0.21 + axum)
Browser-based real-time team monitoring dashboard, replacing the terminal-based Ink/React visualizer.

**Architecture:**
```
[Queue Processor writes events/] → [Viz Server watches events/] → [WebSocket] → [Browser WASM App]
```

**Frontend (Yew WASM):**
- `app.rs` — Main `App` component with `use_reducer` for centralized state management. WebSocket connection via raw `web_sys::WebSocket` callbacks. Settings fetch via `gloo-net` HTTP. Animation tick via `gloo-timers::Interval` + `use_force_update`.
- `types.rs` — Shared types: `AgentState`, `AgentStatus`, `ChainArrow`, `LogEntry`, `VizSettings`, `VizEvent` (with `#[serde(flatten)]` for dynamic fields).
- `components/header.rs` — Team name, uptime, connection status
- `components/agent_card.rs` — Per-agent status card with animated processing dots, provider/model, leader star
- `components/chain_flow.rs` — Message flow arrows: `@agent1 → @agent2 → @agent3`
- `components/activity_log.rs` — Last 12 events with timestamps and colored icons
- `components/status_bar.rs` — Queue depth, processor status, processed count, WebSocket status

**Event types handled:** `processor_start`, `message_received`, `agent_routed`, `team_chain_start`, `chain_step_start`, `chain_step_done`, `chain_handoff`, `team_chain_end`, `response_ready`

**Styling:** Dark terminal theme with CSS variables. Status-colored borders on agent cards. CSS Grid layout.

**Build:** `trunk build` from `crates/rustyclaw-viz/` (requires `wasm32-unknown-unknown` target).

**Run:** `rustyclaw team visualize [--port 8090] [--static-dir path/to/dist]`

---

## Technical Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Async runtime | tokio | Required by serenity, teloxide, axum |
| Discord | serenity 0.12 | De facto Rust Discord library |
| Telegram | teloxide 0.13 | Most popular Rust Telegram bot library |
| WhatsApp | Excluded | No viable pure-Rust approach |
| WASM framework | Yew 0.21 | Most mature Rust WASM framework |
| CLI | clap 4 (derive) | Standard Rust CLI library |
| Interactive prompts | dialoguer 0.11 | Terminal prompts (Input, Select, Confirm, MultiSelect) |
| Error handling | thiserror (core) + anyhow (bins) | Typed errors in library, ergonomic wrapping in binaries |
| File watching | notify 7 | inotify-based with polling fallback |
| Viz server | axum 0.8 | Lightweight, tokio-native, built-in WebSocket |

---

## Test Coverage

| Crate | Tests | Notes |
|---|---|---|
| rustyclaw-core | 39 | config, models, routing, pairing lifecycle, logging, agent_setup |
| rustyclaw-queue | 10 | invoke parsing, conversation state, response handling |
| **Total** | **49** | All passing, zero warnings |

### WASM Build

Verified with `trunk build` — compiles to `wasm32-unknown-unknown`, produces `dist/` with index.html, CSS, JS bindings, and ~2MB WASM binary. Zero warnings.

---

## File Structure

```
rusty-claw/
  Cargo.toml                          # Workspace root
  log.md                              # This file
  crates/
    rustyclaw-core/src/                 # 7 modules, 39 tests
    rustyclaw-queue/src/                # 4 modules, 10 tests
    rustyclaw-discord/src/main.rs       # Serenity bot
    rustyclaw-telegram/src/main.rs      # Teloxide bot
    rustyclaw-heartbeat/src/main.rs     # Periodic prompting
    rustyclaw-cli/src/                  # 8 modules (main, daemon, agents, teams, pairing_cmd, messaging, setup, viz_server)
    rustyclaw-viz/                      # Yew WASM app
      Cargo.toml
      Trunk.toml
      index.html
      style.css
      src/
        lib.rs
        app.rs
        types.rs
        components/
          mod.rs, header.rs, agent_card.rs, chain_flow.rs, activity_log.rs, status_bar.rs
```
