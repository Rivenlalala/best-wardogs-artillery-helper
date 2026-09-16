//! 角落控制面板：从剪贴板收坐标，解算射击诸元，驱动屏幕覆盖层。
//!
//! 坐标从游戏聊天框复制，格式形如 `x99.01, y110.58`。
//! 第一次复制填「我的位置」，之后每次复制都是新目标 —— 炮位很少挪，目标一直换。
//! 换炮位按 RESET。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::Duration;

use eframe::egui;
use egui::{Align, Color32, ComboBox, FontId, Layout, RichText, Sense, ViewportCommand};

use autoartillery_core::{
    Arc, Catalog, METERS_PER_UNIT, Mil, Point, SHIPPED_WEAPONS_JSON, Solution, Weapon,
    coords::parse_point, solve,
};

mod overlay;

const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// 剪贴板内容在状态行里最多显示多少字符。
const CLIP_PREVIEW_CHARS: usize = 28;

/// UI 里的一个整体选择：武器 + 弹道弧。玩家心里只有「我在打什么」这一个念头，
/// 不该逼他先选炮再选弧；core 那边仍然是正交的 Weapon 与 Arc。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Preset {
    Mortar,
    SpgLow,
    SpgHigh,
}

impl Preset {
    const ALL: [Preset; 3] = [Preset::Mortar, Preset::SpgLow, Preset::SpgHigh];

    fn weapon_id(self) -> &'static str {
        match self {
            Preset::Mortar => "mortar",
            Preset::SpgLow | Preset::SpgHigh => "spg",
        }
    }

    fn arc(self) -> Arc {
        match self {
            Preset::Mortar => Arc::Single,
            Preset::SpgLow => Arc::Low,
            Preset::SpgHigh => Arc::High,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Preset::Mortar => "MORTAR",
            Preset::SpgLow => "SPG · LOW",
            Preset::SpgHigh => "SPG · HIGH",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Origin,
    Target,
}

/// 收到的坐标落在哪个槽位：原点为空时填原点，否则每次都是新目标。
#[derive(Debug, Default, PartialEq)]
struct Session {
    origin: Option<Point>,
    target: Option<Point>,
}

impl Session {
    fn submit(&mut self, point: Point) -> Slot {
        if self.origin.is_none() {
            self.origin = Some(point);
            return Slot::Origin;
        }
        self.target = Some(point);
        Slot::Target
    }

    fn reset(&mut self) {
        *self = Session::default();
    }

    fn pair(&self) -> Option<(Point, Point)> {
        Some((self.origin?, self.target?))
    }
}

fn preview(text: &str) -> String {
    let flattened: String = text.chars().map(|ch| if ch.is_control() { ' ' } else { ch }).collect();
    let trimmed = flattened.trim();
    if trimmed.chars().count() <= CLIP_PREVIEW_CHARS {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(CLIP_PREVIEW_CHARS).collect();
    format!("{head}…")
}

fn format_mil(mil: Mil) -> String {
    match mil {
        Mil::Exact(value) => format!("{value:.0}"),
        // 低角与高角在最大射程收敛，同一距离有多个值，这时候给区间而不是假装知道一个数
        Mil::Range(min, max) => format!("{min:.0}–{max:.0}"),
    }
}

fn format_point(point: Point) -> String {
    format!("{:.2}, {:.2}", point.x, point.y)
}

struct Panel {
    catalog: Catalog,
    clipboard: Option<arboard::Clipboard>,
    seen: Option<String>,
    session: Session,
    preset: Preset,
    status: String,
    /// CLEAR OVL 按下后为真：诸元照算照显示，只是不往屏幕上画，直到下一个目标到来。
    overlay_muted: bool,
    /// 已经画上去的（密位, 方位角）。相同就不重画 —— update 每 200ms 跑一次。
    drawn: Option<(f64, f64)>,
    overlay: overlay::Overlay,
}

impl Panel {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let preset = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, "preset"))
            .unwrap_or(Preset::Mortar);

        let (clipboard, mut status) = match arboard::Clipboard::new() {
            Ok(clipboard) => (Some(clipboard), "copy your position".to_string()),
            Err(error) => (None, format!("no clipboard: {error}")),
        };
        // 建不起来（非 Windows、分辨率不符、Win32 失败）就只剩面板，不影响读数。
        let overlay = overlay::Overlay::create();
        if let Some(warning) = overlay.warning() {
            status = warning.to_string();
        }

        Self {
            catalog: Catalog::from_json(SHIPPED_WEAPONS_JSON).expect("内置火表必须能解析"),
            clipboard,
            seen: None,
            session: Session::default(),
            preset,
            status,
            overlay_muted: false,
            drawn: None,
            overlay,
        }
    }

    fn weapon(&self) -> &Weapon {
        self.catalog.get(self.preset.weapon_id()).expect("preset 必须对应内置火表里的武器")
    }

    fn poll_clipboard(&mut self) {
        let Some(clipboard) = self.clipboard.as_mut() else {
            return;
        };
        let Ok(text) = clipboard.get_text() else {
            return;
        };
        if self.seen.as_deref() == Some(text.as_str()) {
            return;
        }
        self.seen = Some(text.clone());

        let Some(point) = parse_point(&text) else {
            self.status = format!("not a coord: {}", preview(&text));
            return;
        };
        match self.session.submit(point) {
            Slot::Origin => self.status = "origin set · copy a target".to_string(),
            Slot::Target => {
                self.status = "target set".to_string();
                // 新目标的刻度必须画出来，否则上一次 CLEAR OVL 会静默吞掉这一发。
                self.overlay_muted = false;
            }
        }
    }

    fn solution(&self) -> Option<Solution> {
        let (origin, target) = self.session.pair()?;
        Some(solve(self.weapon(), origin, target, METERS_PER_UNIT))
    }

    /// 覆盖层永远跟着当前解走：没解、超射程、被静音，都必须是空的 ——
    /// 屏幕上留着上一个目标的刻度是这个工具最危险的失效方式。
    fn sync_overlay(&mut self, solution: Option<&Solution>) {
        let ghost = solution
            .filter(|solution| solution.in_range && !self.overlay_muted)
            .and_then(|solution| {
                Some((solution.mil(self.preset.arc())?.target(), solution.azimuth_deg))
            });
        if ghost == self.drawn {
            return;
        }
        self.drawn = ghost;
        match ghost {
            Some((target_mil, azimuth_deg)) => self.overlay.show_ticks(target_mil, azimuth_deg),
            None => self.overlay.clear(),
        }
    }
}

