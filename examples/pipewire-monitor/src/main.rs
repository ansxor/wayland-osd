use anyhow::{Context as _, Result};
use clap::Parser;
use pipewire::{context::Context as PwContext, main_loop::MainLoop, types::ObjectType};
use regex::Regex;
use std::{os::unix::fs::PermissionsExt, process::Command, fs};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to wayland-osd-client executable
    #[arg(default_value = "wayland-osd-client")]
    client_path: String,
}

fn get_volume_info() -> Result<(f32, bool)> {
    let output = Command::new("wpctl")
        .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
        .output()
        .context("Failed to execute wpctl")?;

    if !output.status.success() {
        anyhow::bail!("wpctl command failed");
    }

    let output_str =
        String::from_utf8(output.stdout).context("Failed to parse wpctl output as UTF-8")?;

    // Parse volume value and mute state using regex
    let re = Regex::new(r"Volume: ([0-9.]+)( \[MUTED\])?").unwrap();
    let caps = re
        .captures(&output_str)
        .context("Unexpected wpctl output format")?;

    let volume: f32 = caps[1].parse().context("Failed to parse volume value")?;
    let is_muted = caps.get(2).is_some();

    Ok((volume, is_muted))
}

fn run_client(client_path: &str, volume_percent: u32, is_muted: bool) -> Result<()> {
    let mut cmd = Command::new(client_path);

    if is_muted {
        cmd.args(["audio", "--mute", &volume_percent.to_string()]);
    } else {
        cmd.args(["audio", &volume_percent.to_string()]);
    }

    cmd.spawn()
        .with_context(|| format!("Failed to execute client at '{}'", client_path))?;

    Ok(())
}

fn check_client_executable(client_path: &str) -> Result<()> {
    if !std::path::Path::new(client_path).exists() {
        anyhow::bail!("Client not found at '{}'", client_path);
    }

    let metadata = std::fs::metadata(client_path)
        .with_context(|| format!("Failed to get metadata for '{}'", client_path))?;

    #[cfg(unix)]
    if metadata.permissions().mode() & 0o111 == 0 {
        anyhow::bail!("Client at '{}' is not executable", client_path);
    }

    Ok(())
}

fn is_volume_command(pid: u32) -> bool {
    if let Ok(cmdline) = fs::read_to_string(format!("/proc/{}/cmdline", pid)) {
        let args: Vec<&str> = cmdline.split('\0').collect();
        args.iter().any(|arg| {
            *arg == "set-volume" || *arg == "set-mute" || *arg == "@DEFAULT_AUDIO_SINK@"
        })
    } else {
        false
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Verify client exists and is executable
    check_client_executable(&args.client_path)?;

    let mainloop = MainLoop::new(None)?;
    let context = PwContext::new(&mainloop)?;
    let core = context.connect(None)?;
    let register = core.get_registry()?;

    let client_path = args.client_path.clone();
    let _listener = register
        .add_listener_local()
        .global(move |global| {
            if global.type_ == ObjectType::Client {
                if let Some(props) = &global.props {
                    if props.get("application.name") == Some("wpctl") {
                        // Check if this wpctl invocation was for volume control
                        if let Some(pid_str) = props.get("pipewire.sec.pid") {
                            if let Ok(pid) = pid_str.parse::<u32>() {
                                if is_volume_command(pid) {
                                    // wpctl volume command detected, get volume info and update OSD
                                    if let Ok((volume, is_muted)) = get_volume_info() {
                                        let volume_percent = (volume * 100.0).round() as u32;
                                        if let Err(e) = run_client(&client_path, volume_percent, is_muted) {
                                            eprintln!("Failed to run client: {}", e);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        })
        .register();

    mainloop.run();

    Ok(())
}
