//! PROTOTYPE. Throwaway. Not the app.
//!
//! Question: How should the main window be arranged?
//!
//! The main window is the Session. Host config is a dialog: a saved-Host
//! list on the left, fields on the right, closed with the window.
//! Launch shows no Session and no fields.
//!
//! The Prototype window switches Launch, Tree view, and Folder view, and
//! the tab bar edge. Those controls are not part of the app.
//!
//! Run: cargo run --example main_window_prototype

use eframe::egui::{self, Color32, CornerRadius, FontId, Frame, Margin, RichText, Stroke, Vec2};

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        persist_window: false,
        viewport: egui::ViewportBuilder::default()
            .with_title("sshexplorer")
            .with_inner_size([1200.0, 760.0]),
        ..Default::default()
    };
    eframe::run_native(
        "sshexplorer-layout-prototype",
        native_options,
        Box::new(|cc| {
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(Prototype::new()))
        }),
    )
}

const APP_BG: Color32 = Color32::from_rgb(20, 20, 20);
const PANEL: Color32 = Color32::from_rgb(30, 30, 30);
const DIALOG: Color32 = Color32::from_rgb(37, 37, 37);
const RAISED: Color32 = Color32::from_rgb(48, 48, 48);
const HOVER: Color32 = Color32::from_rgb(56, 56, 56);
const SELECT: Color32 = Color32::from_rgb(66, 66, 66);
const BORDER: Color32 = Color32::from_rgb(58, 58, 58);
const TEXT: Color32 = Color32::from_rgb(232, 232, 232);
const WEAK: Color32 = Color32::from_rgb(154, 154, 154);
const FIELD: Color32 = Color32::from_rgb(24, 24, 24);

fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = APP_BG;
    visuals.window_fill = DIALOG;
    visuals.extreme_bg_color = FIELD;
    visuals.faint_bg_color = Color32::from_rgb(32, 32, 32);
    visuals.code_bg_color = FIELD;
    visuals.text_edit_bg_color = Some(FIELD);
    visuals.weak_text_color = Some(WEAK);
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.menu_corner_radius = CornerRadius::same(10);
    visuals.window_shadow = egui::Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(180),
    };
    visuals.popup_shadow = visuals.window_shadow;
    visuals.selection.bg_fill = SELECT;
    visuals.selection.stroke = Stroke::new(1.0, TEXT);
    visuals.indent_has_left_vline = false;
    visuals.collapsing_header_frame = false;
    visuals.interact_cursor = None;

    let widget = |fill: Color32, radius: u8| egui::style::WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(1.0, BORDER),
        corner_radius: CornerRadius::same(radius),
        fg_stroke: Stroke::new(1.0, TEXT),
        expansion: 0.0,
    };
    visuals.widgets.noninteractive = widget(APP_BG, 8);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.inactive = widget(RAISED, 8);
    visuals.widgets.hovered = widget(HOVER, 8);
    visuals.widgets.active = widget(SELECT, 8);
    visuals.widgets.open = visuals.widgets.hovered.clone();
    ctx.set_visuals_of(egui::Theme::Dark, visuals);

    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 5.0);
        style.spacing.interact_size.y = 28.0;
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Launch,
    Tree,
    Folder,
}

impl Screen {
    fn label(self) -> &'static str {
        match self {
            Self::Launch => "Launch",
            Self::Tree => "Tree view",
            Self::Folder => "Folder view",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TabEdge {
    Top,
    Left,
    Right,
    Bottom,
}

impl TabEdge {
    fn label(self) -> &'static str {
        match self {
            Self::Top => "Top",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Bottom => "Bottom",
        }
    }

    const ALL: [Self; 4] = [Self::Top, Self::Left, Self::Right, Self::Bottom];
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Host(usize),
    New,
    Unsaved,
}

impl Page {
    fn title(self, hosts: &[Host]) -> String {
        match self {
            Self::Host(index) => hosts
                .get(index)
                .map(|host| {
                    if host.display_name.is_empty() {
                        host.address.clone()
                    } else {
                        host.display_name.clone()
                    }
                })
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "Host".to_string()),
            Self::New => "New Host".to_string(),
            Self::Unsaved => "Unsaved Session".to_string(),
        }
    }
}

enum DialogEvent {
    None,
    Close,
    Select(Page),
    Save,
    OpenSession,
}

