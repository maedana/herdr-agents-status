mod herdr;

use eframe::egui::{self, Color32, RichText, Ui, Vec2};
use std::sync::{Arc, Mutex};

use herdr::{AgentStatus, HerdrState};

const REPAINT_INTERVAL_SECS: u64 = 2;
const MIN_WINDOW_WIDTH: f32 = 180.0;
const WINDOW_EMPTY_HEIGHT: f32 = 40.0;
const ROW_HEIGHT: f32 = 22.0;
const WINDOW_PADDING: f32 = 8.0;
const MARGIN: f32 = 2.0;
const ROW_HORIZONTAL_OVERHEAD: f32 = 82.0;
const ROBOT_BODY_COLOR: Color32 = Color32::from_rgb(210, 110, 30);
const HOVER_OPACITY: f32 = 0.0;
const HOVER_LERP_FACTOR: f32 = 0.25;
const OPACITY_SNAP_THRESHOLD: f32 = 0.01;

#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Position {
    TopLeft,
    #[default]
    TopCenter,
    TopRight,
    MiddleLeft,
    MiddleCenter,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Position {
    fn compute(self, monitor: Vec2, window: Vec2) -> egui::Pos2 {
        let x = match self {
            Self::TopLeft | Self::MiddleLeft | Self::BottomLeft => MARGIN,
            Self::TopCenter | Self::MiddleCenter | Self::BottomCenter => {
                (monitor.x - window.x) / 2.0
            }
            Self::TopRight | Self::MiddleRight | Self::BottomRight => {
                monitor.x - window.x - MARGIN
            }
        };
        let y = match self {
            Self::TopLeft | Self::TopCenter | Self::TopRight => MARGIN,
            Self::MiddleLeft | Self::MiddleCenter | Self::MiddleRight => {
                (monitor.y - window.y) / 2.0
            }
            Self::BottomLeft | Self::BottomCenter | Self::BottomRight => {
                monitor.y - window.y - MARGIN
            }
        };
        egui::pos2(x, y)
    }
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct Config {
    position: Position,
}

fn load_config() -> Config {
    let config_dir = std::process::Command::new(
        std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into()),
    )
    .args(["plugin", "config-dir", "maedana.agents-status"])
    .output()
    .ok()
    .filter(|o| o.status.success())
    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    let Some(dir) = config_dir else {
        return Config::default();
    };

    let path = std::path::Path::new(&dir).join("config.toml");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Config::default();
    };

    toml::from_str(&content).unwrap_or_default()
}

fn pid_file_path() -> std::path::PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/tmp/herdr-agents-status-{}", unsafe { libc::getuid() }));
    std::path::Path::new(&runtime_dir).join("herdr-agents-status.pid")
}

fn toggle_or_start() -> bool {
    let pid_path = pid_file_path();
    if let Ok(content) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = content.trim().parse::<i32>() {
            unsafe {
                if libc::kill(pid, 0) == 0 {
                    libc::kill(pid, libc::SIGTERM);
                    let _ = std::fs::remove_file(&pid_path);
                    return false;
                }
            }
        }
        let _ = std::fs::remove_file(&pid_path);
    }
    let _ = std::fs::write(&pid_path, std::process::id().to_string());
    true
}

fn cleanup_pid_file() {
    let _ = std::fs::remove_file(pid_file_path());
}

fn main() -> eframe::Result<()> {
    if !toggle_or_start() {
        return Ok(());
    }

    let config = load_config();
    let state: Arc<Mutex<HerdrState>> = Arc::new(Mutex::new(HerdrState::default()));
    herdr::start_polling(Arc::clone(&state));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_decorations(false)
            .with_always_on_top()
            .with_mouse_passthrough(true)
            .with_inner_size([MIN_WINDOW_WIDTH, WINDOW_EMPTY_HEIGHT])
            .with_transparent(true)
            .with_active(false)
            .with_window_type(egui::X11WindowType::Utility),
        ..Default::default()
    };

    restore_focus_on_x11();

    let result = eframe::run_native(
        "herdr-agents-status",
        options,
        Box::new(|cc| {
            if let Some(cjk_font) = load_cjk_font() {
                let mut fonts = egui::FontDefinitions::default();
                fonts
                    .font_data
                    .insert("cjk_font".to_owned(), cjk_font.into());
                fonts
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default()
                    .push("cjk_font".to_owned());
                cc.egui_ctx.set_fonts(fonts);
            }

            let mut visuals = cc.egui_ctx.style().visuals.clone();
            visuals.panel_fill = Color32::TRANSPARENT;
            cc.egui_ctx.set_visuals(visuals);
            Ok(Box::new(App {
                state,
                position: config.position,
                hover_opacity: 1.0,
            }))
        }),
    );

    cleanup_pid_file();
    result
}