const DIM: Color32 = Color32::from_rgb(120, 132, 140);
const LIVE: Color32 = Color32::from_rgb(210, 220, 226);
const ALERT: Color32 = Color32::from_rgb(232, 96, 86);

fn slot_row(ui: &mut egui::Ui, label: &str, point: Option<Point>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(DIM).monospace());
        match point {
            Some(point) => ui.label(RichText::new(format_point(point)).color(LIVE).monospace()),
            None => ui.label(RichText::new("—").color(DIM).monospace()),
        };
    });
}

impl eframe::App for Panel {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "preset", &self.preset);
    }

    /// 收数与画覆盖层都放在这里：窗口被遮住或最小化时 eframe 不跑 egui pass，
    /// 只有 logic 照跑。画刻度要是留在 ui 里，面板一被盖住就停在上一个目标上。
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 游戏有焦点时窗口收不到输入事件，必须自己定时醒来读剪贴板。
        ctx.request_repaint_after(POLL_INTERVAL);
        self.poll_clipboard();
        let solution = self.solution();
        self.sync_overlay(solution.as_ref());
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let frame = egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(14, 17, 20, 226))
            .inner_margin(8)
            .corner_radius(6);

        egui::CentralPanel::default().frame(frame).show(root, |ui| {
            // 先占整块背景做拖动区，后面画的控件层级更高，点击照样归它们。
            let background = ui.interact(ui.max_rect(), ui.id().with("drag"), Sense::drag());
            if background.dragged() {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }

            ui.horizontal(|ui| {
                ComboBox::from_id_salt("preset")
                    .selected_text(RichText::new(self.preset.label()).color(LIVE).monospace())
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for preset in Preset::ALL {
                            ui.selectable_value(&mut self.preset, preset, preset.label());
                        }
                    });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("×").clicked() {
                        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                    }
                });
            });

            slot_row(ui, "ME ", self.session.origin);
            slot_row(ui, "TGT", self.session.target);
            ui.separator();

            let solution = self.solution();
            match solution.as_ref() {
                None => {
                    ui.label(RichText::new("—").color(DIM).font(FontId::monospace(20.0)));
                }
                Some(solution) if !solution.in_range => {
                    let weapon = self.weapon();
                    ui.label(RichText::new("OUT OF RANGE").color(ALERT).monospace());
                    ui.label(
                        RichText::new(format!(
                            "{:.0}–{:.0} m · have {:.0} m",
                            weapon.min_range_km * 1000.0,
                            weapon.max_range_km * 1000.0,
                            solution.distance_m
                        ))
                        .color(DIM)
                        .monospace(),
                    );
                }
                Some(solution) => {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{:.0} m", solution.distance_m))
                                .color(LIVE)
                                .monospace(),
                        );
                        ui.label(
                            RichText::new(format!("{:.1}°", solution.azimuth_deg))
                                .color(LIVE)
                                .monospace(),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let mil = solution
                                .mil(self.preset.arc())
                                .map(format_mil)
                                .unwrap_or_else(|| "n/a".to_string());
                            ui.label(
                                RichText::new(format!("{mil} MIL"))
                                    .color(LIVE)
                                    .font(FontId::monospace(20.0)),
                            );
                        });
                    });
                }
            }

            ui.horizontal(|ui| {
                if ui.button("RESET").clicked() {
                    self.session.reset();
                    self.overlay_muted = false;
                    // 剪贴板里现在还是旧目标（玩家还没重新复制）——把它标记为已见，
                    // 否则下一帧轮询会把这个旧值当成新复制填进原点。真正重复复制
                    // 同一个坐标的情况仍会被正常接受 —— 那时剪贴板内容已经换回来了。
                    if let Some(clipboard) = self.clipboard.as_mut() {
                        self.seen = clipboard.get_text().ok();
                    }
                    self.status = "copy your position".to_string();
                }
                if ui.button("CLEAR OVL").clicked() {
                    self.overlay_muted = true;
                }
            });
            ui.label(RichText::new(&self.status).color(DIM).monospace().size(10.0));
        });
    }
}

