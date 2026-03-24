use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::w;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, PostMessageW, SetForegroundWindow, WM_USER,
};

use eframe::egui;

use crate::config::{Config, PipEdge};

/// Custom message posted to the tray window after settings are saved.
pub const WM_SETTINGS_CHANGED: u32 = WM_USER + 100;

const HOTKEY_OPTIONS: &[&str] = &[
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
    "Pause", "ScrollLock", "Insert", "Delete", "Home", "End", "PageUp", "PageDown",
];

const PIP_EDGE_OPTIONS: &[(&str, PipEdge)] = &[
    ("Right", PipEdge::Right),
    ("Left", PipEdge::Left),
    ("Top", PipEdge::Top),
    ("Bottom", PipEdge::Bottom),
];

/// Whether a settings subprocess is currently running.
static SETTINGS_OPEN: AtomicBool = AtomicBool::new(false);

/// Show the settings window by spawning a subprocess.
/// If already open, brings the existing window to the foreground.
pub fn show() {
    if SETTINGS_OPEN.load(Ordering::SeqCst) {
        // Already open — try to focus the existing window.
        unsafe {
            if let Ok(hwnd) = FindWindowW(None, w!("Stonemite Settings")) {
                let _ = SetForegroundWindow(hwnd);
            }
        }
        return;
    }

    // Spawn ourselves with --settings flag.
    let exe = std::env::current_exe().expect("Failed to get current exe path");
    match std::process::Command::new(&exe).arg("--settings").spawn() {
        Ok(mut child) => {
            SETTINGS_OPEN.store(true, Ordering::SeqCst);
            // Wait for the child to exit in a background thread.
            std::thread::spawn(move || {
                let _ = child.wait();
                SETTINGS_OPEN.store(false, Ordering::SeqCst);
            });
        }
        Err(e) => eprintln!("Failed to open settings: {e}"),
    }
}

/// Entry point for the `--settings` subprocess. Runs eframe on the main thread.
pub fn run_standalone() {
    let cfg = Config::load();
    let app = SettingsApp::from_config(&cfg);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Stonemite Settings")
            .with_inner_size([480.0, 400.0])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_icon(load_app_icon()),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "Stonemite Settings",
        options,
        Box::new(|cc| {
            configure_fonts(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    );
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tab {
    General,
    Hotkeys,
    Broadcasting,
    About,
}

fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if let Some(data) = load_system_font() {
        fonts.font_data.insert(
            "system".to_owned(),
            egui::FontData::from_owned(data).into(),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "system".to_owned());
    }
    ctx.set_fonts(fonts);
}

fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();

    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::proportional(12.0),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::proportional(12.0),
    );
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::proportional(12.0),
    );

    style.spacing.item_spacing = egui::vec2(6.0, 5.0);
    style.spacing.button_padding = egui::vec2(16.0, 4.0);
    style.spacing.combo_width = 120.0;

    let r = egui::CornerRadius::same(2);
    let border = egui::Color32::from_gray(190);
    let light_fill = egui::Color32::from_gray(251);
    let hover_fill = egui::Color32::from_gray(243);
    let active_fill = egui::Color32::from_gray(235);
    let text = egui::Color32::from_gray(30);

    style.visuals.widgets.inactive.corner_radius = r;
    style.visuals.widgets.inactive.bg_fill = light_fill;
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, border);
    style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, text);

    style.visuals.widgets.hovered.corner_radius = r;
    style.visuals.widgets.hovered.bg_fill = hover_fill;
    style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(160));
    style.visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, text);

    style.visuals.widgets.active.corner_radius = r;
    style.visuals.widgets.active.bg_fill = active_fill;
    style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(140));
    style.visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, text);

    style.visuals.widgets.noninteractive.corner_radius = r;
    style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, text);

    style.visuals.extreme_bg_color = egui::Color32::WHITE;

    style.visuals.selection.bg_fill = egui::Color32::from_rgb(204, 224, 255);
    style.visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(0, 95, 184));

    ctx.set_style(style);
}

