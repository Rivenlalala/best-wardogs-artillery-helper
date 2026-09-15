//! 从剪贴板读坐标，解算射击诸元。
//!
//! 坐标从游戏聊天框复制，格式形如 `x99.01, y110.58`。
//! 依次复制两次：第一次填原点，第二次填目标。两次都齐了就打印诸元。
//!
//! 目前只有命令行输出。屏幕覆盖层是下一步（见 epic 的 autoart-3qf.4）。

use std::thread::sleep;
use std::time::Duration;

use autoartillery_core::{
    Arc, Catalog, METERS_PER_UNIT, Mil, Point, SHIPPED_WEAPONS_JSON, Weapon, coords::parse_point,
    sight, solve,
};

mod overlay;

const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// 剪贴板内容在提示里最多显示多少字符。
const CLIP_PREVIEW_CHARS: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Origin,
    Target,
}

impl Default for Slot {
    fn default() -> Self {
        Slot::Origin
    }
}

/// 依次接收复制到的坐标：第一次填原点，第二次填目标，之后开始新的一对。
#[derive(Debug, Default)]
struct Pair {
    origin: Option<Point>,
    target: Option<Point>,
    next: Slot,
    last: Option<Point>,
}

impl Pair {
    /// 返回这个点填进了哪个槽位。重复复制同一个点会被忽略。
    fn submit(&mut self, point: Point) -> Option<Slot> {
        // 同一点重复复制时忽略，否则手滑复制两次会白占掉目标槽位。
        if self.last == Some(point) {
            return None;
        }
        self.last = Some(point);

        let slot = self.next;
        match slot {
            Slot::Origin => {
                // 开始新的一对。旧的目标必须一并清掉，否则会用新原点
                // 配上一个过期的目标，立刻算出一个看起来正常但是错的解。
                self.origin = Some(point);
                self.target = None;
                self.next = Slot::Target;
            }
            Slot::Target => {
                self.target = Some(point);
                self.next = Slot::Origin;
            }
        }
        Some(slot)
    }

    fn complete(&self) -> Option<(Point, Point)> {
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

/// 打印诸元和幽灵刻度的位置，返回幽灵刻度对应的目标密位与方位角（密位无则 None）。
fn report(weapon: &Weapon, origin: Point, target: Point) -> Option<(f64, f64)> {
    let solution = solve(weapon, origin, target, METERS_PER_UNIT);
    let mut ghost = None;

    println!();
    println!("  距离   {:.1} m", solution.distance_m);
    println!("  方位角 {:.1}°", solution.azimuth_deg);

    if !solution.in_range {
        println!("  超出射程（{:.0}–{:.0} m）", weapon.min_range_km * 1000.0, weapon.max_range_km * 1000.0);
        return None;
    }

    for (label, arc) in [("单弧", Arc::Single), ("低角", Arc::Low), ("高角", Arc::High)] {
        let Some(mil) = solution.mil(arc) else {
            continue;
        };
        println!("  {label}密位 {}", format_mil(mil));

        if arc == Arc::Low || arc == Arc::Single {
            let target_mil = mil.target();
            ghost = Some((target_mil, solution.azimuth_deg));
            let rendered: Vec<String> = sight::tick_rows(target_mil, 4)
                .iter()
                .map(|(tick, y)| format!("{tick:.0}@{y}"))
                .collect();
            println!("      幽灵刻度 y = {}", rendered.join("  "));
        }
    }
    ghost
}

fn main() {
    let catalog = Catalog::from_json(SHIPPED_WEAPONS_JSON).expect("内置火表必须能解析");

    let requested = std::env::args().nth(1);
    let weapon_id = requested.clone().unwrap_or_else(|| catalog.default.clone());
    let Some(weapon) = catalog.get(&weapon_id) else {
        eprintln!(
            "未知武器 {weapon_id:?}，可用：{}",
            catalog
                .weapons
                .iter()
                .map(|weapon| weapon.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        std::process::exit(2);
    };

    let mut clipboard = match arboard::Clipboard::new() {
        Ok(clipboard) => clipboard,
        Err(error) => {
            eprintln!("打不开剪贴板：{error}");
            std::process::exit(1);
        }
    };

    println!("武器 {}。在游戏里标记坐标并复制：", weapon.id);
    println!("  第 1 次复制 -> 我的位置（原点）");
    println!("  第 2 次复制 -> 目标位置");
    println!("（同一点重复复制会被忽略）");

    let mut pair = Pair::default();
    let mut seen: Option<String> = None;
    // 建不起来（非 Windows、分辨率不符、Win32 失败）就留在纯命令行模式。
    let mut overlay = overlay::Overlay::create();

    loop {
        if let Ok(text) = clipboard.get_text()
            && seen.as_deref() != Some(text.as_str())
        {
            seen = Some(text.clone());
            match parse_point(&text) {
                Some(point) => {
                    if let Some(slot) = pair.submit(point) {
                        let label = match slot {
                            Slot::Origin => "原点",
                            Slot::Target => "目标",
                        };
                        println!("\n{label} = X {:.2}  Y {:.2}", point.x, point.y);
                        // 新一对开始时旧刻度已失效，不能留着误导下一发。
                        if slot == Slot::Origin {
                            overlay.clear();
                        }
                    }
                    if let Some((origin, target)) = pair.complete() {
                        match report(weapon, origin, target) {
                            Some((target_mil, target_azimuth_deg)) => {
                                overlay.show_ticks(target_mil, target_azimuth_deg)
                            }
                            None => overlay.clear(),
                        }
                        println!("\n继续复制以开始新的一对。");
                    }
                }
                None => println!("剪贴板内容不是坐标，已忽略：{:?}", preview(&text)),
            }
        }
        overlay.pump();
        sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn fills_origin_then_target() {
        let mut pair = Pair::default();
        assert_eq!(pair.submit(point(1.0, 2.0)), Some(Slot::Origin));
        assert_eq!(pair.submit(point(3.0, 4.0)), Some(Slot::Target));
        assert_eq!(pair.complete(), Some((point(1.0, 2.0), point(3.0, 4.0))));
    }

    /// 手滑复制两次同一个点，不该把目标槽位占掉。
    #[test]
    fn repeated_copy_of_same_point_is_ignored() {
        let mut pair = Pair::default();
        assert_eq!(pair.submit(point(1.0, 2.0)), Some(Slot::Origin));
        assert_eq!(pair.submit(point(1.0, 2.0)), None, "重复点必须忽略");
        assert_eq!(pair.complete(), None, "目标还没填");
        assert_eq!(pair.submit(point(3.0, 4.0)), Some(Slot::Target));
        assert_eq!(pair.complete(), Some((point(1.0, 2.0), point(3.0, 4.0))));
    }

    #[test]
    fn starts_a_new_pair_after_completion() {
        let mut pair = Pair::default();
        pair.submit(point(1.0, 1.0));
        pair.submit(point(2.0, 2.0));
        assert_eq!(pair.complete(), Some((point(1.0, 1.0), point(2.0, 2.0))));

        // 第三次复制开始新的一对，旧目标必须被清掉
        assert_eq!(pair.submit(point(9.0, 9.0)), Some(Slot::Origin));
        assert_eq!(
            pair.complete(),
            None,
            "新原点不能配上上一轮的过期目标"
        );
        assert_eq!(pair.submit(point(8.0, 8.0)), Some(Slot::Target));
        assert_eq!(pair.complete(), Some((point(9.0, 9.0), point(8.0, 8.0))));
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