struct Node {
    name: String,
    dir: bool,
    children: Vec<Node>,
}

fn dir(name: &str, children: Vec<Node>) -> Node {
    Node {
        name: name.to_string(),
        dir: true,
        children,
    }
}

fn file(name: &str) -> Node {
    Node {
        name: name.to_string(),
        dir: false,
        children: Vec::new(),
    }
}

fn sample_tree() -> Node {
    dir(
        "/",
        vec![
            dir(
                "home",
                vec![dir(
                    "mnj",
                    vec![
                        dir(".ssh", vec![file("config")]),
                        dir(
                            "src",
                            vec![dir(
                                "sshexplorer",
                                vec![
                                    file("Cargo.toml"),
                                    dir("src", vec![file("main.rs")]),
                                    file("README.md"),
                                ],
                            )],
                        ),
                        file("notes.txt"),
                    ],
                )],
            ),
            dir(
                "var",
                vec![dir(
                    "log",
                    vec![file("syslog"), dir("apt", vec![file("history.log")])],
                )],
            ),
        ],
    )
}

struct Host {
    display_name: String,
    address: String,
    account: String,
    key_file: String,
    readonly: bool,
    remember: bool,
    local_folder: String,
    initial_remote_folder: String,
    initial_path: String,
}

fn sample_hosts() -> Vec<Host> {
    vec![
        Host {
            display_name: "build".to_string(),
            address: "build.example".to_string(),
            account: "mnj".to_string(),
            key_file: String::new(),
            readonly: true,
            remember: false,
            local_folder: "~/Downloads".to_string(),
            initial_remote_folder: String::new(),
            initial_path: "/home/mnj".to_string(),
        },
        Host {
            display_name: "logs".to_string(),
            address: "logs.internal".to_string(),
            account: "ops".to_string(),
            key_file: String::new(),
            readonly: true,
            remember: true,
            local_folder: "~/Downloads/logs".to_string(),
            initial_remote_folder: "/var/log".to_string(),
            initial_path: "/var/log".to_string(),
        },
    ]
}

struct Draft {
    display_name: String,
    address: String,
    account: String,
    key_file: String,
    readonly: bool,
    remember: bool,
    local_folder: String,
    initial_remote_folder: String,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            display_name: String::new(),
            address: String::new(),
            account: String::new(),
            key_file: String::new(),
            readonly: true,
            remember: false,
            local_folder: "~/Downloads".to_string(),
            initial_remote_folder: String::new(),
        }
    }
}

impl Draft {
    fn from_host(host: &Host) -> Self {
        Self {
            display_name: host.display_name.clone(),
            address: host.address.clone(),
            account: host.account.clone(),
            key_file: host.key_file.clone(),
            readonly: host.readonly,
            remember: host.remember,
            local_folder: host.local_folder.clone(),
            initial_remote_folder: host.initial_remote_folder.clone(),
        }
    }

    fn label(&self) -> String {
        if !self.display_name.is_empty() {
            self.display_name.clone()
        } else if !self.address.is_empty() {
            self.address.clone()
        } else {
            "(unsaved)".to_string()
        }
    }

    fn initial_path(&self) -> String {
        if !self.initial_remote_folder.is_empty() {
            self.initial_remote_folder.clone()
        } else if !self.account.is_empty() {
            format!("/home/{}", self.account)
        } else {
            "/home/mnj".to_string()
        }
    }

    fn to_host(&self) -> Host {
        Host {
            display_name: self.display_name.clone(),
            address: self.address.clone(),
            account: self.account.clone(),
            key_file: self.key_file.clone(),
            readonly: self.readonly,
            remember: self.remember,
            local_folder: self.local_folder.clone(),
            initial_remote_folder: self.initial_remote_folder.clone(),
            initial_path: self.initial_path(),
        }
    }
}

struct OpenSession {
    host_index: Option<usize>,
    label: String,
    address: String,
    account: String,
    readonly: bool,
    local_folder: String,
    initial_remote_folder: String,
    initial_path: String,
}

impl OpenSession {
    fn from_host(index: usize, host: &Host) -> Self {
        Self {
            host_index: Some(index),
            label: if host.display_name.is_empty() {
                host.address.clone()
            } else {
                host.display_name.clone()
            },
            address: host.address.clone(),
            account: host.account.clone(),
            readonly: host.readonly,
            local_folder: host.local_folder.clone(),
            initial_remote_folder: display_initial(&host.initial_remote_folder),
            initial_path: host.initial_path.clone(),
        }
    }

