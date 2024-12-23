use anyhow::Context;
use clap::{Parser, Subcommand};
use serde_json::json;
use std::process::Command;
use zbus::{dbus_proxy, Connection};

#[dbus_proxy(
    interface = "org.wayland.Osd",
    default_service = "org.wayland.Osd",
    default_path = "/org/wayland/Osd"
)]
trait Osd {
    async fn show_message(&self, message: &str) -> zbus::Result<()>;
}

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

struct OsdClient {
    proxy: OsdProxy<'static>,
}

impl OsdClient {
    async fn new() -> anyhow::Result<Self> {
        let connection = Connection::session()
            .await
            .context("Failed to connect to D-Bus session bus")?;
        let proxy = OsdProxy::new(&connection)
            .await
            .context("Failed to create D-Bus proxy")?;
        Ok(Self { proxy })
    }

    async fn send_message(&self, message: &str) -> anyhow::Result<()> {
        self.proxy
            .show_message(message)
            .await
            .context("Failed to send message")?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let client = OsdClient::new().await?;

    match cli.command {
        Commands::Json { message } => {
            // Validate JSON before sending
            serde_json::from_str::<serde_json::Value>(&message).context("Invalid JSON message")?;
            client.send_message(&message).await?;
        }
        Commands::Audio {
            volume,
            max_volume,
            mute,
        } => {
            let message = if mute {
                json!({
                    "type": "text",
                    "text": "Audio Muted"
                })
            } else {
                json!({
                    "type": "volume",
                    "value": volume,
                    "max_value": max_volume
                })
            };
            client.send_message(&message.to_string()).await?;
        }
        Commands::Brightness { level, max_level } => {
            let message = json!({
                "type": "brightness",
                "value": level,
                "max_value": max_level
            });
            client.send_message(&message.to_string()).await?;
        }
        Commands::Text { message } => {
            let message = json!({
                "type": "text",
                "text": message
            });
            client.send_message(&message.to_string()).await?;
        }
    }

    Ok(())
}
