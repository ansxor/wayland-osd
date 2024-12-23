use anyhow::{Context as _, Result};
use clap::Parser;
use env_logger::Env;
use lazy_static::lazy_static;
use log::{debug, error, info, trace, warn};
use pipewire::{context::Context as PwContext, main_loop::MainLoop, types::ObjectType};
use regex::Regex;
use std::{
    collections::VecDeque, fs, os::unix::fs::PermissionsExt, process::Command, sync::Mutex, thread,
    time::Duration,
};

const MAX_QUEUE_SIZE: usize = 10;

lazy_static! {
    static ref GET_VOLUME_PIDS: Mutex<VecDeque<u32>> =
        Mutex::new(VecDeque::with_capacity(MAX_QUEUE_SIZE));
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to wayland-osd-client executable
    #[arg(default_value = "wayland-osd-client")]
    client_path: String,
}

fn add_get_volume_pid(pid: u32) {
    let mut queue = GET_VOLUME_PIDS.lock().unwrap();
    if queue.len() >= MAX_QUEUE_SIZE {
        queue.pop_front(); // Remove oldest PID if queue is full
    }
    queue.push_back(pid);
    trace!(
        "Added PID {} to get-volume queue. Queue size: {}",
        pid,
        queue.len()
    );
}

fn get_volume_info() -> Result<(f32, bool)> {
    trace!("Getting volume information from wpctl");
    let output = Command::new("wpctl")
        .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
        .output()
        .context("Failed to execute wpctl")?;

    // Add our wpctl PID to the queue
    let Ok(pid) = std::process::id().try_into();
    add_get_volume_pid(pid);

    if !output.status.success() {
        error!("wpctl command failed with status: {}", output.status);
        anyhow::bail!("wpctl command failed");
    }

    let output_str =
        String::from_utf8(output.stdout).context("Failed to parse wpctl output as UTF-8")?;
    debug!("Raw wpctl output: {}", output_str);

    // Parse volume value and mute state using regex
    let re = Regex::new(r"Volume: ([0-9.]+)( \[MUTED\])?").unwrap();
    let caps = re
        .captures(&output_str)
        .context("Unexpected wpctl output format")?;

    let volume: f32 = caps[1].parse().context("Failed to parse volume value")?;
    let is_muted = caps.get(2).is_some();

    debug!("Parsed volume: {}, muted: {}", volume, is_muted);
    Ok((volume, is_muted))
}

fn run_client(client_path: &str, volume_percent: u32, is_muted: bool) -> Result<()> {
    let mut cmd = Command::new(client_path);

    if is_muted {
        debug!(
            "Running client with mute state, volume: {}%",
            volume_percent
        );
        cmd.args(["audio", "--mute", &volume_percent.to_string()]);
    } else {
        debug!("Running client with volume: {}%", volume_percent);
        cmd.args(["audio", &volume_percent.to_string()]);
    }

    cmd.spawn()
        .with_context(|| format!("Failed to execute client at '{}'", client_path))?;

    trace!("Client process spawned successfully");
    Ok(())
}

fn check_client_executable(client_path: &str) -> Result<()> {
    debug!("Checking if client exists at '{}'", client_path);
    if !std::path::Path::new(client_path).exists() {
        error!("Client not found at '{}'", client_path);
        anyhow::bail!("Client not found at '{}'", client_path);
    }

    let metadata = std::fs::metadata(client_path)
        .with_context(|| format!("Failed to get metadata for '{}'", client_path))?;

    #[cfg(unix)]
    {
        let mode = metadata.permissions().mode();
        debug!("Client file permissions: {:o}", mode);
        if mode & 0o111 == 0 {
            error!(
                "Client at '{}' is not executable (mode: {:o})",
                client_path, mode
            );
            anyhow::bail!("Client at '{}' is not executable", client_path);
        }
    }

    debug!("Client executable check passed");
    Ok(())
}

fn is_volume_command(pid: u32) -> bool {
    trace!("Checking if PID {} is a volume command", pid);

    // Early return if this is one of our get-volume calls
    {
        let queue = GET_VOLUME_PIDS.lock().unwrap();
        if queue.contains(&pid) {
            debug!("PID {} is from our get-volume call, ignoring", pid);
            return false;
        }
    }

    if let Ok(cmdline) = fs::read_to_string(format!("/proc/{}/cmdline", pid)) {
        let args: Vec<&str> = cmdline.split('\0').collect();
        debug!("Command arguments for PID {}: {:?}", pid, args);

        let is_volume_cmd = args.iter().any(|arg| {
            *arg == "set-volume" || *arg == "set-mute" || *arg == "@DEFAULT_AUDIO_SINK@"
        });

        if is_volume_cmd {
            info!("Detected volume control command: {:?}", args);
        } else {
            debug!("Not a volume control command: {:?}", args);
        }

        is_volume_cmd
    } else {
        warn!("Failed to read command line for PID {}", pid);
        false
    }
}

fn main() -> Result<()> {
    // Initialize logger with timestamp and module path
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .format_module_path(true)
        .init();

    info!("Starting pipewire volume monitor");
    let args = Args::parse();
    info!("Using client path: {}", args.client_path);

    // Verify client exists and is executable
    check_client_executable(&args.client_path)?;

    debug!("Initializing pipewire connection");
    let mainloop = MainLoop::new(None)?;
    let context = PwContext::new(&mainloop)?;
    let core = context.connect(None)?;
    let register = core.get_registry()?;

    info!("Connected to pipewire, monitoring for volume changes");
    let client_path = args.client_path.clone();
    let _listener = register
        .add_listener_local()
        .global(move |global| {
            if global.type_ == ObjectType::Client {
                if let Some(props) = &global.props {
                    trace!("Detected pipewire client: {:?}", props);
                    if props.get("application.name") == Some("wpctl") {
                        debug!("Detected wpctl client");
                        // Check if this wpctl invocation was for volume control
                        if let Some(pid_str) = props.get("pipewire.sec.pid") {
                            if let Ok(pid) = pid_str.parse::<u32>() {
                                if is_volume_command(pid) {
                                    info!("Volume change detected, waiting for changes to take effect");
                                    // Add a small delay to ensure volume change has taken effect
                                    thread::sleep(Duration::from_millis(50));

                                    // Get updated volume info and update OSD
                                    match get_volume_info() {
                                        Ok((volume, is_muted)) => {
                                            let volume_percent = (volume * 100.0).round() as u32;
                                            info!(
                                                "Volume updated - level: {}%, muted: {}",
                                                volume_percent, is_muted
                                            );
                                            if let Err(e) =
                                                run_client(&client_path, volume_percent, is_muted)
                                            {
                                                error!("Failed to run client: {}", e);
                                            }
                                        }
                                        Err(e) => {
                                            error!("Failed to get volume info: {}", e);
                                        }
                                    }
                                }
                            } else {
                                warn!("Invalid PID in pipewire properties: {}", pid_str);
                            }
                        } else {
                            warn!("No PID found in pipewire properties");
                        }
                    }
                }
            }
        })
        .register();

    info!("Starting event loop");
    mainloop.run();

    Ok(())
}