    fn from_draft(draft: &Draft) -> Self {
        Self {
            host_index: None,
            label: draft.label(),
            address: draft.address.clone(),
            account: draft.account.clone(),
            readonly: draft.readonly,
            local_folder: draft.local_folder.clone(),
            initial_remote_folder: display_initial(&draft.initial_remote_folder),
            initial_path: draft.initial_path(),
        }
    }
}

struct Prototype {
    screen: Screen,
    tab_edge: TabEdge,
    tree: Node,
    hosts: Vec<Host>,
    dialog: Option<Page>,
    draft: Draft,
    open: Option<OpenSession>,
    folder_path: String,
    selection: Option<String>,
    download_note: Option<String>,
}

impl Prototype {
    fn new() -> Self {
        Self {
            screen: Screen::Launch,
            tab_edge: TabEdge::Top,
            tree: sample_tree(),
            hosts: sample_hosts(),
            dialog: None,
            draft: Draft::default(),
            open: None,
            folder_path: "/home/mnj".to_string(),
            selection: None,
            download_note: None,
        }
    }

    fn session_open(&self) -> bool {
        self.screen != Screen::Launch && self.open.is_some()
    }

    fn ensure_shown_session(&mut self) {
        if self.screen == Screen::Launch || self.open.is_some() {
            return;
        }
        let screen = self.screen;
        self.open_saved(0);
        self.screen = screen;
    }

    fn open_hosts_dialog(&mut self) {
        if self.dialog.is_some() {
            return;
        }
        let page = match &self.open {
            Some(session) => match session.host_index {
                Some(index) => Page::Host(index),
                None => Page::Unsaved,
            },
            None if self.hosts.is_empty() => Page::New,
            None => Page::Host(0),
        };
        self.load_page(page);
    }

    fn load_page(&mut self, page: Page) {
        self.draft = match page {
            Page::Host(index) => self
                .hosts
                .get(index)
                .map(Draft::from_host)
                .unwrap_or_default(),
            Page::New => Draft::default(),
            Page::Unsaved => self.unsaved_draft(),
        };
        self.dialog = Some(page);
    }

    fn unsaved_draft(&self) -> Draft {
        let Some(session) = &self.open else {
            return Draft::default();
        };
        if session.host_index.is_some() {
            return Draft::default();
        }
        Draft {
            display_name: session.label.clone(),
            address: session.address.clone(),
            account: session.account.clone(),
            key_file: String::new(),
            readonly: session.readonly,
            remember: false,
            local_folder: session.local_folder.clone(),
            initial_remote_folder: if session.initial_remote_folder == "(home)" {
                String::new()
            } else {
                session.initial_remote_folder.clone()
            },
        }
    }

    fn open_saved(&mut self, index: usize) {
        let Some(host) = self.hosts.get(index) else {
            return;
        };
        let path = host.initial_path.clone();
        self.open = Some(OpenSession::from_host(index, host));
        self.folder_path = path;
        self.selection = None;
        self.download_note = None;
        self.screen = Screen::Tree;
    }

    fn open_unsaved(&mut self) {
        let session = OpenSession::from_draft(&self.draft);
        self.folder_path = session.initial_path.clone();
        self.open = Some(session);
        self.selection = None;
        self.download_note = None;
        self.screen = Screen::Tree;
    }

    fn write_host(&mut self, index: usize) {
        let Some(host) = self.hosts.get_mut(index) else {
            return;
        };
        *host = self.draft.to_host();
        if self
            .open
            .as_ref()
            .is_some_and(|session| session.host_index == Some(index))
        {
            self.open = Some(OpenSession::from_host(index, host));
        }
    }

    fn apply_dialog(&mut self, event: DialogEvent) {
        match event {
            DialogEvent::None => {}
            DialogEvent::Close => self.dialog = None,
            DialogEvent::Select(page) => {
                if self.dialog != Some(page) {
                    self.load_page(page);
                }
            }
            DialogEvent::Save => self.commit_dialog(false),
            DialogEvent::OpenSession => self.commit_dialog(true),
        }
    }

