use anyhow::Result;
use pipewire::{self as pw, context::Context, main_loop::MainLoop, types::ObjectType};
use std::process::Command;

fn main() -> Result<()> {
    let mainloop = MainLoop::new(None)?;
    let context = Context::new(&mainloop)?;
    let core = context.connect(None)?;
    let register = core.get_registry()?;

    let _listener = register
        .add_listener_local()
        .global(|global| {
            if global.type_ == ObjectType::Client {
                if let Some(props) = global.props {
                    match props.get("application.name") {
                        Some("wpctl") => {
                            println!("wpctl is running");
                        }
                        _ => {}
                    }
                }
            }
        })
        .register();

    mainloop.run();

    Ok(())
}
