use std::fs;
use std::io::ErrorKind;
use std::os::fd::{FromRawFd, RawFd};
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use env_logger::Env;
use gtk::{
    glib::{self, result_from_gboolean},
    prelude::*,
};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use log::{debug, error, info, trace, warn};
use nix::sys::stat;
use nix::fcntl::{OFlag, open};
use serde::{Deserialize, Serialize};

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
    device_name: Option<String>,
}

struct UiElements {
    window: gtk::ApplicationWindow,
    progress_bar: gtk::ProgressBar,
    label: gtk::Label,
    device_label: gtk::Label,
    icon: gtk::Image,
    drawing_area: gtk::DrawingArea,
    max_value: Arc<Mutex<i32>>,
    timeout_source_id: Arc<Mutex<Option<glib::SourceId>>>,
}

fn load_icon_from_string(svg_data: &str) -> gtk::Image {
    // Add white fill color to SVG content
    let bytes = glib::Bytes::from_owned(svg_data.as_bytes().to_vec());
    let texture = gtk::gdk::Texture::from_bytes(&bytes).expect("Failed to load icon");
    gtk::Image::from_paintable(Some(&texture))
}

fn get_volume_icon(value: i32, muted: bool) -> gtk::Image {
    if muted {
        return load_icon_from_string(ICON_VOLUME_MUTED);
    }

    let icon_data = if value > 100 {
        ICON_VOLUME_OVERAMPLIFIED
    } else if value > 66 {
        ICON_VOLUME_HIGH
    } else if value > 33 {
        ICON_VOLUME_MEDIUM
    } else {
        ICON_VOLUME_LOW
    };

    load_icon_from_string(icon_data)
}

fn setup_css() -> gtk::CssProvider {
    let provider = gtk::CssProvider::new();
    let css_data = "
        window {
            background-color: rgba(0, 0, 0, 0.8);
            transform: translateX(-50%);
            border-radius: 10px;
        }
        .osd-overlay {
            margin-left: 10px;
            margin-right: 10px;
            margin-top: 5px;
            margin-bottom: 5px;
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
        progressbar.overamplified progress {
            background-color: #cc0000;
        }
        progressbar.overamplified trough {
            background-color: rgba(204, 0, 0, 0.3) !important;
        }
        label {
            color: white;
            font-size: 16px;
        }
        .device-label {
            color: #cccccc;
            font-size: 12px;
            margin-top: -10px;
            margin-bottom: -10px;
        }
    ";
    provider.load_from_data(css_data);
    provider
}

fn create_ui(app: &gtk::Application) -> UiElements {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Wayland OSD")
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

    let main_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(vec!["osd-overlay"])
        .build();

    // Create horizontal box for icon and progress bar
    let hbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .halign(gtk::Align::Center)
        .build();

    let icon = load_icon_from_string(ICON_VOLUME_MEDIUM);
    icon.set_visible(false);

    // Create an overlay for progress bar and marker line
    let progress_overlay = gtk::Overlay::new();

    let progress_bar = gtk::ProgressBar::new();
    progress_bar.set_visible(false);
    progress_overlay.set_child(Some(&progress_bar));

    // Create drawing area for the marker line
    let drawing_area = gtk::DrawingArea::new();
    drawing_area.set_visible(false);
    drawing_area.set_can_target(false);
    drawing_area.set_content_height(10); // Match progress bar height

    // Create shared max_value for drawing area
    let max_value = Arc::new(Mutex::new(100));
    let max_value_for_draw = max_value.clone();

    drawing_area.set_draw_func(move |_area, cr, width, height| {
        // Draw white vertical line
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.8);
        cr.set_line_width(2.0);

        // Position line at 100% mark using the current max_value
        let max = *max_value_for_draw.lock().unwrap();
        let x = (width as f64) * (100.0 / max as f64);
        trace!("Drawing line to y={}", height);
        cr.move_to(x, 1.0);
        cr.line_to(x, 11.0);
        cr.stroke().expect("Failed to draw line");
    });

    progress_overlay.add_overlay(&drawing_area);

    let label = gtk::Label::new(None);
    label.set_visible(false);

    let device_label = gtk::Label::new(None);
    device_label.set_visible(false);
    device_label.set_css_classes(&["device-label"]);

    hbox.append(&icon);
    hbox.append(&progress_overlay);

    main_box.append(&hbox);
    main_box.append(&device_label);
    main_box.append(&label);
    window.set_child(Some(&main_box));

    window.set_visible(false);

    UiElements {
        window,
        progress_bar,
        label,
        device_label,
        icon,
        drawing_area,
        max_value,
        timeout_source_id: Arc::new(Mutex::new(None)),
    }
}