    fn commit_dialog(&mut self, open: bool) {
        let Some(page) = self.dialog else {
            return;
        };
        match page {
            Page::Host(index) => {
                self.write_host(index);
                if open {
                    self.open_saved(index);
                    self.dialog = None;
                }
            }
            Page::New => {
                self.hosts.push(self.draft.to_host());
                let index = self.hosts.len() - 1;
                self.dialog = Some(Page::Host(index));
                if open {
                    self.open_saved(index);
                    self.dialog = None;
                }
            }
            Page::Unsaved => {
                if open {
                    self.open_unsaved();
                    self.dialog = None;
                }
            }
        }
    }
}

impl eframe::App for Prototype {
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        apply_theme(ctx);
        self.chrome_viewport(ctx);
        self.ensure_shown_session();
        self.main_window(ctx);
        self.host_dialog(ctx);
    }
}

impl Prototype {
    fn chrome_viewport(&mut self, parent: &egui::Context) {
        parent.show_viewport_immediate(
            egui::ViewportId::from_hash_of("layout-prototype-chrome"),
            egui::ViewportBuilder::default()
                .with_title("Prototype")
                .with_inner_size([420.0, 460.0])
                .with_always_on_top(),
            |ctx, class| {
                apply_theme(ctx);
                if ctx.input(|input| input.viewport().close_requested()) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                }
                let mut fill = |ui: &mut egui::Ui| self.chrome_ui(ui);
                if class == egui::ViewportClass::Embedded {
                    egui::Window::new("Prototype")
                        .default_pos(egui::pos2(24.0, 24.0))
                        .show(ctx, |ui| fill(ui));
                } else {
                    egui::CentralPanel::default()
                        .frame(panel_frame(ctx, APP_BG))
                        .show(ctx, |ui| fill(ui));
                }
            },
        );
    }

    fn chrome_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Prototype").strong());
        ui.label(
            RichText::new("Not the app. Host config is the dialog on the sshexplorer window.")
                .color(WEAK),
        );
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.label("Show");
            for screen in [Screen::Launch, Screen::Tree, Screen::Folder] {
                if pill(ui, screen.label(), self.screen == screen) {
                    self.screen = screen;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Tab bar");
            for edge in TabEdge::ALL {
                if pill(ui, edge.label(), self.tab_edge == edge) {
                    self.tab_edge = edge;
                }
            }
        });
        ui.label(
            RichText::new(
                "Launch draws no tab bar. Tree view and Folder view use the edge above. Default is top.",
            )
            .color(WEAK)
            .size(12.0),
        );
        ui.separator();
        ui.label(RichText::new("Left unset").color(WEAK).size(12.0));
        ui.label(
            RichText::new(
                "Search, Session keyboard bindings, preview formats, how authentication combines, and what Download does when the local name exists.",
            )
            .color(WEAK)
            .size(12.0),
        );
        ui.separator();
        ui.label(RichText::new(self.state_text()).monospace().size(12.0));
    }

    fn state_text(&self) -> String {
        let mut lines = vec![
            format!("screen: {}", self.screen.label()),
            format!("tab bar: {}", self.tab_edge.label().to_lowercase()),
            format!(
                "dialog: {}",
                self.dialog
                    .map(|page| page.title(&self.hosts))
                    .unwrap_or_else(|| "closed".to_string())
            ),
        ];
        match &self.open {
            None => lines.push("session: none".to_string()),
            Some(session) => {
                let shown = if self.screen == Screen::Launch {
                    "held, not shown"
                } else {
                    "on screen"
                };
                lines.push(format!("session ({shown}): {}", session.label));
                lines.push(format!("readonly: {}", on_off(session.readonly)));
                lines.push(format!("local folder: {}", session.local_folder));
            }
        }
        lines.push(format!(
            "selection: {}",
            self.selection.as_deref().unwrap_or("nothing")
        ));
        lines.push(format!(
            "download: {}",
            self.download_note.as_deref().unwrap_or("not pressed")
        ));
        lines.join("\n")
    }

    fn main_window(&mut self, ctx: &egui::Context) {
        if self.session_open() {
            self.tab_bar_panel(ctx, "tab");
        }

        let mut open_dialog = false;
        let mut pressed_download = false;
        egui::TopBottomPanel::top("toolbar")
            .frame(panel_frame(ctx, APP_BG))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if quiet_button(ui, "Hosts") {
                        open_dialog = true;
                    }
                    if self.session_open() {
                        ui.add_space(8.0);
                        if pill(ui, "Tree view", self.screen == Screen::Tree) {
                            self.screen = Screen::Tree;
                        }
                        if pill(ui, "Folder view", self.screen == Screen::Folder) {
                            self.screen = Screen::Folder;
                        }
                        ui.add_space(8.0);
                        ui.label(RichText::new(self.place_label()).color(WEAK));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_enabled_ui(self.selection.is_some(), |ui| {
                                pressed_download = quiet_button(ui, "Download");
                            });
                            if let Some(session) = &self.open {
                                ui.label(
                                    RichText::new(format!("Local folder {}", session.local_folder))
                                        .color(WEAK),
                                );
                                ui.label(
                                    RichText::new(format!("Readonly {}", on_off(session.readonly)))
                                        .color(WEAK),
                                );
                            }
                        });
                    }
                });
            });

        if self.session_open() {
            egui::SidePanel::right("preview")
                .default_width(300.0)
                .frame(panel_frame(ctx, PANEL))
                .show(ctx, |ui| self.preview(ui));
        }

        egui::CentralPanel::default()
            .frame(panel_frame(ctx, APP_BG))
            .show(ctx, |ui| {
                if !self.session_open() {
                    self.launch_empty(ui);
                    return;
                }
                self.listing(ui);
            });

        if open_dialog {
            self.open_hosts_dialog();
        }
        if pressed_download {
            self.note_download();
        }
    }

    fn launch_empty(&self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(180.0);
            ui.label(RichText::new("No Session is open").size(16.0));
            ui.add_space(6.0);
            ui.label(RichText::new("Open Hosts to choose or configure a Host.").color(WEAK));
        });
    }

    fn host_dialog(&mut self, ctx: &egui::Context) {
        if self.dialog.is_none() {
            return;
        }
        let style = ctx.style();
        let frame = Frame::popup(style.as_ref())
            .inner_margin(0)
            .corner_radius(12)
            .fill(DIALOG)
            .stroke(Stroke::new(1.0, BORDER))
            .shadow(egui::Shadow {
                offset: [0, 16],
                blur: 40,
                spread: 4,
                color: Color32::from_black_alpha(200),
            });
        let mut event = DialogEvent::None;
        let response = egui::Modal::new(egui::Id::new("host-config"))
            .frame(frame)
            .backdrop_color(Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                event = self.dialog_ui(ui);
            });
        if response.should_close() {
            event = DialogEvent::Close;
        }
        self.apply_dialog(event);
    }

    fn dialog_ui(&mut self, ui: &mut egui::Ui) -> DialogEvent {
        ui.set_width(760.0);
        ui.set_min_height(520.0);
        let mut event = DialogEvent::None;

        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 44.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(16.0);
                ui.label(RichText::new("Host").size(14.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(10.0);
                    if close_button(ui) {
                        event = DialogEvent::Close;
                    }
                });
            },
        );
        hairline(ui);

        let body_height = 476.0;
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(220.0, body_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    if let Some(selected) = self.nav(ui) {
                        event = DialogEvent::Select(selected);
                    }
                },
            );
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(1.0, body_height), egui::Sense::hover());
            ui.painter().rect_filled(rect, 0.0, BORDER);
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), body_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    if let Some(next) = self.dialog_form(ui) {
                        event = next;
                    }
                },
            );
        });

        event
    }

    fn nav(&mut self, ui: &mut egui::Ui) -> Option<Page> {
        let page = self.dialog;
        let mut selected = None;
        egui::ScrollArea::vertical()
            .id_salt("host-nav")
            .show(ui, |ui| {
                Frame::new()
                    .inner_margin(Margin::symmetric(8, 10))
                    .show(ui, |ui| {
                        ui.label(RichText::new("Saved Hosts").size(11.0).color(WEAK));
                        ui.add_space(4.0);
                        for (index, host) in self.hosts.iter().enumerate() {
                            let name = if host.display_name.is_empty() {
                                host.address.as_str()
                            } else {
                                host.display_name.as_str()
                            };
                            let name = if name.is_empty() { "Host" } else { name };
                            if nav_row(ui, name, page == Some(Page::Host(index))).clicked() {
                                selected = Some(Page::Host(index));
                            }
                        }
                        ui.add_space(12.0);
                        if nav_row(ui, "New Host", page == Some(Page::New)).clicked() {
                            selected = Some(Page::New);
                        }
                        if nav_row(ui, "Unsaved Session", page == Some(Page::Unsaved)).clicked() {
                            selected = Some(Page::Unsaved);
                        }
                    });
            });
        selected
    }

    fn dialog_form(&mut self, ui: &mut egui::Ui) -> Option<DialogEvent> {
        let page = self.dialog?;
        let title = page.title(&self.hosts);
        let saved = !matches!(page, Page::Unsaved);
        let mut event = None;
        egui::ScrollArea::vertical()
            .id_salt("host-form")
            .show(ui, |ui| {
                Frame::new()
                    .inner_margin(Margin::symmetric(22, 16))
                    .show(ui, |ui| {
                        ui.label(RichText::new(title).size(18.0).strong());
                        ui.add_space(12.0);
                        field(ui, "Display name", &mut self.draft.display_name);
                        field(ui, "Address", &mut self.draft.address);
                        field(ui, "Account", &mut self.draft.account);
                        field(ui, "Key file", &mut self.draft.key_file);
                        field(ui, "Local folder", &mut self.draft.local_folder);
                        field(
                            ui,
                            "Initial remote folder",
                            &mut self.draft.initial_remote_folder,
                        );
                        ui.label(RichText::new("Empty initial remote folder means the account's home directory.").color(WEAK).size(12.0));
                        ui.add_space(4.0);
                        ui.checkbox(&mut self.draft.readonly, "Readonly");
                        if saved {
                            ui.checkbox(&mut self.draft.remember, "Remember");
                        } else {
                            ui.label(
                                RichText::new("Remember applies to a saved Host. An unsaved Session does not use the credential store.")
                                    .color(WEAK)
                                    .size(12.0),
                            );
                        }
                        ui.label(RichText::new("Authentication is not arranged in this prototype.").color(WEAK).size(12.0));
                        ui.add_space(16.0);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if primary_button(ui, "Open Session") {
                                event = Some(DialogEvent::OpenSession);
                            }
                            if saved && quiet_button(ui, "Save") {
                                event = Some(DialogEvent::Save);
                            }
                        });
                    });
            });
        event
    }

    fn tab_bar_panel(&self, ctx: &egui::Context, id: &'static str) {
        let label = self
            .open
            .as_ref()
            .map(|session| session.label.clone())
            .unwrap_or_else(|| "Session".to_string());
        let draw = |ui: &mut egui::Ui| {
            let _ = pill(ui, &label, true);
        };
        match self.tab_edge {
            TabEdge::Top => {
                egui::TopBottomPanel::top(id)
                    .frame(panel_frame(ctx, APP_BG))
                    .show(ctx, |ui| {
                        ui.horizontal(draw);
                    });
            }
            TabEdge::Bottom => {
                egui::TopBottomPanel::bottom(id)
                    .frame(panel_frame(ctx, APP_BG))
                    .show(ctx, |ui| {
                        ui.horizontal(draw);
                    });
            }
            TabEdge::Left => {
                egui::SidePanel::left(id)
                    .resizable(true)
                    .default_width(160.0)
                    .width_range(120.0..=280.0)
                    .frame(panel_frame(ctx, APP_BG))
                    .show(ctx, |ui| {
                        ui.vertical(draw);
                    });
            }
            TabEdge::Right => {
                egui::SidePanel::right(id)
                    .resizable(true)
                    .default_width(160.0)
                    .width_range(120.0..=280.0)
                    .frame(panel_frame(ctx, APP_BG))
                    .show(ctx, |ui| {
                        ui.vertical(draw);
                    });
            }
        }
    }

    fn place_label(&self) -> String {
        match self.screen {
            Screen::Folder => format!("Folder view · {}", self.folder_path),
            Screen::Tree => {
                let initial = self
                    .open
                    .as_ref()
                    .map(|session| session.initial_path.as_str())
                    .unwrap_or("/");
                format!("Tree view · Initial remote folder {initial}")
            }
            Screen::Launch => "Launch".to_string(),
        }
    }

    fn listing(&mut self, ui: &mut egui::Ui) {
        let screen = self.screen;
        let initial = self
            .open
            .as_ref()
            .map(|session| session.initial_path.clone())
            .unwrap_or_else(|| "/".to_string());
        egui::ScrollArea::vertical()
            .id_salt("listing")
            .show(ui, |ui| match screen {
                Screen::Tree => show_tree(ui, &self.tree, "/", &initial, &mut self.selection),
                Screen::Folder => {
                    show_folder(ui, &self.tree, &mut self.folder_path, &mut self.selection);
                }
                Screen::Launch => {}
            });
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Preview").size(14.0).strong());
        ui.add_space(8.0);
        if !self.session_open() {
            ui.label(RichText::new("No Session is open.").color(WEAK));
            return;
        }
        match &self.selection {
            None => {
                ui.label(RichText::new("Nothing selected.").color(WEAK));
            }
            Some(path) => {
                let kind = if path_is_dir(&self.tree, path) {
                    "Directory"
                } else {
                    "File"
                };
                ui.label(kind);
                ui.add_space(4.0);
                ui.label(RichText::new(path).color(WEAK));
                ui.add_space(12.0);
                ui.label(RichText::new("Preview format is unset.").color(WEAK));
            }
        }
    }

    fn note_download(&mut self) {
        let Some(path) = &self.selection else {
            return;
        };
        let local = self
            .open
            .as_ref()
            .map(|session| session.local_folder.as_str())
            .unwrap_or("~/Downloads");
        self.download_note = Some(format!(
            "pressed for {path} into {local}; the existing-name case is unset"
        ));
    }
}

