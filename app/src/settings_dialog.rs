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
            .with_icon(load_app_icon())
            .with_position(cfg.settings_position
                .map(|p| egui::pos2(p[0], p[1]))
                .unwrap_or_else(|| centered_position(480.0, 400.0))),
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
    PiP,
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

const FILTER_MODE_OPTIONS: &[(&str, &str)] = &[
    ("Blacklist", "blacklist"),
    ("Whitelist", "whitelist"),
];

struct SettingsApp {
    tab: Tab,
    eq_dir: String,
    hide_hotkey: String,
    capturing_hotkey: bool,
    broadcast_hotkey: String,
    capturing_broadcast_hotkey: bool,
    swap_hotkeys: [String; 6],
    capturing_swap_hotkey: Option<usize>,
    filter_mode_index: usize,
    filter_keys_text: String,
    edge_index: usize,
    label_height: u32,
    label_opacity: u32,
    toast_enabled: bool,
    toast_height: u32,
    toast_duration_tenths: u32,
    auto_update_check: bool,
    update_check_interval_days: u32,
    last_position: Option<[f32; 2]>,
    logo: Option<egui::TextureHandle>,
    avatar: Option<egui::TextureHandle>,
}

impl SettingsApp {
    fn from_config(cfg: &Config) -> Self {
        let edge_index = PIP_EDGE_OPTIONS
            .iter()
            .position(|(_, e)| *e == cfg.pip_edge)
            .unwrap_or(0);

        let filter_mode_index = FILTER_MODE_OPTIONS
            .iter()
            .position(|(_, v)| *v == cfg.broadcast_filter_mode)
            .unwrap_or(0);

        let mut swap_hotkeys = [
            "Ctrl+F1".to_string(), "Ctrl+F2".to_string(), "Ctrl+F3".to_string(),
            "Ctrl+F4".to_string(), "Ctrl+F5".to_string(), "Ctrl+F6".to_string(),
        ];
        for (i, s) in cfg.swap_hotkeys.iter().enumerate().take(6) {
            swap_hotkeys[i] = s.clone();
        }

        Self {
            tab: Tab::General,
            eq_dir: cfg.eq_dir.clone(),
            hide_hotkey: cfg.hide_hotkey.clone(),
            capturing_hotkey: false,
            broadcast_hotkey: cfg.broadcast_hotkey.clone(),
            capturing_broadcast_hotkey: false,
            swap_hotkeys,
            capturing_swap_hotkey: None,
            filter_mode_index,
            filter_keys_text: cfg.broadcast_filter_keys.join(", "),
            edge_index,
            label_height: cfg.pip_label_height.unwrap_or(48),
            label_opacity: cfg.pip_label_opacity.unwrap_or(80),
            toast_enabled: cfg.toast_enabled,
            toast_height: cfg.toast_height.unwrap_or(64),
            toast_duration_tenths: cfg.toast_duration.map(|d| (d * 10.0).round() as u32).unwrap_or(20),
            auto_update_check: cfg.auto_update_check,
            update_check_interval_days: cfg.update_check_interval_days,
            last_position: None,
            logo: None,
            avatar: None,
        }
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        configure_style(ctx);

        if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
            self.last_position = Some([rect.min.x, rect.min.y]);
        }

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::General, "General");
                ui.selectable_value(&mut self.tab, Tab::PiP, "PiP");
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
                        self.save_position();
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
                Tab::PiP => self.pip_tab(ui),
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

        section(ui, "Toast notifications", |ui| {
            ui.checkbox(&mut self.toast_enabled, "Enabled");
            ui.horizontal(|ui| {
                ui.label("Height:");
                ui.scope(|ui| {
                    ui.style_mut().visuals.widgets.inactive.bg_fill =
                        egui::Color32::from_gray(220);
                    ui.add(egui::Slider::new(&mut self.toast_height, 24..=128).suffix(" px"));
                });
            });
            ui.horizontal(|ui| {
                ui.label("Duration:");
                ui.scope(|ui| {
                    ui.style_mut().visuals.widgets.inactive.bg_fill =
                        egui::Color32::from_gray(220);
                    ui.add(
                        egui::Slider::new(&mut self.toast_duration_tenths, 5..=100)
                            .custom_formatter(|v, _| format!("{:.1} s", v / 10.0))
                            .custom_parser(|s| {
                                s.trim().trim_end_matches('s').trim().parse::<f64>().ok().map(|v| v * 10.0)
                            }),
                    );
                });
            });
        });

        section(ui, "Updates", |ui| {
            ui.checkbox(&mut self.auto_update_check, "Check automatically on launch");
            ui.horizontal(|ui| {
                ui.label("Check every:");
                ui.scope(|ui| {
                    ui.style_mut().visuals.widgets.inactive.bg_fill =
                        egui::Color32::from_gray(220);
                    ui.add(
                        egui::Slider::new(&mut self.update_check_interval_days, 1..=30)
                            .custom_formatter(|v, _| {
                                let d = v as u32;
                                if d == 1 { "1 day".to_string() } else { format!("{d} days") }
                            })
                            .custom_parser(|s| {
                                s.trim().trim_end_matches("days").trim_end_matches("day").trim().parse::<f64>().ok()
                            }),
                    );
                });
            });
        });
    }

    fn pip_tab(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);

        section(ui, "Layout", |ui| {
            ui.label("Screen edge where PiP thumbnails are anchored");
            egui::ComboBox::from_id_salt("pip_edge")
                .selected_text(PIP_EDGE_OPTIONS[self.edge_index].0)
                .show_ui(ui, |ui| {
                    for (i, (label, _)) in PIP_EDGE_OPTIONS.iter().enumerate() {
                        ui.selectable_value(&mut self.edge_index, i, *label);
                    }
                });
        });

        section(ui, "Labels", |ui| {
            ui.horizontal(|ui| {
                ui.label("Height:");
                ui.scope(|ui| {
                    ui.style_mut().visuals.widgets.inactive.bg_fill =
                        egui::Color32::from_gray(220);
                    ui.add(egui::Slider::new(&mut self.label_height, 24..=64).suffix(" px"));
                });
            });
            ui.horizontal(|ui| {
                ui.label("Opacity:");
                ui.scope(|ui| {
                    ui.style_mut().visuals.widgets.inactive.bg_fill =
                        egui::Color32::from_gray(220);
                    ui.add(egui::Slider::new(&mut self.label_opacity, 10..=100).suffix("%"));
                });
            });
        });

        section(ui, "Hide overlay hotkey", |ui| {
            ui.label("Toggle PiP overlay visibility while EQ is focused");
            ui.horizontal(|ui| {
                if let Some(combo) = hotkey_capture_button(ui, &self.hide_hotkey, &mut self.capturing_hotkey) {
                    self.hide_hotkey = combo;
                }
            });
        });
    }

    fn hotkeys_tab(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);

        section(ui, "Swap to window", |ui| {
            for slot in 0..6 {
                ui.horizontal(|ui| {
                    ui.label(format!("Window {}:", slot + 1));
                    let mut capturing = self.capturing_swap_hotkey == Some(slot);
                    if let Some(combo) = hotkey_capture_button(ui, &self.swap_hotkeys[slot], &mut capturing) {
                        self.swap_hotkeys[slot] = combo;
                    }
                    // Sync the per-slot capturing state back.
                    if capturing && self.capturing_swap_hotkey != Some(slot) {
                        self.capturing_swap_hotkey = Some(slot);
                    } else if !capturing && self.capturing_swap_hotkey == Some(slot) {
                        self.capturing_swap_hotkey = None;
                    }
                });
            }
        });
    }

    fn broadcasting_tab(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);

        section(ui, "Broadcast toggle hotkey", |ui| {
            ui.label("Toggle key broadcasting on/off");
            ui.horizontal(|ui| {
                if let Some(combo) = hotkey_capture_button(ui, &self.broadcast_hotkey, &mut self.capturing_broadcast_hotkey) {
                    self.broadcast_hotkey = combo;
                }
            });
        });

        {
            section(ui, "Key filter", |ui| {
                ui.label("Choose which keys are broadcast to background windows");
                ui.horizontal(|ui| {
                    ui.label("Mode:");
                    egui::ComboBox::from_id_salt("filter_mode")
                        .selected_text(FILTER_MODE_OPTIONS[self.filter_mode_index].0)
                        .show_ui(ui, |ui| {
                            for (i, (label, _)) in FILTER_MODE_OPTIONS.iter().enumerate() {
                                ui.selectable_value(&mut self.filter_mode_index, i, *label);
                            }
                        });
                });
                ui.label("Keys (comma-separated, e.g. Enter, Escape, Tab):");
                ui.add(
                    egui::TextEdit::multiline(&mut self.filter_keys_text)
                        .desired_width(ui.available_width())
                        .desired_rows(3),
                );
            });
        }
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
                ui.label("EverQuest multiboxing tool");
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
        let filter_keys: Vec<String> = self
            .filter_keys_text
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let cfg = Config {
            eq_dir: self.eq_dir.clone(),
            hide_hotkey: self.hide_hotkey.clone(),
            pip_edge: PIP_EDGE_OPTIONS[self.edge_index].1,
            pip_strip_width: existing.pip_strip_width,
            pip_positions: existing.pip_positions,
            snap_grid: existing.snap_grid,
            trusik: existing.trusik,
            swap_hotkeys: self.swap_hotkeys.to_vec(),
            settings_position: self.last_position,
            broadcast_hotkey: self.broadcast_hotkey.clone(),
            broadcast_filter_mode: FILTER_MODE_OPTIONS[self.filter_mode_index].1.to_string(),
            broadcast_filter_keys: filter_keys,
            auto_update_check: self.auto_update_check,
            update_check_interval_days: self.update_check_interval_days,
            last_update_check: existing.last_update_check,
            telemetry: existing.telemetry,
            telemetry_id: existing.telemetry_id,
            pip_label_height: Some(self.label_height),
            pip_label_opacity: Some(self.label_opacity),
            toast_enabled: self.toast_enabled,
            toast_height: Some(self.toast_height),
            toast_duration: Some(self.toast_duration_tenths as f32 / 10.0),
        };
        if let Err(e) = cfg.save() {
            eprintln!("Failed to save config: {e}");
        }

        notify_tray();
    }

    fn save_position(&self) {
        let mut cfg = Config::load();
        cfg.settings_position = self.last_position;
        let _ = cfg.save();
    }
}

