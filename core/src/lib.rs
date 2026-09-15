//! 射击诸元解算。纯逻辑，无平台依赖，可在任意平台上编译和测试。
//!
//! 输入是两个游戏坐标，输出是方位角、距离和火表仰角(MIL)。
//! 坐标比例尺、武器与弹道弧按游戏内的实际选择作为入参传入。

use serde::Deserialize;

/// 游戏坐标单位到米的换算。真实值来自每张地图的 `coordinateMetersPerUnit`，
/// 三张官方地图都是 100；wardogs-calculator 对未知地图回落到 1000，
/// 那个默认值是个陷阱，这里不做静默回落。
pub const METERS_PER_UNIT: f64 = 100.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// 火表在某个距离上给出的 MIL。
///
/// 低角和高角在最大射程附近收敛，同一个距离上可能有多个值。
/// 那种情况下只有一个区间是诚实的，不能假装知道确定值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mil {
    Exact(f64),
    Range(f64, f64),
}

impl Mil {
    /// 区间中点。用于闭环控制里作为目标值。
    pub fn target(self) -> f64 {
        match self {
            Mil::Exact(mil) => mil,
            Mil::Range(min, max) => (min + max) / 2.0,
        }
    }

    /// 区间半宽。0 表示火表给的是确定值。
    pub fn uncertainty(self) -> f64 {
        match self {
            Mil::Exact(_) => 0.0,
            Mil::Range(min, max) => (max - min) / 2.0,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Ballistics {
    #[serde(default)]
    pub single: Vec<[f64; 2]>,
    #[serde(default)]
    pub low: Vec<[f64; 2]>,
    #[serde(default)]
    pub high: Vec<[f64; 2]>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Weapon {
    pub id: String,
    pub min_range_km: f64,
    pub max_range_km: f64,
    pub ballistics: Ballistics,
}

#[derive(Debug, Deserialize)]
pub struct Catalog {
    pub default: String,
    pub weapons: Vec<Weapon>,
}

impl Catalog {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn get(&self, id: &str) -> Option<&Weapon> {
        self.weapons.iter().find(|weapon| weapon.id == id)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Solution {
    /// 0° = +Y 轴，向 +X 递增。
    pub azimuth_deg: f64,
    pub distance_m: f64,
    /// 只有 mortar 用单弧，只有 spg 用低/高双弧，两个字段互斥。
    pub single: Option<Mil>,
    pub low: Option<Mil>,
    pub high: Option<Mil>,
    pub in_range: bool,
}

impl Solution {
    /// 双弧武器必须显式选弧。单弧武器返回单弧解。
    pub fn mil(&self, arc: Arc) -> Option<Mil> {
        match arc {
            Arc::Single => self.single,
            Arc::Low => self.low,
            Arc::High => self.high,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arc {
    Single,
    Low,
    High,
}

pub fn solve(weapon: &Weapon, from: Point, to: Point, meters_per_unit: f64) -> Solution {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let distance_m = dx.hypot(dy) * meters_per_unit;

    let mut azimuth_deg = dx.atan2(dy).to_degrees();
    if azimuth_deg < 0.0 {
        azimuth_deg += 360.0;
    }

    let in_range = distance_m + 1e-9 >= weapon.min_range_km * 1000.0
        && distance_m <= weapon.max_range_km * 1000.0 + 1e-9;

    let arc = |table: &[[f64; 2]]| {
        if in_range {
            lookup(table, distance_m)
        } else {
            None
        }
    };

    Solution {
        azimuth_deg,
        distance_m,
        single: arc(&weapon.ballistics.single),
        low: arc(&weapon.ballistics.low),
        high: arc(&weapon.ballistics.high),
        in_range,
    }
}

/// 火表查值。表项是 `[距离米, MIL]`。
///
/// `spg.high` 在数据里是**降序**存储的，所以不能省掉排序。
/// 距离上有多项时返回区间。
fn lookup(table: &[[f64; 2]], distance_m: f64) -> Option<Mil> {
    if table.is_empty() {
        return None;
    }

    let mut sorted = table.to_vec();
    sorted.sort_by(|a, b| a[0].total_cmp(&b[0]));

    const EPS: f64 = 1e-6;
    let hits: Vec<f64> = sorted
        .iter()
        .filter(|entry| (entry[0] - distance_m).abs() <= EPS)
        .map(|entry| entry[1])
        .collect();

    if !hits.is_empty() {
        let min = hits.iter().copied().fold(f64::INFINITY, f64::min);
        let max = hits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        return Some(if (max - min).abs() <= 1e-9 {
            Mil::Exact(min)
        } else {
            Mil::Range(min, max)
        });
    }

    let index = sorted.partition_point(|entry| entry[0] < distance_m);
    if index == 0 || index == sorted.len() {
        return None;
    }

    let left = sorted[index - 1];
    let right = sorted[index];
    let factor = (distance_m - left[0]) / (right[0] - left[0]);
    Some(Mil::Exact(left[1] + factor * (right[1] - left[1])))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_CATALOG: &str = include_str!("../data/weapons.json");

    fn catalog() -> Catalog {
        Catalog::from_json(REAL_CATALOG).expect("shipped weapons.json must parse")
    }

    fn approx(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-4,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn shipped_data_parses_and_has_both_weapons() {
        let catalog = catalog();
        assert_eq!(catalog.default, "mortar");
        assert_eq!(catalog.weapons.len(), 2);
        let mortar = catalog.get("mortar").unwrap();
        assert_eq!(mortar.min_range_km, 0.132);
        assert_eq!(mortar.max_range_km, 0.684);
        assert_eq!(mortar.ballistics.single.len(), 84);
        assert!(mortar.ballistics.low.is_empty());
        let spg = catalog.get("spg").unwrap();
        assert_eq!(spg.ballistics.low.len(), 59);
        assert_eq!(spg.ballistics.high.len(), 80);
    }

    #[test]
    fn distance_uses_map_scale() {
        let catalog = catalog();
        let mortar = catalog.get("mortar").unwrap();
        let solution = solve(mortar, Point::new(0.0, 0.0), Point::new(3.0, 4.0), METERS_PER_UNIT);
        approx(solution.distance_m, 500.0);
        assert!(solution.in_range);
    }

    #[test]
    fn azimuth_is_zero_at_north_and_clockwise() {
        let catalog = catalog();
        let mortar = catalog.get("mortar").unwrap();
        let origin = Point::new(0.0, 0.0);
        let bearing = |to| solve(mortar, origin, to, METERS_PER_UNIT).azimuth_deg;
        approx(bearing(Point::new(0.0, 1.0)), 0.0);
        approx(bearing(Point::new(1.0, 0.0)), 90.0);
        approx(bearing(Point::new(0.0, -1.0)), 180.0);
        approx(bearing(Point::new(-1.0, 0.0)), 270.0);
    }

    #[test]
    fn mortar_exact_table_distance() {
        let catalog = catalog();
        let mortar = catalog.get("mortar").unwrap();
        let solution = solve(mortar, Point::new(0.0, 0.0), Point::new(0.0, 2.5), METERS_PER_UNIT);
        approx(solution.distance_m, 250.0);
        assert_eq!(solution.mil(Arc::Single), Some(Mil::Exact(740.0)));
    }

    #[test]
    fn mortar_interpolates_between_table_rows() {
        let catalog = catalog();
        let mortar = catalog.get("mortar").unwrap();
        // 表里 [494,470] 和 [501,460] 之间插 500 m
        let solution = solve(mortar, Point::new(0.0, 0.0), Point::new(5.0, 0.0), METERS_PER_UNIT);
        approx(solution.distance_m, 500.0);
        approx(solution.mil(Arc::Single).unwrap().target(), 461.4286);
    }

    #[test]
    fn spg_low_arc_interpolates() {
        let catalog = catalog();
        let spg = catalog.get("spg").unwrap();
        let solution = solve(spg, Point::new(0.0, 0.0), Point::new(20.0, 0.0), METERS_PER_UNIT);
        approx(solution.distance_m, 2000.0);
        approx(solution.mil(Arc::Low).unwrap().target(), 206.0);
    }

    #[test]
    fn spg_high_arc_is_stored_descending_and_must_be_sorted() {
        let catalog = catalog();
        let spg = catalog.get("spg").unwrap();
        let table = &spg.ballistics.high;
        assert!(table[0][0] > table[table.len() - 1][0], "high table is descending in the data");

        // 1000 m 落在 [999,1340] 和 [1041,1330] 之间。
        // 不排序的话 partition_point 会直接返回 None。
        let solution = solve(spg, Point::new(0.0, 0.0), Point::new(0.0, 10.0), METERS_PER_UNIT);
        approx(solution.distance_m, 1000.0);
        approx(solution.mil(Arc::High).unwrap().target(), 1339.7619);
    }

    #[test]
    fn spg_arcs_converge_at_max_range_so_mil_is_a_range() {
        let catalog = catalog();
        let spg = catalog.get("spg").unwrap();
        let solution = solve(spg, Point::new(0.0, 0.0), Point::new(26.29, 0.0), METERS_PER_UNIT);
        approx(solution.distance_m, 2629.0);
        assert_eq!(solution.mil(Arc::Low), Some(Mil::Exact(600.0)));
        assert_eq!(solution.mil(Arc::High), Some(Mil::Range(610.0, 620.0)));
        approx(solution.mil(Arc::High).unwrap().uncertainty(), 5.0);
    }

    #[test]
    fn out_of_range_yields_no_solution_and_no_mil() {
        let catalog = catalog();
        let mortar = catalog.get("mortar").unwrap();
        let too_far = solve(mortar, Point::new(0.0, 0.0), Point::new(7.0, 0.0), METERS_PER_UNIT);
        approx(too_far.distance_m, 700.0);
        assert!(!too_far.in_range);
        assert_eq!(too_far.mil(Arc::Single), None);

        let too_close = solve(mortar, Point::new(0.0, 0.0), Point::new(1.0, 0.0), METERS_PER_UNIT);
        assert!(!too_close.in_range);
        assert_eq!(too_close.mil(Arc::Single), None);
    }

    #[test]
    fn lookup_refuses_to_extrapolate() {
        let table = [[100.0, 500.0], [200.0, 400.0]];
        assert_eq!(lookup(&table, 50.0), None);
        assert_eq!(lookup(&table, 250.0), None);
        assert_eq!(lookup(&table, 150.0), Some(Mil::Exact(450.0)));
        assert_eq!(lookup(&[], 150.0), None);
    }
}