fn panel_frame(ctx: &egui::Context, fill: Color32) -> Frame {
    let style = ctx.style();
    Frame::side_top_panel(style.as_ref())
        .fill(fill)
        .inner_margin(Margin::symmetric(10, 6))
}

fn hairline(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, BORDER);
}

fn nav_row(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 30.0), egui::Sense::click());
    let fill = if selected {
        SELECT
    } else if response.hovered() {
        HOVER
    } else {
        Color32::TRANSPARENT
    };
    if fill != Color32::TRANSPARENT {
        ui.painter()
            .rect_filled(rect.shrink2(Vec2::new(4.0, 1.0)), 6.0, fill);
    }
    let color = if selected { Color32::WHITE } else { TEXT };
    let font = FontId::proportional(13.5);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, color);
    let pos = egui::pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, color);
    response
}

fn pill(ui: &mut egui::Ui, label: &str, selected: bool) -> bool {
    let font = FontId::proportional(13.0);
    let color = if selected { Color32::WHITE } else { TEXT };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, color);
    let width = galley.size().x + 20.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 26.0), egui::Sense::click());
    if selected || response.hovered() {
        let fill = if selected { SELECT } else { HOVER };
        ui.painter().rect_filled(rect, 6.0, fill);
    }
    let pos = rect.center() - galley.size() / 2.0;
    ui.painter().galley(pos, galley, color);
    response.clicked()
}