fn main() -> eframe::Result {
    // 必须早于 winit 建窗口：DPI 感知只有进程里第一次设置算数，
    // 晚了就轮到 winit 定，刻度常数按的物理像素对不上。
    overlay::Overlay::make_dpi_aware();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([268.0, 156.0])
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native(
        "auto-artillery",
        options,
        Box::new(|cc| Ok(Box::new(Panel::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn first_copy_is_origin_then_every_copy_is_a_new_target() {
        let mut session = Session::default();
        assert_eq!(session.submit(point(1.0, 2.0)), Slot::Origin);
        assert_eq!(session.submit(point(3.0, 4.0)), Slot::Target);
        assert_eq!(session.pair(), Some((point(1.0, 2.0), point(3.0, 4.0))));

        // 炮位不动，只换目标：原点必须粘住。
        assert_eq!(session.submit(point(5.0, 6.0)), Slot::Target);
        assert_eq!(session.pair(), Some((point(1.0, 2.0), point(5.0, 6.0))));
    }

    #[test]
    fn reset_clears_both_slots_and_the_next_copy_is_the_origin() {
        let mut session = Session::default();
        session.submit(point(1.0, 2.0));
        session.submit(point(3.0, 4.0));
        session.reset();
        assert_eq!(session.pair(), None);
        assert_eq!(session.submit(point(9.0, 9.0)), Slot::Origin, "重置后第一次复制是新炮位");
        assert_eq!(session.pair(), None, "新炮位不能配上一轮的过期目标");
    }

    #[test]
    fn presets_resolve_to_a_shipped_weapon_and_an_arc_that_has_a_table() {
        let catalog = Catalog::from_json(SHIPPED_WEAPONS_JSON).unwrap();
        for preset in Preset::ALL {
            let weapon = catalog.get(preset.weapon_id()).expect("preset 指向的武器必须存在");
            let mid_km = (weapon.min_range_km + weapon.max_range_km) / 2.0;
            let solution = solve(
                weapon,
                point(0.0, 0.0),
                point(mid_km * 1000.0 / METERS_PER_UNIT, 0.0),
                METERS_PER_UNIT,
            );
            assert!(
                solution.mil(preset.arc()).is_some(),
                "{:?} 在射程中点必须有密位",
                preset
            );
        }
    }

    /// 原点与目标相同是无效输入，不能悄悄算出一个零距离的解。
    #[test]
    fn identical_points_are_flagged_as_out_of_range() {
        let catalog = Catalog::from_json(SHIPPED_WEAPONS_JSON).unwrap();
        let mortar = catalog.get("mortar").unwrap();
        let solution = solve(mortar, point(50.0, 50.0), point(50.0, 50.0), METERS_PER_UNIT);
        assert_eq!(solution.distance_m, 0.0);
        assert!(!solution.in_range, "零距离对迫击炮应在射程外（最小 132m）");
        assert_eq!(solution.single, None);
    }

    #[test]
    fn preview_truncates_and_flattens() {
        assert_eq!(preview("x1.00, y2.00"), "x1.00, y2.00");
        assert_eq!(preview("a\nb\r\nc"), "a b  c");
        let long = "9".repeat(100);
        assert_eq!(preview(&long).chars().count(), CLIP_PREVIEW_CHARS + 1);
    }
}
