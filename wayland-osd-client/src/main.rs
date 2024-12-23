use anyhow::Context;
use clap::{Parser, Subcommand};
use serde_json::json;
use std::fs::OpenOptions;
use std::io::Write;

const PIPE_PATH: &str = "/tmp/wayland-osd.pipe";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Send raw JSON message
    Json {
        /// JSON message to send
        message: String,
    },
    /// Control audio-related OSD
    Audio {
        /// Current volume level
        volume: i32,
        /// Maximum volume level
        #[arg(long, default_value = "100")]
        max_volume: i32,
        /// Show muted state
        #[arg(long)]
        mute: bool,
    },
    /// Control brightness-related OSD
    Brightness {
        /// Current brightness level
        level: i32,
        /// Maximum brightness level
        #[arg(long, default_value = "100")]
        max_level: i32,
    },
    /// Show text message
    Text {
        /// Message to display
        message: String,
    },
}

struct OsdClient;

impl OsdClient {
    fn new() -> anyhow::Result<Self> {
        Ok(Self)
    }

    fn send_message(&self, message: &str) -> anyhow::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .open(PIPE_PATH)
            .context("Failed to open OSD pipe")?;

        writeln!(file, "{}", message).context("Failed to write to OSD pipe")?;
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let client = OsdClient::new()?;

    match cli.command {
        Commands::Json { message } => {
            // Validate JSON before sending
            serde_json::from_str::<serde_json::Value>(&message).context("Invalid JSON message")?;
            client.send_message(&message)?;
        }
        Commands::Audio {
            volume,
            max_volume,
            mute,
        } => {
            let message = json!({
                "type": "volume",
                "value": volume,
                "max_value": max_volume,
                "muted": mute
            });
            client.send_message(&message.to_string())?;
        }
        Commands::Brightness { level, max_level } => {
            let message = json!({
                "type": "brightness",
                "value": level,
                "max_value": max_level
            });
            client.send_message(&message.to_string())?;
        }
        Commands::Text { message } => {
            let message = json!({
                "type": "text",
                "text": message
            });
            client.send_message(&message.to_string())?;
        }
    }

    Ok(())
}