fn quiet_button(ui: &mut egui::Ui, label: &str) -> bool {
    ui.scope(|ui| {
        let visuals = ui.visuals_mut();
        visuals.widgets.inactive.weak_bg_fill = RAISED;
        visuals.widgets.inactive.bg_stroke = Stroke::NONE;
        visuals.widgets.hovered.weak_bg_fill = HOVER;
        visuals.widgets.hovered.bg_stroke = Stroke::NONE;
        visuals.widgets.active.weak_bg_fill = SELECT;
        visuals.widgets.active.bg_stroke = Stroke::NONE;
        ui.button(label).clicked()
    })
    .inner
}

fn primary_button(ui: &mut egui::Ui, label: &str) -> bool {
    ui.scope(|ui| {
        let light = Color32::from_rgb(236, 236, 236);
        let dark = Color32::from_rgb(20, 20, 20);
        let visuals = ui.visuals_mut();
        visuals.widgets.inactive.weak_bg_fill = light;
        visuals.widgets.inactive.fg_stroke.color = dark;
        visuals.widgets.inactive.bg_stroke = Stroke::NONE;
        visuals.widgets.hovered.weak_bg_fill = Color32::WHITE;
        visuals.widgets.hovered.fg_stroke.color = dark;
        visuals.widgets.hovered.bg_stroke = Stroke::NONE;
        visuals.widgets.active.weak_bg_fill = Color32::from_rgb(210, 210, 210);
        visuals.widgets.active.fg_stroke.color = dark;
        visuals.widgets.active.bg_stroke = Stroke::NONE;
        ui.button(label).clicked()
    })
    .inner
}