struct SettingsApp {
    tab: Tab,
    eq_dir: String,
    hotkey_index: usize,
    edge_index: usize,
    logo: Option<egui::TextureHandle>,
    avatar: Option<egui::TextureHandle>,
}

impl SettingsApp {
    fn from_config(cfg: &Config) -> Self {
        let hotkey_index = HOTKEY_OPTIONS
            .iter()
            .position(|k| k.eq_ignore_ascii_case(cfg.hide_hotkey.trim()))
            .unwrap_or(8);

        let edge_index = PIP_EDGE_OPTIONS
            .iter()
            .position(|(_, e)| *e == cfg.pip_edge)
            .unwrap_or(0);

        Self {
            tab: Tab::General,
            eq_dir: cfg.eq_dir.clone(),
            hotkey_index,
            edge_index,
            logo: None,
            avatar: None,
        }
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        configure_style(ctx);

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::General, "General");
                ui.selectable_value(&mut self.tab, Tab::Hotkeys, "Hotkeys");
                ui.selectable_value(&mut self.tab, Tab::Broadcasting, "Broadcasting");
                ui.selectable_value(&mut self.tab, Tab::About, "About");
            });
            ui.add_space(2.0);
        });

        egui::TopBottomPanel::bottom("buttons")
            .min_height(40.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("  Save  ").clicked() {
                        self.save_config();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.tab {
                Tab::General => self.general_tab(ui),
                Tab::Hotkeys => self.hotkeys_tab(ui),
                Tab::Broadcasting => self.broadcasting_tab(ui),
                Tab::About => self.about_tab(ui),
            }
        });
    }
}

impl SettingsApp {
    fn general_tab(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);

        section(ui, "EverQuest directory", |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.eq_dir)
                        .desired_width(ui.available_width() - 88.0),
                );
                if ui.button("Browse...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_directory(&self.eq_dir)
                        .pick_folder()
                    {
                        self.eq_dir = path.display().to_string();
                    }
                }
            });
        });

        section(ui, "PiP edge", |ui| {
            ui.label("Screen edge where PiP thumbnails are anchored");
            egui::ComboBox::from_id_salt("pip_edge")
                .selected_text(PIP_EDGE_OPTIONS[self.edge_index].0)
                .show_ui(ui, |ui| {
                    for (i, (label, _)) in PIP_EDGE_OPTIONS.iter().enumerate() {
                        ui.selectable_value(&mut self.edge_index, i, *label);
                    }
                });
        });

        section(ui, "Hide overlay hotkey", |ui| {
            ui.label("Toggle PiP overlay visibility while EQ is focused");
            egui::ComboBox::from_id_salt("hotkey")
                .selected_text(HOTKEY_OPTIONS[self.hotkey_index])
                .show_ui(ui, |ui| {
                    for (i, key) in HOTKEY_OPTIONS.iter().enumerate() {
                        ui.selectable_value(&mut self.hotkey_index, i, *key);
                    }
                });
        });
    }

    fn hotkeys_tab(&self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.colored_label(
            ui.visuals().weak_text_color(),
            "Hotkey configuration coming soon.",
        );
    }

    fn broadcasting_tab(&self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.colored_label(
            ui.visuals().weak_text_color(),
            "Broadcasting settings coming soon.",
        );
    }

    fn about_tab(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);

        let logo = self.logo.get_or_insert_with(|| {
            let png_data = include_bytes!("../assets/app.png");
            let img = image::load_from_memory(png_data).expect("Failed to load logo");
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [w as usize, h as usize],
                &rgba.into_raw(),
            );
            ui.ctx().load_texture("logo", color_image, egui::TextureOptions::LINEAR)
        });

        let logo_size = egui::vec2(48.0, 48.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.strong(format!("Stonemite v{}", env!("CARGO_PKG_VERSION")));
                ui.add_space(2.0);
                ui.label("EverQuest multiboxing PiP overlay tool");
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                ui.image(egui::load::SizedTexture::new(logo.id(), logo_size));
            });
        });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        let avatar = self.avatar.get_or_insert_with(|| {
            let png_data = include_bytes!("../assets/author.png");
            let img = image::load_from_memory(png_data).expect("Failed to load avatar");
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [w as usize, h as usize],
                &rgba.into_raw(),
            );
            ui.ctx().load_texture("avatar", color_image, egui::TextureOptions::LINEAR)
        });

        let avatar_size = egui::vec2(48.0, 48.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label("Author: Laika");
                ui.horizontal(|ui| {
                    ui.label("GitHub:");
                    ui.hyperlink("https://github.com/eqlaika/stonemite");
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                let (rect, _) = ui.allocate_exact_size(avatar_size, egui::Sense::hover());
                let rounding = egui::CornerRadius::same(6);
                ui.painter().add(egui::epaint::RectShape::filled(
                    rect,
                    rounding,
                    egui::Color32::WHITE,
                ).with_texture(
                    avatar.id(),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                ));
            });
        });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        ui.strong("Contact");
        ui.label("In-game: /tell Xegony.Laika");
        ui.horizontal(|ui| {
            ui.label("Email:");
            ui.hyperlink_to("laika@laikasoft.co", "mailto:laika@laikasoft.co");
        });
    }

    fn save_config(&self) {
        let existing = Config::load();
        let cfg = Config {
            eq_dir: self.eq_dir.clone(),
            hide_hotkey: HOTKEY_OPTIONS[self.hotkey_index].to_string(),
            pip_edge: PIP_EDGE_OPTIONS[self.edge_index].1,
            pip_strip_width: existing.pip_strip_width,
            pip_positions: existing.pip_positions,
            snap_grid: existing.snap_grid,
            telemetry: existing.telemetry,
            telemetry_id: existing.telemetry_id,
        };
        if let Err(e) = cfg.save() {
            eprintln!("Failed to save config: {e}");
        }
        notify_tray();
    }
}

