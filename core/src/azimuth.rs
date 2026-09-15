//! 方位角罗盘条几何：把方位角换算成屏幕坐标。
//!
//! 全部常数是从 3840x2160 无边框截图逐像素量出来的，见 epic 的 design note。
//! **换分辨率或改 HUD 缩放后必须重量** —— 比例是否线性没有实测过，不能假定。
//!
//! 用法：算出目标方位角后，用 [`nearby_ticks`] 和 [`tick_x`] 得到幽灵刻度的位置，
//! 玩家转动视角直到游戏的罗盘条刻度与它们重合 —— 那一刻方位角就等于目标值。

/// 准星垂直中心线的屏幕 x。罗盘条是刚性水平带，准星本身就是屏幕水平中心。
///
/// 实测：准星竖线中心 x≈1919.5；同一帧里罗盘条 180° 刻度线落在 x=1920.0，
/// 两者在 1px 内重合，说明"准星对准的刻度即当前方位角"这条假设成立。
pub const RETICLE_X: f64 = 1920.0;

/// 罗盘条的分辨率。实测 966.5 px = 87°（29 个 3° 小刻度间隔的平均值）。
pub const PX_PER_DEG: f64 = 11.11;

/// 相邻两条带数字的刻度线之间的方位角差。
pub const TICK_STEP_DEG: f64 = 15.0;

/// 罗盘条刻度线的垂直范围（左闭右开）。游戏刻度线落在 y≈600..644，
/// 我们的幽灵线不需要像素级匹配这段——对齐靠的是 x，不是 y——但要落在
/// 罗盘条附近且不挡准星（准星 T 标在 y≈950 起）。
pub const TICK_LINE_Y0: i32 = 600;
pub const TICK_LINE_Y1: i32 = 650;

/// 幽灵数字绘制框的垂直范围，紧贴刻度线上方。
pub const NUMBER_Y0: i32 = 520;
pub const NUMBER_Y1: i32 = 592;

/// 方位角是环形的：把 `deg` 归一化到 `[0, 360)`。
fn normalize(deg: f64) -> f64 {
    ((deg % 360.0) + 360.0) % 360.0
}

/// `tick_deg` 相对 `target_deg` 的最短带符号角差，落在 `(-180, 180]`。
///
/// 环绕关键：359° 的刻度相对 1° 的目标该算作 -2°（差 2°），不能算成 358°
/// （绕远路）。
fn shortest_diff(tick_deg: f64, target_deg: f64) -> f64 {
    let raw = normalize(tick_deg - target_deg + 180.0) - 180.0;
    raw
}

/// 某条刻度线在屏幕上的 x。
///
/// 罗盘条是刚性的：`x(v) = 准星线 + PX_PER_DEG * 最短角差(v, 目标)`。
/// 目标本身必然落在准星线上（这就是"对齐"的含义）。
pub fn tick_x(target_deg: f64, tick_deg: f64) -> f64 {
    RETICLE_X + PX_PER_DEG * shortest_diff(tick_deg, target_deg)
}

/// 覆盖层要画的 `count` 条幽灵刻度：`(刻度值, 屏幕 x)`。
///
/// x 四舍五入到整像素 —— 常数本身是逐像素量出来的，覆盖层不能因为
/// 半像素取整引入偏差。
pub fn tick_columns(target_deg: f64, count: usize) -> Vec<(f64, i32)> {
    nearby_ticks(target_deg, count)
        .into_iter()
        .map(|tick_deg| (tick_deg, tick_x(target_deg, tick_deg).round() as i32))
        .collect()
}

/// 目标附近的 `count` 条刻度值，对齐到 15 的整数倍，并归一化到 `[0, 360)`。
///
/// 必须是 15 的整数倍，因为游戏带子上只标这些值 —— 幽灵要写同样的数字，
/// 玩家才能靠"哪条线对应哪个数"来消除周期性带来的歧义。
pub fn nearby_ticks(target_deg: f64, count: usize) -> Vec<f64> {
    if count == 0 {
        return Vec::new();
    }
    let anchor = (target_deg / TICK_STEP_DEG).round() * TICK_STEP_DEG;
    let first = -(((count as i64) - 1) / 2) as f64;
    (0..count as i64)
        .map(|index| normalize(anchor + (first + index as f64) * TICK_STEP_DEG))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实截图的反算验证（同一帧，方位角 180.0，罗盘条五条标注刻度实测 x）。
    ///
    /// 帧实测：刻度 150/165/180/195/210 分别在 x = 1587/1754/1920/2087/2254，
    /// 准星竖线中心 x≈1920 恰好落在 180° 刻度上，说明当时方位角就是 180.0。
    /// 把它代回公式，五条刻度应该全部复现到 1px 以内。
    #[test]
    fn reproduces_measured_ticks_from_a_real_frame() {
        let current_deg = 180.0;
        for (tick_deg, measured_x) in [
            (150.0, 1587.0),
            (165.0, 1754.0),
            (180.0, 1920.0),
            (195.0, 2087.0),
            (210.0, 2254.0),
        ] {
            let computed = tick_x(current_deg, tick_deg).round();
            assert!(
                (computed - measured_x).abs() <= 1.0,
                "tick {tick_deg}: computed {computed}, measured {measured_x}"
            );
        }
    }

    #[test]
    fn target_tick_lands_exactly_on_reticle() {
        assert_eq!(tick_x(233.4, 233.4), RETICLE_X);
    }

    /// 0°/360° 环绕：359° 对着方位角 1° 的目标，最短角差是 -2°，不是 358°。
    #[test]
    fn wraps_around_zero_the_short_way() {
        let x_at_359 = tick_x(1.0, 359.0);
        let x_at_minus1 = tick_x(1.0, -1.0);
        assert_eq!(x_at_359, x_at_minus1, "359° 和 -1° 是同一条最短路径的刻度");
        assert!(x_at_359 < RETICLE_X, "359° 在目标 1° 的逆时针一侧，应在准星左边");
    }

    #[test]
    fn nearby_ticks_wrap_across_360_boundary() {
        let ticks = nearby_ticks(5.0, 3);
        assert_eq!(ticks, vec![345.0, 0.0, 15.0]);
    }

    #[test]
    fn nearby_ticks_align_to_15_degree_steps() {
        assert_eq!(nearby_ticks(182.0, 3), vec![165.0, 180.0, 195.0]);
    }
}