fn close_button(ui: &mut egui::Ui) -> bool {
    ui.scope(|ui| {
        let visuals = ui.visuals_mut();
        visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        visuals.widgets.inactive.bg_stroke = Stroke::NONE;
        visuals.widgets.hovered.weak_bg_fill = HOVER;
        visuals.widgets.hovered.bg_stroke = Stroke::NONE;
        visuals.widgets.active.weak_bg_fill = SELECT;
        visuals.widgets.active.bg_stroke = Stroke::NONE;
        ui.add(egui::Button::new(RichText::new("×").size(16.0)).frame(true))
            .clicked()
    })
    .inner
}

fn field(ui: &mut egui::Ui, label: &str, text: &mut String) {
    ui.label(RichText::new(label).size(13.0));
    ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(ui.available_width())
            .margin(Margin::symmetric(10, 6)),
    );
    ui.add_space(2.0);
}

fn show_tree(
    ui: &mut egui::Ui,
    node: &Node,
    path: &str,
    initial: &str,
    selection: &mut Option<String>,
) {
    let name = if node.name == "/" {
        "/"
    } else {
        node.name.as_str()
    };
    let title = if path == initial {
        format!("{name}  ·  Initial remote folder")
    } else {
        name.to_string()
    };
    if node.dir {
        let response = egui::CollapsingHeader::new(title)
            .id_salt(path)
            .default_open(is_ancestor(path, initial))
            .show(ui, |ui| {
                for child in &node.children {
                    let child_path = join_path(path, &child.name);
                    show_tree(ui, child, &child_path, initial, selection);
                }
            });
        if response.header_response.clicked() {
            *selection = Some(path.to_string());
        }
    } else if ui
        .selectable_label(selection.as_deref() == Some(path), title)
        .clicked()
    {
        *selection = Some(path.to_string());
    }
}

