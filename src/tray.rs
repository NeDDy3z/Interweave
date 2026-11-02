use adw::prelude::*;
use adw::{gio, glib};
use ksni::menu::StandardItem;
use ksni::MenuItem;

pub struct Tray {
    pub combining: bool,
}

/// Tray callbacks run on ksni's own thread, so hop to the GTK main thread before touching the app.
fn on_main(f: impl FnOnce(&adw::Application) + Send + 'static) {
    glib::idle_add_once(move || {
        if let Some(app) = gio::Application::default().and_downcast::<adw::Application>() {
            f(&app);
        }
    });
}

fn show_window() {
    // Wayland only raises a window that comes with the activation token of the click.
    let token = ksni::current_xdg_activation_token();
    on_main(move |app| {
        if let Some(window) = app.windows().first() {
            if let Some(token) = &token {
                window.set_startup_id(token);
            }
            window.present();
        }
    });
}

fn run_action(name: &'static str) {
    on_main(move |app| app.activate_action(name, None));
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "interweave".into()
    }

    fn title(&self) -> String {
        "Interweave".into()
    }

    fn icon_name(&self) -> String {
        format!("{}-symbolic", crate::APP_ID)
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        show_window();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let toggle = if self.combining { "Stop Combining" } else { "Start Combining" };
        vec![
            StandardItem { label: "Open".into(), activate: Box::new(|_| show_window()), ..Default::default() }.into(),
            StandardItem { label: toggle.into(), activate: Box::new(|_| run_action("toggle")), ..Default::default() }
                .into(),
            MenuItem::Separator,
            StandardItem { label: "Quit".into(), activate: Box::new(|_| run_action("quit")), ..Default::default() }
                .into(),
        ]
    }
}
