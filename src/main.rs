mod links;

use adw::prelude::*;
use adw::{gio, glib, gtk};
use links::Limits;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const APP_ID: &str = "dev.neddy.Interweave";
const HELPER: &str = "/usr/libexec/interweave/interweave-helper";

struct LinkRow {
    dev: String,
    kind: String,
    row: adw::ExpanderRow,
    weight: adw::SpinRow,
    down: adw::SpinRow,
    up: adw::SpinRow,
}

impl LinkRow {
    fn new(dev: &str, kind: &str, saved: Option<&Limits>) -> Self {
        let limits = saved.copied().unwrap_or_default();
        let row = adw::ExpanderRow::builder()
            .title(dev)
            .subtitle(kind)
            .show_enable_switch(true)
            .enable_expansion(saved.is_some())
            .build();
        let spin = |title: &str, subtitle: &str, min: f64, max: f64, value: u32| {
            let spin = adw::SpinRow::with_range(min, max, 1.0);
            spin.set_title(title);
            spin.set_subtitle(subtitle);
            spin.set_value(value.into());
            row.add_row(&spin);
            spin
        };
        let weight = spin("Weight", "Share of new connections sent over this link", 1.0, 256.0, limits.weight);
        let down = spin("Download limit", "Mbit/s, 0 = no limit", 0.0, 100000.0, limits.down_mbit);
        let up = spin("Upload limit", "Mbit/s, 0 = no limit", 0.0, 100000.0, limits.up_mbit);
        LinkRow { dev: dev.into(), kind: kind.into(), row, weight, down, up }
    }

    fn limits(&self) -> Limits {
        Limits { weight: self.weight.value() as u32, down_mbit: self.down.value() as u32, up_mbit: self.up.value() as u32 }
    }
}

fn build_ui(app: &adw::Application) {
    let saved = links::read_config();
    let rows: Rc<Vec<LinkRow>> =
        Rc::new(links::list_devices().iter().map(|(dev, kind)| LinkRow::new(dev, kind, saved.get(dev))).collect());

    let combine = adw::SwitchRow::builder()
        .title("Combine connections")
        .subtitle("Spread traffic over all enabled links below")
        .active(!saved.is_empty())
        .build();
    let main_group = adw::PreferencesGroup::new();
    main_group.add(&combine);

    let links_group = adw::PreferencesGroup::builder()
        .title("Connections")
        .description("Each download goes over one link. Apps with many downloads at once, like Steam, use all of them.")
        .build();
    for r in rows.iter() {
        links_group.add(&r.row);
    }
    combine.bind_property("active", &links_group, "sensitive").sync_create().build();

    let page = adw::PreferencesPage::new();
    page.add(&main_group);
    page.add(&links_group);

    let apply = gtk::Button::with_label("Apply");
    apply.add_css_class("suggested-action");
    let header = adw::HeaderBar::new();
    header.pack_end(&apply);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&view));

    apply.connect_clicked(glib::clone!(
        #[strong]
        rows,
        #[weak]
        combine,
        #[weak]
        toasts,
        move |button| {
            let enabled: Vec<(String, Limits)> = if combine.is_active() {
                rows.iter().filter(|r| r.row.enables_expansion()).map(|r| (r.dev.clone(), r.limits())).collect()
            } else {
                Vec::new()
            };
            button.set_sensitive(false);
            glib::spawn_future_local(glib::clone!(
                #[weak]
                button,
                async move {
                    let message = match save(links::format_config(&enabled)).await {
                        Ok(()) if enabled.is_empty() => "Connections are back to normal".to_string(),
                        Ok(()) => "Applied".to_string(),
                        Err(e) => e,
                    };
                    toasts.add_toast(adw::Toast::new(&message));
                    button.set_sensitive(true);
                }
            ));
        }
    ));

    let last: RefCell<HashMap<String, (u64, u64)>> = RefCell::default();
    glib::timeout_add_seconds_local(1, glib::clone!(
        #[strong]
        rows,
        move || {
            let mut last = last.borrow_mut();
            for r in rows.iter() {
                let Some(now) = links::counters(&r.dev) else { continue };
                if let Some(prev) = last.insert(r.dev.clone(), now) {
                    let mbit = |a: u64, b: u64| a.saturating_sub(b) as f64 * 8.0 / 1e6;
                    r.row.set_subtitle(&format!(
                        "{}  -  down {:.1} Mbit/s, up {:.1} Mbit/s",
                        r.kind,
                        mbit(now.0, prev.0),
                        mbit(now.1, prev.1)
                    ));
                }
            }
            glib::ControlFlow::Continue
        }
    ));

    adw::ApplicationWindow::builder()
        .application(app)
        .title("Interweave")
        .default_width(520)
        .default_height(640)
        .content(&toasts)
        .build()
        .present();
}

async fn save(config: String) -> Result<(), String> {
    let proc = gio::Subprocess::newv(
        &["pkexec".as_ref(), HELPER.as_ref(), "save".as_ref()],
        gio::SubprocessFlags::STDIN_PIPE | gio::SubprocessFlags::STDERR_PIPE,
    )
    .map_err(|e| e.to_string())?;
    let (_, stderr) = proc.communicate_utf8_future(Some(config)).await.map_err(|e| e.to_string())?;
    if proc.is_successful() {
        Ok(())
    } else {
        let err = stderr.map(|s| s.trim().to_string()).unwrap_or_default();
        Err(if err.is_empty() { "Not applied".to_string() } else { err })
    }
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}