fn section(ui: &mut egui::Ui, heading: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.strong(heading);
    ui.indent(heading, |ui| {
        content(ui);
    });
    ui.add_space(6.0);
}

fn load_app_icon() -> egui::IconData {
    let ico_data = include_bytes!("../assets/app.ico");
    let count = u16::from_le_bytes([ico_data[4], ico_data[5]]) as usize;
    let mut best = (0usize, 0u32);
    for i in 0..count {
        let off = 6 + i * 16;
        let w = if ico_data[off] == 0 { 256 } else { ico_data[off] as u32 };
        if w > best.1 {
            best = (i, w);
        }
    }
    let entry_off = 6 + best.0 * 16;
    let data_size = u32::from_le_bytes([
        ico_data[entry_off + 8], ico_data[entry_off + 9],
        ico_data[entry_off + 10], ico_data[entry_off + 11],
    ]) as usize;
    let data_offset = u32::from_le_bytes([
        ico_data[entry_off + 12], ico_data[entry_off + 13],
        ico_data[entry_off + 14], ico_data[entry_off + 15],
    ]) as usize;
    let png_data = &ico_data[data_offset..data_offset + data_size];

    if let Ok(img) = image::load_from_memory(png_data) {
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        return egui::IconData {
            rgba: rgba.into_raw(),
            width: w,
            height: h,
        };
    }
    egui::IconData { rgba: vec![0; 4], width: 1, height: 1 }
}

fn load_system_font() -> Option<Vec<u8>> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".to_string());
    let fonts_dir = std::path::Path::new(&windir).join("Fonts");
    for filename in ["SegUIVar.ttf", "segoeui.ttf", "tahoma.ttf"] {
        if let Ok(data) = std::fs::read(fonts_dir.join(filename)) {
            return Some(data);
        }
    }
    None
}

fn notify_tray() {
    unsafe {
        if let Ok(tray) = FindWindowW(w!("StonemiteTrayClass"), w!("Stonemite")) {
            let _ = PostMessageW(tray, WM_SETTINGS_CHANGED, WPARAM(0), LPARAM(0));
        }
    }
}