fn handle_message(ui: &UiElements, msg: OsdMessage) {
    debug!("Handling message: {:?}", msg);

    match msg.message_type.as_str() {
        "volume" => {
            if let (Some(value), Some(max)) = (msg.value, msg.max_value) {
                debug!(
                    "Volume update - level: {}, max: {}, muted: {:?}",
                    value, max, msg.muted
                );
                let fraction = value as f64 / max as f64;
                ui.progress_bar.set_fraction(fraction);
                ui.progress_bar.set_visible(true);
                ui.label.set_visible(false);
                
                // Update device name if provided
                if let Some(device_name) = msg.device_name {
                    ui.device_label.set_text(&device_name);
                    ui.device_label.set_visible(true);
                } else {
                    ui.device_label.set_visible(false);
                }

                // Add CSS classes based on volume level
                let style_context = ui.progress_bar.style_context();
                if value > 100 {
                    style_context.add_class("overamplified");
                } else {
                    style_context.remove_class("overamplified");
                }

                // Update max value and show/hide marker line
                if max > 100 {
                    *ui.max_value.lock().unwrap() = max;
                    ui.drawing_area.set_visible(true);
                    ui.drawing_area.queue_draw(); // Force redraw with new max value
                } else {
                    ui.drawing_area.set_visible(false);
                }

                // Update icon based on volume level and muted state
                let new_icon = get_volume_icon(value, msg.muted.unwrap_or(false));
                if let Some(paintable) = new_icon.paintable() {
                    ui.icon.set_paintable(Some(&paintable));
                    trace!("Updated volume icon");
                }
                ui.icon.set_visible(true);
            } else {
                warn!("Received volume message with missing value or max_value");
            }
        }
        "brightness" => {
            if let (Some(value), Some(max)) = (msg.value, msg.max_value) {
                info!("Brightness update - level: {}, max: {}", value, max);
                ui.progress_bar.set_fraction(value as f64 / max as f64);
                ui.progress_bar.set_visible(true);
                ui.label.set_visible(false);
                ui.device_label.set_visible(false);
                ui.drawing_area.set_visible(false); // Always hide marker for brightness

                let brightness_icon = load_icon_from_string(ICON_BRIGHTNESS);
                if let Some(paintable) = brightness_icon.paintable() {
                    ui.icon.set_paintable(Some(&paintable));
                    trace!("Updated brightness icon");
                }
                ui.icon.set_visible(true);
            } else {
                warn!("Received brightness message with missing value or max_value");
            }
        }
        "text" => {
            if let Some(text) = msg.text {
                info!("Text message update: {}", text);
                ui.label.set_text(&text);
                ui.label.set_visible(true);
                ui.progress_bar.set_visible(false);
                ui.icon.set_visible(false);
                ui.device_label.set_visible(false);
                ui.drawing_area.set_visible(false); // Hide marker for text messages
            } else {
                warn!("Received text message with no text content");
            }
        }
        _ => {
            warn!("Received unknown message type: {}", msg.message_type);
            return;
        }
    }

    debug!("Getting to end of building window");

    // Remove existing timeout if any
    if let Some(source_id) = ui.timeout_source_id.lock().unwrap().take() {
        unsafe {
            if let Err(err) = result_from_gboolean!(
                glib::ffi::g_source_remove(source_id.as_raw()),
                "Failed to remove source"
            ) {
                error!(
                    "Failed to remove source {}, it may have already been removed: {}",
                    source_id.as_raw(),
                    err.message
                );
            }
        }
    }

    ui.window.set_visible(true);
    debug!("Showing window");

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
    debug!("Setting up named pipe at {}", PIPE_PATH);

    // Remove existing pipe if it exists
    if Path::new(PIPE_PATH).exists() {
        debug!("Removing existing pipe");
        fs::remove_file(PIPE_PATH)?;
    }

    // Create new pipe with proper permissions
    debug!("Creating new pipe with permissions");
    nix::unistd::mkfifo(
        PIPE_PATH,
        stat::Mode::S_IRUSR | stat::Mode::S_IWUSR | stat::Mode::S_IWGRP | stat::Mode::S_IWOTH,
    )?;

    info!("Named pipe setup complete");
    Ok(())
}

