mod daemon;
mod agents;
mod teams;
mod pairing_cmd;
mod messaging;
mod setup;
mod viz_server;

use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};

use rustyclaw_core::config::Paths;

#[derive(Parser)]
#[command(name = "rustyclaw", about = "Rusty Claw - Multi-Agent AI Assistant System")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the Rusty Claw daemon (tmux session)
    Start,
    /// Stop the Rusty Claw daemon
    Stop,
    /// Restart the Rusty Claw daemon
    Restart,
    /// Show daemon status
    Status,
    /// Attach to the tmux session
    Attach,
    /// Send a message to the queue
    Send {
        /// The message to send
        message: Vec<String>,
    },
    /// View logs
    Logs {
        /// Log target: queue, discord, telegram, heartbeat, or all
        #[arg(default_value = "queue")]
        target: String,
    },
    /// Reset agent conversation(s)
    Reset {
        /// Agent IDs to reset
        agent_ids: Vec<String>,
    },
    /// Agent management commands
    Agent {
        #[command(subcommand)]
        command: AgentCommands,
    },
    /// Team management commands
    Team {
        #[command(subcommand)]
        command: TeamCommands,
    },
    /// Pairing management commands
    Pairing {
        #[command(subcommand)]
        command: PairingCommands,
    },
    /// Open the browser-based visualizer dashboard
    Visualize {
        /// Port for the viz server
        #[arg(long, default_value = "8090")]
        port: u16,
    },
    /// Run the interactive setup wizard
    Setup,
    /// Set the default provider
    Provider {
        /// Provider name (anthropic, openai, opencode)
        name: Option<String>,
        /// Model to use
        #[arg(long)]
        model: Option<String>,
    },
    /// Set the default model
    Model {
        /// Model name or ID
        name: Option<String>,
    },
    /// Internal: run a daemon component (used by 'rustyclaw start')
    #[command(hide = true)]
    Run {
        #[command(subcommand)]
        component: RunComponent,
    },
}

#[derive(Subcommand)]
enum RunComponent {
    Queue,
    Heartbeat,
    Discord,
    Telegram,
}

#[derive(Subcommand)]
enum AgentCommands {
    /// List all configured agents
    List,
    /// Add a new agent
    Add,
    /// Remove an agent
    Remove {
        /// Agent ID to remove
        agent_id: String,
    },
    /// Show agent details
    Show {
        /// Agent ID to show
        agent_id: String,
    },
    /// Reset an agent's conversation
    Reset {
        /// Agent IDs to reset
        agent_ids: Vec<String>,
    },
}

#[derive(Subcommand)]
enum TeamCommands {
    /// List all configured teams
    List,
    /// Add a new team
    Add,
    /// Remove a team
    Remove {
        /// Team ID to remove
        team_id: String,
    },
    /// Show team details
    Show {
        /// Team ID to show
        team_id: String,
    },
    /// Launch the browser-based team visualizer
    Visualize {
        /// Port for the viz server
        #[arg(long, default_value = "8090")]
        port: u16,
        /// Path to WASM dist directory (built by trunk)
        #[arg(long)]
        static_dir: Option<String>,
    },
}

#[derive(Subcommand)]
enum PairingCommands {
    /// List pending pairing requests
    Pending,
    /// List approved senders
    Approved,
    /// List all (pending + approved)
    List,
    /// Approve a pairing code
    Approve {
        /// The pairing code to approve
        code: String,
    },
    /// Unpair a sender
    Unpair {
        /// Sender ID to unpair
        sender_id: String,
    },
}