/// Map an egui Key to the config key name used by `config::parse_vk_name`.
fn egui_key_to_config_name(key: &egui::Key) -> Option<&'static str> {
    use egui::Key::*;
    match key {
        F1 => Some("F1"),   F2 => Some("F2"),   F3 => Some("F3"),   F4 => Some("F4"),
        F5 => Some("F5"),   F6 => Some("F6"),   F7 => Some("F7"),   F8 => Some("F8"),
        F9 => Some("F9"),   F10 => Some("F10"), F11 => Some("F11"), F12 => Some("F12"),
        Insert => Some("Insert"),
        Delete => Some("Delete"),
        Home => Some("Home"),
        End => Some("End"),
        PageUp => Some("PageUp"),
        PageDown => Some("PageDown"),
        A => Some("A"), B => Some("B"), C => Some("C"), D => Some("D"),
        E => Some("E"), F => Some("F"), G => Some("G"), H => Some("H"),
        I => Some("I"), J => Some("J"), K => Some("K"), L => Some("L"),
        M => Some("M"), N => Some("N"), O => Some("O"), P => Some("P"),
        Q => Some("Q"), R => Some("R"), S => Some("S"), T => Some("T"),
        U => Some("U"), V => Some("V"), W => Some("W"), X => Some("X"),
        Y => Some("Y"), Z => Some("Z"),
        Num0 => Some("0"), Num1 => Some("1"), Num2 => Some("2"), Num3 => Some("3"),
        Num4 => Some("4"), Num5 => Some("5"), Num6 => Some("6"), Num7 => Some("7"),
        Num8 => Some("8"), Num9 => Some("9"),
        Space => Some("Space"),
        Tab => Some("Tab"),
        Minus => Some("Minus"),
        Plus => Some("Plus"),
        Equals => Some("Equals"),
        Backtick => Some("Backtick"),
        OpenBracket => Some("OpenBracket"),
        CloseBracket => Some("CloseBracket"),
        Backslash => Some("Backslash"),
        Semicolon => Some("Semicolon"),
        Quote => Some("Quote"),
        Comma => Some("Comma"),
        Period => Some("Period"),
        Slash => Some("Slash"),
        _ => None,
    }
}