fn main() -> anyhow::Result<()> {
    // Initialize logger with timestamp and module path
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .format_module_path(true)
        .init();

    info!("Starting Wayland OSD server");
    gtk::init()?;

    debug!("Setting up named pipe at {}", PIPE_PATH);
    setup_pipe()?;

    info!("Initializing GTK application");
    let application = gtk::Application::builder()
        .application_id("org.wayland.osd")
        .build();

    let ui_elements = Arc::new(parking_lot::Mutex::new(None));
    let ui_elements_clone = ui_elements.clone();

    application.connect_activate(move |app| {
        let ui = create_ui(app);
        *ui_elements_clone.lock() = Some(ui);

        // Start pipe reading in the GTK main context
        let ui_elements = ui_elements_clone.clone();
        let mut buffer = Vec::with_capacity(4096);
        let mut read_buffer = [0u8; 1024];
        const MAX_MESSAGE_SIZE: usize = 8192;

        // Open pipe in non-blocking mode
        let pipe_fd = match open(PIPE_PATH, OFlag::O_RDONLY | OFlag::O_NONBLOCK, stat::Mode::empty()) {
            Ok(fd) => {
                trace!("Successfully opened pipe in non-blocking mode");
                Some(fd)
            }
            Err(e) => {
                error!("Failed to open pipe: {}", e);
                None
            }
        };

        if let Some(fd) = pipe_fd {
            let mut file = unsafe { std::fs::File::from_raw_fd(fd as RawFd) };
            
            glib::source::idle_add_local(move || {
                match std::io::Read::read(&mut file, &mut read_buffer) {
                    Ok(0) => {
                        // EOF received, but we don't need to reopen since we use message delimiters
                        trace!("EOF received, continuing to next iteration");
                    }
                    Ok(n) => {
                        let mut start = 0;
                        for (i, &byte) in read_buffer[..n].iter().enumerate() {
                            if byte == 0 {
                                if !buffer.is_empty() || i > start {
                                    buffer.extend_from_slice(&read_buffer[start..i]);

                                    if buffer.len() > MAX_MESSAGE_SIZE {
                                        error!("Message too large ({} bytes), discarding", buffer.len());
                                        buffer.clear();
                                    } else if !buffer.is_empty() {
                                        if let Ok(msg_str) = String::from_utf8(buffer.clone()) {
                                            trace!("Received raw message: {}", msg_str);
                                            if let Ok(msg) = serde_json::from_str::<OsdMessage>(&msg_str) {
                                                debug!("Parsed message: {:?}", msg);
                                                if let Some(ui) = &*ui_elements.lock() {
                                                    handle_message(ui, msg);
                                                } else {
                                                    warn!("UI elements not initialized, skipping message");
                                                }
                                            } else {
                                                error!("Failed to parse message: {}", msg_str);
                                            }
                                        } else {
                                            error!("Invalid UTF-8 in message");
                                        }
                                    }
                                    buffer.clear();
                                }
                                start = i + 1;
                            }
                        }

                        if start < n {
                            let remaining = &read_buffer[start..n];
                            if buffer.len() + remaining.len() > MAX_MESSAGE_SIZE {
                                error!("Message would exceed size limit, discarding");
                                buffer.clear();
                            } else {
                                buffer.extend_from_slice(remaining);
                            }
                        }
                    }
                    Err(e) => {
                        if e.kind() != ErrorKind::WouldBlock {
                            error!("Error reading from pipe: {}", e);
                        }
                    }
                }

                glib::ControlFlow::Continue
            });
        }
    });

    application.run();
    Ok(())
}