fn restore_focus_on_x11() {
    if std::env::var("XDG_SESSION_TYPE")
        .map(|v| v == "x11")
        .unwrap_or(false)
    {
        if let Some(window_id) = std::process::Command::new("xdotool")
            .args(["getactivewindow"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|id| !id.is_empty())
        {
            std::thread::spawn(move || {
                let _ = std::process::Command::new("xdotool")
                    .args(["search", "--sync", "--name", "herdr-agents-status"])
                    .output();
                let _ = std::process::Command::new("xdotool")
                    .args(["windowactivate", &window_id])
                    .status();
            });
        }
    }
}

fn load_cjk_font() -> Option<egui::FontData> {
    use font_kit::family_name::FamilyName;
    use font_kit::properties::Properties;
    use font_kit::source::SystemSource;

    let families = [
        "Noto Sans CJK JP",
        "Noto Sans JP",
        "Hiragino Sans",
        "Hiragino Kaku Gothic ProN",
        "Yu Gothic",
        "MS Gothic",
    ];

    let source = SystemSource::new();
    for name in &families {
        if let Ok(handle) =
            source.select_best_match(&[FamilyName::Title(name.to_string())], &Properties::new())
        {
            if let Ok(font) = handle.load() {
                if let Some(data) = font.copy_font_data() {
                    return Some(egui::FontData::from_owned((*data).clone()));
                }
            }
        }
    }
    None
}

struct App {
    state: Arc<Mutex<HerdrState>>,
    position: Position,
    hover_opacity: f32,
}

fn status_color(status: &AgentStatus) -> Color32 {
    match status {
        AgentStatus::Working => Color32::from_rgb(80, 200, 80),
        AgentStatus::Blocked => Color32::from_rgb(220, 180, 0),
        AgentStatus::Idle | AgentStatus::Unknown => Color32::from_gray(160),
        AgentStatus::Done => Color32::from_rgb(100, 180, 220),
    }
}

fn apply_opacity(color: Color32, opacity: f32) -> Color32 {
    let [r, g, b, a] = color.to_array();
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let new_a = (f32::from(a) * opacity) as u8;
    Color32::from_rgba_unmultiplied(r, g, b, new_a)
}

fn bubble_fill_color() -> Color32 {
    Color32::from_rgba_unmultiplied(30, 30, 45, 220)
}

fn format_label(agent: &herdr::AgentInfo) -> String {
    let mut parts = Vec::new();

    match &agent.git_branch {
        Some(branch) => parts.push(format!("{} ({})", agent.project_name, branch)),
        None => parts.push(agent.project_name.clone()),
    }

    if let Some(title) = &agent.terminal_title_stripped {
        let truncated = if title.chars().count() > 20 {
            let s: String = title.chars().take(20).collect();
            format!("{s}…")
        } else {
            title.clone()
        };
        if !truncated.is_empty() {
            parts.push(truncated);
        }
    }

    parts.push(format!("[{}]", agent.status.label()));
    parts.join("  ")
}

fn measure_text_width(ctx: &egui::Context, text: &str) -> f32 {
    let font_id = egui::FontId::proportional(11.0);
    ctx.fonts(|fonts| {
        let galley = fonts.layout_no_wrap(text.to_string(), font_id, Color32::WHITE);
        galley.size().x
    })
}

#[cfg(target_os = "linux")]
fn get_cursor_screen_position() -> Option<(f32, f32)> {
    unsafe {
        let xlib = x11_dl::xlib::Xlib::open().ok()?;
        let display = (xlib.XOpenDisplay)(std::ptr::null());
        if display.is_null() {
            return None;
        }
        let screen = (xlib.XDefaultScreen)(display);
        let root = (xlib.XRootWindow)(display, screen);
        let mut root_return = 0;
        let mut child_return = 0;
        let mut root_x = 0;
        let mut root_y = 0;
        let mut win_x = 0;
        let mut win_y = 0;
        let mut mask = 0;
        let ok = (xlib.XQueryPointer)(
            display,
            root,
            &mut root_return,
            &mut child_return,
            &mut root_x,
            &mut root_y,
            &mut win_x,
            &mut win_y,
            &mut mask,
        );
        (xlib.XCloseDisplay)(display);
        if ok != 0 {
            Some((root_x as f32, root_y as f32))
        } else {
            None
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn get_cursor_screen_position() -> Option<(f32, f32)> {
    None
}

fn is_cursor_in_rect(cx: f32, cy: f32, x: f32, y: f32, w: f32, h: f32) -> bool {
    cx >= x && cx <= x + w && cy >= y && cy <= y + h
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
            egui::WindowLevel::AlwaysOnTop,
        ));
        ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(true));

        let agents = match self.state.lock() {
            Ok(guard) => guard.agents.clone(),
            Err(_) => return,
        };

        let has_pulse = agents.iter().any(|a| should_pulse(&a.status));

        if has_pulse || agents.iter().any(|a| a.status == AgentStatus::Working) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else if !agents.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs(REPAINT_INTERVAL_SECS));
        }

        let n = agents.len() as f32;
        let window_height = if agents.is_empty() {
            WINDOW_EMPTY_HEIGHT
        } else {
            n * ROW_HEIGHT + (n - 1.0) * 4.0 + WINDOW_PADDING * 2.0
        };

        let window_width = if agents.is_empty() {
            MIN_WINDOW_WIDTH
        } else {
            let max_text = agents
                .iter()
                .map(|a| measure_text_width(ctx, &format_label(a)))
                .fold(0.0_f32, f32::max);
            (max_text + ROW_HORIZONTAL_OVERHEAD).max(MIN_WINDOW_WIDTH)
        };

        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
            window_width,
            window_height,
        )));

        let mut is_hovering = false;
        if let Some(monitor_size) = ctx.input(|i| i.viewport().monitor_size) {
            let pos = self
                .position
                .compute(monitor_size, Vec2::new(window_width, window_height));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));

            is_hovering = get_cursor_screen_position()
                .map(|(cx, cy)| is_cursor_in_rect(cx, cy, pos.x, pos.y, window_width, window_height))
                .unwrap_or(false);
        }

        let target_opacity = if is_hovering { HOVER_OPACITY } else { 1.0 };
        let diff = target_opacity - self.hover_opacity;
        self.hover_opacity += diff * HOVER_LERP_FACTOR;
        if (self.hover_opacity - target_opacity).abs() < OPACITY_SNAP_THRESHOLD {
            self.hover_opacity = target_opacity;
        }

        if (self.hover_opacity - target_opacity).abs() > OPACITY_SNAP_THRESHOLD || is_hovering {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }

        let hover_opacity = self.hover_opacity;
        let time = ctx.input(|i| i.time);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::symmetric(8.0, WINDOW_PADDING)),
            )
            .show(ctx, |ui| {
                if agents.is_empty() {
                    ui.label(
                        RichText::new("No agents")
                            .color(apply_opacity(Color32::from_gray(120), hover_opacity))
                            .size(12.0),
                    );
                } else {
                    for agent in &agents {
                        render_agent_row(ui, agent, time, hover_opacity);
                    }
                }
            });
    }
}

