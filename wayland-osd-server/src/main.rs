use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use gtk::{
    glib::{self, result_from_gboolean},
    prelude::*,
};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use nix::libc;
use nix::sys::stat;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

const PIPE_PATH: &str = "/tmp/wayland-osd.pipe";

// Embed SVG files
const ICON_VOLUME_HIGH: &str = include_str!("../assets/sink-volume-high-symbolic.svg");
const ICON_VOLUME_MEDIUM: &str = include_str!("../assets/sink-volume-medium-symbolic.svg");
const ICON_VOLUME_LOW: &str = include_str!("../assets/sink-volume-low-symbolic.svg");
const ICON_VOLUME_MUTED: &str = include_str!("../assets/sink-volume-muted-symbolic.svg");
const ICON_VOLUME_OVERAMPLIFIED: &str =
    include_str!("../assets/sink-volume-overamplified-symbolic.svg");
const ICON_BRIGHTNESS: &str = include_str!("../assets/display-brightness-symbolic.svg");

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OsdMessage {
    #[serde(rename = "type")]
    message_type: String,
    value: Option<i32>,
    max_value: Option<i32>,
    text: Option<String>,
    muted: Option<bool>,
}

struct UiElements {
    window: gtk::ApplicationWindow,
    progress_bar: gtk::ProgressBar,
    label: gtk::Label,
    icon: gtk::Image,
    timeout_source_id: Arc<Mutex<Option<glib::SourceId>>>,
}

fn load_icon_from_string(svg_data: &str) -> gtk::Image {
    let bytes = glib::Bytes::from_owned(svg_data.as_bytes().to_vec());
    let texture = gtk::gdk::Texture::from_bytes(&bytes).expect("Failed to load icon");
    gtk::Image::from_paintable(Some(&texture))
}

fn get_volume_icon(value: i32, max_value: i32, muted: bool) -> gtk::Image {
    if muted {
        return load_icon_from_string(ICON_VOLUME_MUTED);
    }

    let percentage = (value as f64 / max_value as f64) * 100.0;

    let icon_data = if percentage > 100.0 {
        ICON_VOLUME_OVERAMPLIFIED
    } else if percentage > 66.0 {
        ICON_VOLUME_HIGH
    } else if percentage > 33.0 {
        ICON_VOLUME_MEDIUM
    } else {
        ICON_VOLUME_LOW
    };

    load_icon_from_string(icon_data)
}

#[derive(Debug, Clone)]
struct OsdServer {
    sender: mpsc::UnboundedSender<OsdMessage>,
}

fn setup_css() -> gtk::CssProvider {
    let provider = gtk::CssProvider::new();
    let css_data = "
        window {
            background-color: rgba(0, 0, 0, 0.8);
            min-width: 200px;
            width: 600px;
            margin-start: 50%;
            margin-end: 50%;
            transform: translateX(-50%);
        }
        .osd-overlay {
            margin: 20px;
            padding: 10px;
        }
        progressbar {
            min-height: 10px;
        }
        progressbar trough {
            min-height: 10px;
            background-color: rgba(100, 100, 100, 0.7);
            border-radius: 5px;
        }
        progressbar progress {
            min-height: 10px;
            background-color: #729fcf;
            border-radius: 5px;
        }
        label {
            color: white;
            font-size: 16px;
        }
    ";
    provider.load_from_data(css_data);
    provider
}

fn create_ui(app: &gtk::Application) -> UiElements {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Wayland OSD")
        .default_width(300)
        .default_height(100)
        .build();

    // Initialize as layer shell window
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);

    // Anchor to bottom-center
    window.set_anchor(Edge::Bottom, true);

    // Set margins
    window.set_margin(Edge::Bottom, 50);

    // Set up CSS
    let provider = setup_css();
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("Could not get default display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let overlay = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(vec!["osd-overlay"])
        .build();

    // Create horizontal box for icon and progress bar
    let hbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();

    let icon = load_icon_from_string(ICON_VOLUME_MEDIUM);
    icon.set_visible(false);

    let progress_bar = gtk::ProgressBar::new();
    progress_bar.set_visible(false);

    let label = gtk::Label::new(None);
    label.set_visible(false);

    hbox.append(&icon);
    hbox.append(&progress_bar);

    overlay.append(&hbox);
    overlay.append(&label);
    window.set_child(Some(&overlay));

    window.set_visible(false);

    UiElements {
        window,
        progress_bar,
        label,
        icon,
        timeout_source_id: Arc::new(Mutex::new(None)),
    }
}