/// Render a hotkey capture button. When `capturing` is true, waits for a key
/// combo (Escape cancels). Returns `Some(combo)` when a combo is captured.
fn hotkey_capture_button(ui: &mut egui::Ui, current_value: &str, capturing: &mut bool) -> Option<String> {
    if *capturing {
        let mods = ui.input(|i| i.modifiers);
        let mut parts = Vec::new();
        if mods.ctrl { parts.push("Ctrl"); }
        if mods.alt { parts.push("Alt"); }
        if mods.shift { parts.push("Shift"); }

        let label = if parts.is_empty() {
            "Press a key combo...".to_string()
        } else {
            format!("{}+...", parts.join("+"))
        };

        let btn = egui::Button::new(egui::RichText::new(&label).italics());
        let resp = ui.add(btn);

        let pressed = ui.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Key { key, pressed: true, modifiers, .. } = e {
                    if *key == egui::Key::Escape {
                        return Some(None);
                    }
                    egui_key_to_config_name(key).map(|name| {
                        let mut combo = Vec::new();
                        if modifiers.ctrl { combo.push("Ctrl"); }
                        if modifiers.alt { combo.push("Alt"); }
                        if modifiers.shift { combo.push("Shift"); }
                        combo.push(name);
                        Some(combo.join("+"))
                    })
                } else {
                    None
                }
            })
        });

        match pressed {
            Some(Some(combo)) => {
                *capturing = false;
                return Some(combo);
            }
            Some(None) => {
                *capturing = false;
            }
            None => {
                resp.request_focus();
            }
        }
        None
    } else {
        let label = if current_value.is_empty() { "None" } else { current_value };
        if ui.button(label).clicked() {
            *capturing = true;
        }
        ui.colored_label(ui.visuals().weak_text_color(), "Click to change");
        None
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

fn centered_position(width: f32, height: f32) -> egui::Pos2 {
    use windows::Win32::UI::HiDpi::GetDpiForSystem;
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    unsafe {
        let dpi = GetDpiForSystem() as f32;
        let scale = dpi / 96.0;
        let sw = GetSystemMetrics(SM_CXSCREEN) as f32 / scale;
        let sh = GetSystemMetrics(SM_CYSCREEN) as f32 / scale;
        egui::pos2((sw - width) / 2.0, (sh - height) / 2.0)
    }
}

fn notify_tray() {
    unsafe {
        if let Ok(tray) = FindWindowW(w!("StonemiteTrayClass"), w!("Stonemite")) {
            let _ = PostMessageW(tray, WM_SETTINGS_CHANGED, WPARAM(0), LPARAM(0));
        }
    }
}
