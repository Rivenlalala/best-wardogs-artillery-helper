//! 命令行解算器：给出两个游戏坐标，打印方位角、距离和 MIL。
//!
//! 这一层没有任何平台依赖，存在的意义有两个：
//! 在没有截屏和按键注入的情况下先核对解算结果是否和游戏里一致，
//! 以及验证交叉编译产物确实能在 Windows 上跑起来。
//!
//! 用法：autoartillery-core [武器] <原点X> <原点Y> <目标X> <目标Y>

use autoartillery_core::{Arc, Catalog, METERS_PER_UNIT, Mil, Point, solve};

const CATALOG: &str = include_str!("../data/weapons.json");

fn format_mil(mil: Mil) -> String {
    match mil {
        Mil::Exact(value) => format!("{value:.0}"),
        Mil::Range(min, max) => format!("{min:.0}–{max:.0}（区间，不确定 ±{:.0}）", mil.uncertainty()),
    }
}

fn report(arc: Arc, mil: Option<Mil>) {
    let label = match arc {
        Arc::Single => "单弧",
        Arc::Low => "低角",
        Arc::High => "高角",
    };
    match mil {
        Some(mil) => println!("  {label}: MIL {}  (闭环目标值 {:.1})", format_mil(mil), mil.target()),
        None => println!("  {label}: 无解"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let catalog = Catalog::from_json(CATALOG).expect("内置 weapons.json 必须能解析");

    let (weapon_id, coords) = if args.len() >= 5 {
        (args[0].clone(), &args[1..5])
    } else {
        if !args.is_empty() {
            eprintln!("参数不足，使用演示坐标。用法：autoartillery-core [武器] <原点X> <原点Y> <目标X> <目标Y>");
        }
        (catalog.default.clone(), &["0".into(), "0".into(), "3".into(), "4".into()][..])
    };

    let parse = |text: &String| text.parse::<f64>().expect("坐标必须是数字");
    let from = Point::new(parse(&coords[0]), parse(&coords[1]));
    let to = Point::new(parse(&coords[2]), parse(&coords[3]));

    let weapon = catalog
        .get(&weapon_id)
        .unwrap_or_else(|| panic!("未知武器 {weapon_id}，可用：{:?}", catalog.weapons.iter().map(|w| &w.id).collect::<Vec<_>>()));

    let solution = solve(weapon, from, to, METERS_PER_UNIT);

    println!("武器 {}", weapon.id);
    println!("原点 {from:?}  目标 {to:?}");
    println!("距离 {:.1} m", solution.distance_m);
    println!("方位角 {:.1}°", solution.azimuth_deg);
    println!("在射程内 {}", solution.in_range);
    report(Arc::Single, solution.mil(Arc::Single));
    report(Arc::Low, solution.mil(Arc::Low));
    report(Arc::High, solution.mil(Arc::High));
}