fn handle_message(ui: &UiElements, msg: OsdMessage) {
    match msg.message_type.as_str() {
        "volume" => {
            if let (Some(value), Some(max)) = (msg.value, msg.max_value) {
                println!("received message: {}", value);
                ui.progress_bar.set_fraction(value as f64 / max as f64);
                ui.progress_bar.set_visible(true);
                ui.label.set_visible(false);

                // Update icon based on volume level and muted state
                let new_icon = get_volume_icon(value, max, msg.muted.unwrap_or(false));
                if let Some(paintable) = new_icon.paintable() {
                    ui.icon.set_paintable(Some(&paintable));
                }
                ui.icon.set_visible(true);
            }
        }
        "brightness" => {
            if let (Some(value), Some(max)) = (msg.value, msg.max_value) {
                println!("received message: {}", value);
                ui.progress_bar.set_fraction(value as f64 / max as f64);
                ui.progress_bar.set_visible(true);
                ui.label.set_visible(false);
                let brightness_icon = load_icon_from_string(ICON_BRIGHTNESS);
                if let Some(paintable) = brightness_icon.paintable() {
                    ui.icon.set_paintable(Some(&paintable));
                }
                ui.icon.set_visible(true);
            }
        }
        "text" => {
            if let Some(text) = msg.text {
                ui.label.set_text(&text);
                ui.label.set_visible(true);
                ui.progress_bar.set_visible(false);
                ui.icon.set_visible(false);
            }
        }
        _ => return,
    }

    // Remove existing timeout if any
    if let Some(source_id) = ui.timeout_source_id.lock().unwrap().take() {
        unsafe {
            if let Err(err) = result_from_gboolean!(
                glib::ffi::g_source_remove(source_id.as_raw()),
                "Failed to remove source"
            ) {
                eprintln!(
                    "Failed to remove source {}, it may have already been removed: {}",
                    source_id.as_raw(),
                    err.message
                );
            }
        }
    }

    ui.window.set_visible(true);

    // Schedule new hide timeout after 3 seconds
    let window = ui.window.clone();
    let timeout_source_id = ui.timeout_source_id.clone();
    let source_id = glib::timeout_add_seconds_local(3, move || {
        window.set_visible(false);
        *timeout_source_id.lock().unwrap() = None;
        glib::ControlFlow::Break
    });

    // Store the new timeout source ID
    *ui.timeout_source_id.lock().unwrap() = Some(source_id);
}

fn setup_pipe() -> anyhow::Result<()> {
    // Remove existing pipe if it exists
    if Path::new(PIPE_PATH).exists() {
        fs::remove_file(PIPE_PATH)?;
    }

    // Create new pipe with proper permissions
    nix::unistd::mkfifo(
        PIPE_PATH,
        stat::Mode::S_IRUSR | stat::Mode::S_IWUSR | stat::Mode::S_IWGRP | stat::Mode::S_IWOTH,
    )?;

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    gtk::init()?;

    // Set up the named pipe
    setup_pipe()?;

    let (sender, mut receiver) = mpsc::unbounded_channel::<OsdMessage>();
    let server = OsdServer { sender };

    let application = gtk::Application::builder()
        .application_id("org.wayland.osd")
        .build();

    let ui_elements = Arc::new(parking_lot::Mutex::new(None));
    let ui_elements_clone = ui_elements.clone();

    application.connect_activate(move |app| {
        let ui = create_ui(app);
        *ui_elements_clone.lock() = Some(ui);
    });

    // Spawn pipe reading task
    let server_clone = server.clone();
    tokio::spawn(async move {
        loop {
            // Open the pipe for reading
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(PIPE_PATH)
                .unwrap();

            let reader = BufReader::new(file);
            for line in reader.lines() {
                match line {
                    Ok(message) => {
                        if let Ok(msg) = serde_json::from_str::<OsdMessage>(&message) {
                            if server_clone.sender.send(msg).is_err() {
                                eprintln!("Failed to send message through channel");
                            }
                        }
                    }
                    Err(e) => {
                        // Only log errors that aren't EAGAIN (Resource temporarily unavailable)
                        if e.kind() != ErrorKind::WouldBlock {
                            eprintln!("Error reading from pipe: {}", e);
                        }
                    }
                }
            }
            // Small delay before reopening the pipe
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    });

    // Message handling loop
    let ui_elements_clone = ui_elements.clone();
    let context = glib::MainContext::default();
    context.spawn_local(async move {
        while let Some(msg) = receiver.recv().await {
            if let Some(ui) = &*ui_elements_clone.lock() {
                handle_message(ui, msg);
            }
        }
    });

    application.run();
    Ok(())
}
