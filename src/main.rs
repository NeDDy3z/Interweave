mod links;
mod tray;
mod update;

use adw::prelude::*;
use adw::{gio, glib, gtk};
use ksni::blocking::TrayMethods;
use links::{LinkPrefs, Limits};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

const APP_ID: &str = "dev.neddy.Interweave";
const HELPER: &str = "/usr/libexec/interweave/interweave-helper";
const REPO_URL: &str = "https://github.com/NeDDy3z/Interweave";
const DESCRIPTION: &str =
    "Combines your internet connections for more total download speed and caps the speed of each one.";

struct LinkRow {
    dev: String,
    kind: String,
    row: adw::ExpanderRow,
    weight: adw::SpinRow,
    down: adw::SpinRow,
    up: adw::SpinRow,
}

impl LinkRow {
    fn new(dev: &str, kind: &str, prefs: &LinkPrefs) -> Self {
        let limits = prefs.limits;
        let row = adw::ExpanderRow::builder()
            .subtitle(format!("{dev}, {kind}"))
            .show_enable_switch(true)
            .enable_expansion(prefs.enabled)
            .visible(!prefs.hidden)
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
        let link = LinkRow { dev: dev.into(), kind: kind.into(), row, weight, down, up };
        link.set_name(&prefs.name);
        link
    }

    fn limits(&self) -> Limits {
        Limits { weight: self.weight.value() as u32, down_mbit: self.down.value() as u32, up_mbit: self.up.value() as u32 }
    }

    fn set_name(&self, name: &str) {
        self.row.set_title(if name.is_empty() { &self.dev } else { name });
    }

    fn reset(&self) {
        let defaults = Limits::default();
        self.row.set_enable_expansion(false);
        self.weight.set_value(defaults.weight.into());
        self.down.set_value(defaults.down_mbit.into());
        self.up.set_value(defaults.up_mbit.into());
        self.set_name("");
        self.row.set_visible(true);
    }
}

struct State {
    rows: Vec<LinkRow>,
    combine: adw::SwitchRow,
    toasts: adw::ToastOverlay,
    prefs: RefCell<HashMap<String, LinkPrefs>>,
    tray: Option<ksni::blocking::Handle<tray::Tray>>,
    pending: RefCell<Option<glib::SourceId>>,
    saving: Cell<bool>,
    quiet: Cell<bool>,
}

impl State {
    fn config(&self) -> String {
        if !self.combine.is_active() {
            return String::new();
        }
        let prefs = self.prefs.borrow();
        let enabled: Vec<(String, Limits)> = self
            .rows
            .iter()
            .filter(|r| r.row.enables_expansion() && !prefs.get(&r.dev).is_some_and(|p| p.hidden))
            .map(|r| (r.dev.clone(), r.limits()))
            .collect();
        links::format_config(&enabled)
    }

    fn edited(self: &Rc<Self>) {
        let mut prefs = self.prefs.borrow_mut();
        for r in &self.rows {
            let p = prefs.entry(r.dev.clone()).or_default();
            p.enabled = r.row.enables_expansion();
            p.limits = r.limits();
        }
        prefs.retain(|_, p| *p != LinkPrefs::default());
        links::write_prefs(&prefs);
        drop(prefs);
        if self.combine.is_active() {
            self.schedule_apply(Duration::from_secs(1));
        }
    }

    fn schedule_apply(self: &Rc<Self>, delay: Duration) {
        if self.quiet.get() {
            return;
        }
        if let Some(id) = self.pending.take() {
            id.remove();
        }
        let state = self.clone();
        let id = glib::timeout_add_local_once(delay, move || {
            state.pending.take();
            state.apply();
        });
        *self.pending.borrow_mut() = Some(id);
    }

    fn apply(self: &Rc<Self>) {
        if self.saving.replace(true) {
            self.schedule_apply(Duration::from_millis(500));
            return;
        }
        let state = self.clone();
        let config = self.config();
        glib::spawn_future_local(async move {
            let result = save(config).await;
            state.saving.set(false);
            if let Err(e) = result {
                state.toasts.add_toast(adw::Toast::new(&e));
                let on = !links::read_config().is_empty();
                state.quietly(|| state.combine.set_active(on));
            }
        });
    }

    fn quietly(&self, f: impl FnOnce()) {
        self.quiet.set(true);
        f();
        self.quiet.set(false);
    }

    fn update_prefs(&self, dev: &str, f: impl FnOnce(&mut LinkPrefs)) {
        let mut prefs = self.prefs.borrow_mut();
        f(prefs.entry(dev.to_string()).or_default());
        prefs.retain(|_, p| *p != LinkPrefs::default());
        links::write_prefs(&prefs);
    }

    fn combine_changed(self: &Rc<Self>) {
        let on = self.combine.is_active();
        if let Some(tray) = &self.tray {
            tray.update(|t| t.combining = on);
        }
        self.schedule_apply(Duration::ZERO);
    }