fn show_folder(
    ui: &mut egui::Ui,
    tree: &Node,
    folder_path: &mut String,
    selection: &mut Option<String>,
) {
    let current = folder_path.clone();
    folder_row(ui, ".", &current, true, selection, folder_path);
    let parent = parent_path(&current);
    folder_row(ui, "..", &parent, true, selection, folder_path);
    let Some(node) = find(tree, "/", &current) else {
        ui.label(RichText::new("That directory is not in this throwaway tree.").color(WEAK));
        return;
    };
    for child in &node.children {
        let path = join_path(&current, &child.name);
        folder_row(ui, &child.name, &path, child.dir, selection, folder_path);
    }
}

fn folder_row(
    ui: &mut egui::Ui,
    name: &str,
    path: &str,
    is_dir: bool,
    selection: &mut Option<String>,
    folder_path: &mut String,
) {
    ui.horizontal(|ui| {
        let label = if is_dir && name != "." && name != ".." {
            format!("{name}/")
        } else {
            name.to_string()
        };
        let selected = selection.as_deref() == Some(path);
        if pill(ui, &label, selected) {
            *selection = Some(path.to_string());
        }
        if is_dir && name != "." && quiet_button(ui, "Open") {
            *folder_path = path.to_string();
            *selection = Some(path.to_string());
        }
    });
}

fn find<'a>(node: &'a Node, node_path: &str, want: &str) -> Option<&'a Node> {
    if node_path == want {
        return Some(node);
    }
    if !is_ancestor(node_path, want) {
        return None;
    }
    for child in &node.children {
        let child_path = join_path(node_path, &child.name);
        if let Some(found) = find(child, &child_path, want) {
            return Some(found);
        }
    }
    None
}

fn path_is_dir(tree: &Node, path: &str) -> bool {
    find(tree, "/", path).is_some_and(|node| node.dir)
}

fn join_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

fn parent_path(path: &str) -> String {
    if path == "/" {
        return "/".to_string();
    }
    match path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(index) => path[..index].to_string(),
    }
}

fn is_ancestor(path: &str, descendant: &str) -> bool {
    path == "/" || path == descendant || descendant.starts_with(&format!("{path}/"))
}

fn display_initial(folder: &str) -> String {
    if folder.is_empty() {
        "(home)".to_string()
    } else {
        folder.to_string()
    }
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}