/// Auto-detect the WASM visualizer dist directory.
/// Checks (in order):
///   1. RUSTYCLAW_VIZ_DIR env var
///   2. Alongside the binary: <binary_dir>/viz-dist/
///   3. In the source tree: <binary_dir>/../../crates/rustyclaw-viz/dist/
fn find_viz_dist_dir() -> Option<String> {
    if let Ok(dir) = env::var("RUSTYCLAW_VIZ_DIR") {
        let p = PathBuf::from(&dir);
        if p.join("index.html").exists() {
            return Some(dir);
        }
    }

    if let Ok(exe) = env::current_exe() {
        if let Some(bin_dir) = exe.parent() {
            // Installed layout: viz-dist/ next to the binary
            let installed = bin_dir.join("viz-dist");
            if installed.join("index.html").exists() {
                return Some(installed.to_string_lossy().into_owned());
            }

            // Dev layout: binary is in target/debug/ or target/release/
            // Source tree is at ../../crates/rustyclaw-viz/dist/
            let dev = bin_dir
                .join("..")
                .join("..")
                .join("crates")
                .join("rustyclaw-viz")
                .join("dist");
            if dev.join("index.html").exists() {
                return Some(dev.canonicalize().unwrap_or(dev).to_string_lossy().into_owned());
            }
        }
    }

    None
}

fn resolve_paths() -> Paths {
    let script_dir = env::var("RUSTYCLAW_SCRIPT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."))
        });
    Paths::resolve(&script_dir)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = resolve_paths();

    match cli.command {
        Some(Commands::Start) => daemon::start(&paths),
        Some(Commands::Stop) => daemon::stop(&paths),
        Some(Commands::Restart) => daemon::restart(&paths),
        Some(Commands::Status) => daemon::status(&paths),
        Some(Commands::Attach) => daemon::attach(),
        Some(Commands::Setup) => setup::run_setup(&paths),
        Some(Commands::Send { message }) => {
            let msg = message.join(" ");
            messaging::send_message(&msg, &paths)
        }
        Some(Commands::Logs { target }) => messaging::view_logs(&target, &paths),
        Some(Commands::Reset { agent_ids }) => agents::reset_agents(&agent_ids, &paths),
        Some(Commands::Agent { command }) => match command {
            AgentCommands::List => agents::list_agents(&paths),
            AgentCommands::Add => agents::add_agent(&paths),
            AgentCommands::Remove { agent_id } => agents::remove_agent(&agent_id, &paths),
            AgentCommands::Show { agent_id } => agents::show_agent(&agent_id, &paths),
            AgentCommands::Reset { agent_ids } => agents::reset_agents(&agent_ids, &paths),
        },
        Some(Commands::Team { command }) => match command {
            TeamCommands::List => teams::list_teams(&paths),
            TeamCommands::Add => teams::add_team(&paths),
            TeamCommands::Remove { team_id } => teams::remove_team(&team_id, &paths),
            TeamCommands::Show { team_id } => teams::show_team(&team_id, &paths),
            TeamCommands::Visualize { port, static_dir } => {
                viz_server::start_viz_server(&paths, port, static_dir.as_deref())
            }
        },
        Some(Commands::Pairing { command }) => match command {
            PairingCommands::Pending => pairing_cmd::list_pending(&paths),
            PairingCommands::Approved => pairing_cmd::list_approved(&paths),
            PairingCommands::List => pairing_cmd::list_all(&paths),
            PairingCommands::Approve { code } => pairing_cmd::approve(&code, &paths),
            PairingCommands::Unpair { sender_id } => pairing_cmd::unpair(&sender_id, &paths),
        },
        Some(Commands::Visualize { port }) => {
            let static_dir = find_viz_dist_dir();
            viz_server::start_viz_server(&paths, port, static_dir.as_deref())
        }
        Some(Commands::Provider { name, model }) => agents::set_provider(name.as_deref(), model.as_deref(), &paths),
        Some(Commands::Model { name }) => agents::set_model(name.as_deref(), &paths),
        Some(Commands::Run { component }) => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let paths_arc = Arc::new(paths);
                match component {
                    RunComponent::Queue => rustyclaw_queue::run(paths_arc).await,
                    RunComponent::Heartbeat => rustyclaw_heartbeat::run((*paths_arc).clone()).await,
                    RunComponent::Discord => rustyclaw_discord::run(paths_arc).await,
                    RunComponent::Telegram => rustyclaw_telegram::run(paths_arc).await,
                }
            })
        }
        None => {
            daemon::status(&paths)?;
            Ok(())
        }
    }
}