    /// Turns combining off for real (root config emptied), then exits, tray included.
    fn quit(self: &Rc<Self>, app: &adw::Application) {
        if let Some(id) = self.pending.take() {
            id.remove();
        }
        if self.saving.get() {
            let (state, app) = (self.clone(), app.clone());
            glib::timeout_add_local_once(Duration::from_millis(500), move || state.quit(&app));
            return;
        }
        if links::read_config().is_empty() {
            app.quit();
            return;
        }
        let (state, app) = (self.clone(), app.clone());
        glib::spawn_future_local(async move {
            match save(String::new()).await {
                Ok(()) => app.quit(),
                Err(e) => {
                    state.toasts.add_toast(adw::Toast::new(&format!("Could not stop combining: {e}")));
                    if let Some(window) = app.windows().first() {
                        window.present();
                    }
                }
            }
        });
    }

    fn reset_all(self: &Rc<Self>) {
        self.quietly(|| {
            self.combine.set_active(false);
            self.rows.iter().for_each(LinkRow::reset);
        });
        self.prefs.borrow_mut().clear();
        links::write_prefs(&HashMap::new());
        self.schedule_apply(Duration::ZERO);
    }
}

fn build_ui(app: &adw::Application) {
    let combining = !links::read_config().is_empty();
    let prefs = links::read_prefs();
    let rows: Vec<LinkRow> = links::list_devices()
        .iter()
        .map(|(dev, kind)| LinkRow::new(dev, kind, &prefs.get(dev).cloned().unwrap_or_default()))
        .collect();

    let combine = adw::SwitchRow::builder()
        .title("Combine connections")
        .subtitle("Spread traffic over the enabled links below")
        .active(combining)
        .build();
    let main_group = adw::PreferencesGroup::new();
    main_group.add(&combine);

    let links_group = adw::PreferencesGroup::builder().title("Connections").build();
    for r in &rows {
        links_group.add(&r.row);
    }

    let page = adw::PreferencesPage::new();
    page.add(&main_group);
    page.add(&links_group);

    let menu = gio::Menu::new();
    menu.append(Some("Preferences"), Some("win.preferences"));
    menu.append(Some("About Interweave"), Some("win.about"));
    menu.append(Some("Quit"), Some("app.quit"));
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .tooltip_text("Main Menu")
        .build();
    let header = adw::HeaderBar::new();
    header.pack_end(&menu_button);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&view));

    let state = Rc::new(State {
        rows,
        combine,
        toasts,
        prefs: RefCell::new(prefs),
        // No tray host (e.g. the AppIndicator extension is off): the app just quits on close like before.
        tray: tray::Tray { combining }.spawn().ok(),
        pending: RefCell::default(),
        saving: Cell::new(false),
        quiet: Cell::new(false),
    });

    let s = state.clone();
    state.combine.connect_active_notify(move |_| s.combine_changed());
    for r in &state.rows {
        let s = state.clone();
        r.row.connect_enable_expansion_notify(move |_| s.edited());
        for spin in [&r.weight, &r.down, &r.up] {
            let s = state.clone();
            spin.connect_value_notify(move |_| s.edited());
        }
    }

    let last: RefCell<HashMap<String, (u64, u64)>> = RefCell::default();
    glib::timeout_add_seconds_local(1, glib::clone!(
        #[strong]
        state,
        move || {
            let mut last = last.borrow_mut();
            for r in &state.rows {
                let Some(now) = links::counters(&r.dev) else { continue };
                if let Some(prev) = last.insert(r.dev.clone(), now) {
                    let mbit = |a: u64, b: u64| a.saturating_sub(b) as f64 * 8.0 / 1e6;
                    r.row.set_subtitle(&format!(
                        "{}, {}  -  down {:.1} Mbit/s, up {:.1} Mbit/s",
                        r.dev,
                        r.kind,
                        mbit(now.0, prev.0),
                        mbit(now.1, prev.1)
                    ));
                }
            }
            glib::ControlFlow::Continue
        }
    ));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Interweave")
        .default_width(520)
        .default_height(640)
        .content(&state.toasts)
        .build();
    if state.tray.is_some() {
        window.connect_close_request(|win| {
            win.set_visible(false);
            glib::Propagation::Stop
        });
    }
    app.add_action_entries([
        gio::ActionEntry::builder("toggle")
            .activate(glib::clone!(
                #[strong]
                state,
                move |_: &adw::Application, _, _| state.combine.set_active(!state.combine.is_active())
            ))
            .build(),
        gio::ActionEntry::builder("quit")
            .activate(glib::clone!(
                #[strong]
                state,
                move |app: &adw::Application, _, _| state.quit(app)
            ))
            .build(),
    ]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    window.add_action_entries([
        gio::ActionEntry::builder("preferences")
            .activate(move |win: &adw::ApplicationWindow, _, _| show_preferences(win, &state))
            .build(),
        gio::ActionEntry::builder("about").activate(|win: &adw::ApplicationWindow, _, _| show_about(win)).build(),
    ]);
    window.present();
}

