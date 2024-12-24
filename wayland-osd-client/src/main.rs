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
        // Try to open pipe multiple times
        let mut attempts = 0;
        let max_attempts = 5;
        let mut last_error = None;

        while attempts < max_attempts {
            match OpenOptions::new().write(true).open(PIPE_PATH) {
                Ok(mut file) => {
                    // Create a single buffer with message and separator to ensure atomic write
                    let mut buffer = message.as_bytes().to_vec();
                    buffer.push(0);
                    file.write_all(&buffer)
                        .context("Failed to write message to OSD pipe")?;
                    // Ensure the write is flushed
                    file.flush()
                        .context("Failed to flush message to OSD pipe")?;
                    // Add a small delay to prevent overwhelming the server
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    return Ok(());
                }
                Err(e) => {
                    last_error = Some(e);
                    attempts += 1;
                    if attempts < max_attempts {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                }
            }
        }

        Err(last_error.unwrap().into())
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