fn should_pulse(status: &AgentStatus) -> bool {
    matches!(status, AgentStatus::Blocked | AgentStatus::Done)
}

fn calc_stroke_width(time: f64, pulse: bool) -> f32 {
    if pulse {
        let p = ((time * 16.0).sin() as f32 + 1.0) / 2.0;
        1.0 + p * 2.0
    } else {
        1.0
    }
}

fn render_agent_row(ui: &mut Ui, agent: &herdr::AgentInfo, time: f64, hover_opacity: f32) {
    let color = apply_opacity(status_color(&agent.status), hover_opacity);
    let body_color = apply_opacity(ROBOT_BODY_COLOR, hover_opacity);
    let fill = apply_opacity(bubble_fill_color(), hover_opacity);
    let label = format_label(agent);
    let pulse = should_pulse(&agent.status);
    let stroke_width = calc_stroke_width(time, pulse);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        render_robot_art(ui, color, body_color);
        ui.add_space(2.0);

        let max_label_width = (ui.available_width() - 14.0).max(0.0);
        render_speech_bubble(ui, stroke_width, color, fill, Some(max_label_width), |ui| {
            ui.label(RichText::new(label).color(color).size(11.0));
        });
    });
}

fn render_robot_art(ui: &mut Ui, state_color: Color32, body_color: Color32) {
    ui.allocate_ui(Vec2::new(40.0, ROW_HEIGHT), |ui| {
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let lines: [(&str, Color32); 4] = [
                ("▟█▙", state_color),
                ("▐▛███▜▌", body_color),
                ("▝▜█████▛▘", body_color),
                ("▘▘ ▝▝", body_color),
            ];
            for (text, color) in lines {
                ui.label(RichText::new(text).size(5.0).color(color).monospace());
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_pulse_blocked() {
        assert!(should_pulse(&AgentStatus::Blocked));
    }

    #[test]
    fn should_pulse_done() {
        assert!(should_pulse(&AgentStatus::Done));
    }

    #[test]
    fn should_not_pulse_working() {
        assert!(!should_pulse(&AgentStatus::Working));
    }

    #[test]
    fn should_not_pulse_idle() {
        assert!(!should_pulse(&AgentStatus::Idle));
    }

    #[test]
    fn should_not_pulse_unknown() {
        assert!(!should_pulse(&AgentStatus::Unknown));
    }

    #[test]
    fn calc_stroke_width_no_pulse_is_always_one() {
        assert_eq!(calc_stroke_width(0.0, false), 1.0);
        assert_eq!(calc_stroke_width(5.0, false), 1.0);
    }

    #[test]
    fn calc_stroke_width_pulse_oscillates() {
        let mut saw_peak = false;
        for t in 0..100 {
            let time = t as f64 * 0.1;
            let w = calc_stroke_width(time, true);
            assert!(w >= 1.0 && w <= 3.0, "got {w} at time {time}");
            if w > 2.5 {
                saw_peak = true;
            }
        }
        assert!(saw_peak, "should reach near 3.0");
    }
}

fn render_speech_bubble(
    ui: &mut Ui,
    stroke_width: f32,
    state_color: Color32,
    bubble_fill: Color32,
    max_label_width: Option<f32>,
    content: impl FnOnce(&mut Ui),
) {
    let inner = egui::Frame::none()
        .fill(bubble_fill)
        .stroke(egui::Stroke::new(stroke_width, state_color))
        .rounding(egui::Rounding::same(5.0))
        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
        .show(ui, |ui: &mut Ui| {
            if let Some(w) = max_label_width {
                ui.set_max_width(w);
            }
            content(ui);
        });

    let rect = inner.response.rect;
    let mid_y = rect.center().y;
    let tail_tip = egui::pos2(rect.left() - 4.0, mid_y);
    let tail_top = egui::pos2(rect.left(), mid_y - 4.0);
    let tail_bot = egui::pos2(rect.left(), mid_y + 4.0);
    let painter = ui.painter();
    painter.add(egui::Shape::convex_polygon(
        vec![tail_tip, tail_top, tail_bot],
        bubble_fill,
        egui::Stroke::NONE,
    ));
    painter.line_segment(
        [tail_tip, tail_top],
        egui::Stroke::new(stroke_width, state_color),
    );
    painter.line_segment(
        [tail_tip, tail_bot],
        egui::Stroke::new(stroke_width, state_color),
    );
}