fn show_preferences(win: &adw::ApplicationWindow, state: &Rc<State>) {
    let dialog = adw::PreferencesDialog::new();

    let names = adw::PreferencesGroup::builder()
        .title("Connections")
        .description("Rename or hide connections. Hidden connections are not used.")
        .build();
    for (i, r) in state.rows.iter().enumerate() {
        let label = state.prefs.borrow().get(&r.dev).cloned().unwrap_or_default();
        let entry = adw::EntryRow::builder().title(format!("{}, {}", r.dev, r.kind)).text(&label.name).build();
        let shown = gtk::Switch::builder()
            .active(!label.hidden)
            .valign(gtk::Align::Center)
            .tooltip_text("Show this connection")
            .build();
        entry.add_suffix(&shown);

        let s = state.clone();
        entry.connect_changed(move |e| {
            let name = e.text().trim().to_string();
            s.rows[i].set_name(&name);
            s.update_prefs(&s.rows[i].dev, |l| l.name = name);
        });
        let s = state.clone();
        shown.connect_active_notify(move |sw| {
            s.rows[i].row.set_visible(sw.is_active());
            s.update_prefs(&s.rows[i].dev, |l| l.hidden = !sw.is_active());
            s.edited();
        });
        names.add(&entry);
    }

    let permission = adw::PreferencesGroup::builder().title("Permission").build();
    permission.add(&adw::ActionRow::builder().title("Administrator access").subtitle(permission_status()).build());

    let reset = adw::ButtonRow::builder().title("Reset All").build();
    reset.add_css_class("destructive-action");
    let reset_group = adw::PreferencesGroup::new();
    reset_group.add(&reset);
    reset.connect_activated(glib::clone!(
        #[weak]
        dialog,
        #[strong]
        state,
        move |_| {
            let alert = adw::AlertDialog::new(
                Some("Reset everything?"),
                Some("Turns combining off, removes all speed limits, and forgets connection names and hidden connections."),
            );
            alert.add_responses(&[("cancel", "_Cancel"), ("reset", "_Reset")]);
            alert.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
            alert.set_default_response(Some("cancel"));
            alert.set_close_response("cancel");
            alert.connect_response(Some("reset"), glib::clone!(
                #[weak]
                dialog,
                #[strong]
                state,
                move |_, _| {
                    state.reset_all();
                    dialog.close();
                }
            ));
            alert.present(Some(&dialog));
        }
    ));

    let page = adw::PreferencesPage::new();
    page.add(&names);
    page.add(&permission);
    page.add(&reset_group);
    dialog.add(&page);
    dialog.present(Some(win));
}

fn permission_status() -> &'static str {
    if !std::path::Path::new(HELPER).exists() {
        return "Helper not installed, reinstall the package";
    }
    let pid = std::process::id().to_string();
    let check = std::process::Command::new("pkcheck")
        .args(["--action-id", "dev.neddy.Interweave.apply", "--process", &pid])
        .output();
    match check.map(|out| out.status.code()) {
        Ok(Some(0)) => "Granted, your password is remembered for a few minutes",
        Ok(Some(1)) => "Not allowed for this user",
        _ => "Asks for your password when settings change",
    }
}

fn show_about(win: &adw::ApplicationWindow) {
    let version = env!("CARGO_PKG_VERSION");
    let about = adw::AboutDialog::builder()
        .application_name("Interweave")
        .application_icon(APP_ID)
        .version(version)
        .developer_name("Erik Vanek")
        .website(REPO_URL)
        .issue_url(format!("{REPO_URL}/issues"))
        .license_type(gtk::License::Apache20)
        .comments(format!("{DESCRIPTION}\n\nChecking for updates..."))
        .build();
    about.add_link("Releases", &format!("{REPO_URL}/releases"));
    glib::spawn_future_local(glib::clone!(
        #[weak]
        about,
        async move {
            let status = match update::latest_tag().await {
                Some(tag) if update::is_newer(&tag, version) => {
                    format!("Version {} is available, see Releases.", tag.trim_start_matches('v'))
                }
                Some(_) => "You have the latest version.".to_string(),
                None => "Could not check for updates.".to_string(),
            };
            about.set_comments(&format!("{DESCRIPTION}\n\n{status}"));
        }
    ));
    about.present(Some(win));
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
    // A second launch while running in the background just brings the window back.
    app.connect_activate(|app| match app.windows().first() {
        Some(window) => window.present(),
        None => build_ui(app),
    });
    app.run()
}
