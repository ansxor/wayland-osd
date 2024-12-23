use futures::future;
use gtk::{glib, prelude::*};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use zbus::{dbus_interface, Connection};

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OsdMessage {
    #[serde(rename = "type")]
    message_type: String,
    value: Option<i32>,
    max_value: Option<i32>,
    text: Option<String>,
}

struct UiElements {
    window: gtk::ApplicationWindow,
    progress_bar: gtk::ProgressBar,
    label: gtk::Label,
}

#[derive(Debug, Clone)]
struct OsdServer {
    sender: mpsc::UnboundedSender<OsdMessage>,
}

#[dbus_interface(name = "org.wayland.Osd")]
impl OsdServer {
    async fn show_message(&self, json_message: &str) -> zbus::fdo::Result<()> {
        let msg: OsdMessage = serde_json::from_str(json_message)
            .map_err(|e| zbus::fdo::Error::Failed(format!("Invalid JSON: {}", e)))?;

        self.sender
            .send(msg)
            .map_err(|_| zbus::fdo::Error::Failed("Failed to send message".into()))?;

        Ok(())
    }

    fn quit(&self) -> Result<(), zbus::fdo::Error> {
        Ok(())
        // Err(Error::NotSupported("Not implemented".to_string()))
    }

    fn raise(&self) -> Result<(), zbus::fdo::Error> {
        Ok(())
        // self.sender
        //     .unbounded_send(AppAction::Raise)
        //     .map_err(|_| Error::Failed("Could not send action".to_string()))
    }

    #[dbus_interface(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[dbus_interface(property)]
    fn can_raise(&self) -> bool {
        true
    }

    #[dbus_interface(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[dbus_interface(property)]
    fn identity(&self) -> &'static str {
        "Spot"
    }

    #[dbus_interface(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        vec![]
    }

    #[dbus_interface(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec![]
    }

    #[dbus_interface(property)]
    fn desktop_entry(&self) -> &'static str {
        "dev.alextren.Spot"
    }
}

fn setup_css() -> gtk::CssProvider {
    let provider = gtk::CssProvider::new();
    let css_data = "
        window {
            background-color: rgba(0, 0, 0, 0.8);
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

    // Anchor to top-center
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_anchor(Edge::Right, true);

    // Set margins
    window.set_margin(Edge::Top, 50);
    window.set_margin(Edge::Left, 0);
    window.set_margin(Edge::Right, 0);

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

    let progress_bar = gtk::ProgressBar::new();
    progress_bar.set_visible(false);

    let label = gtk::Label::new(None);
    label.set_visible(false);

    overlay.append(&progress_bar);
    overlay.append(&label);
    window.set_child(Some(&overlay));

    window.set_visible(false);

    UiElements {
        window,
        progress_bar,
        label,
    }
}

fn handle_message(ui: &UiElements, msg: OsdMessage) {
    match msg.message_type.as_str() {
        "volume" | "brightness" => {
            if let (Some(value), Some(max)) = (msg.value, msg.max_value) {
                ui.progress_bar.set_fraction(value as f64 / max as f64);
                ui.progress_bar.set_visible(true);
                ui.label.set_visible(false);
            }
        }
        "text" => {
            if let Some(text) = msg.text {
                ui.label.set_text(&text);
                ui.label.set_visible(true);
                ui.progress_bar.set_visible(false);
            }
        }
        _ => return,
    }

    ui.window.set_visible(true);

    // Schedule hide after 3 seconds
    let window = ui.window.clone();
    glib::timeout_add_seconds_local(3, move || {
        window.set_visible(false);
        glib::ControlFlow::Continue
    });
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    gtk::init()?;

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

    // let connection = ConnectionBuilder::session()?
    //     .name("org.wayland.Osd")?
    //     .serve_at("/org/wayland/Osd", server)?
    //     .build()
    //     .await?;

    tokio::spawn(async move {
        let conn = Connection::session().await?;
        conn.request_name("org.wayland.Osd").await?;
        conn.object_server().at("/org/wayland/Osd", server).await?;
        future::pending::<()>().await;
        Ok::<_, anyhow::Error>(())
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
