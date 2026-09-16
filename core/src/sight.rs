//! 瞄准镜几何：把密位换算成屏幕坐标。
//!
//! 全部常数是从 3840x2160 无边框截图逐像素量出来的，见 epic 的 design note。
//! **换分辨率或改 HUD 缩放后必须重量** —— 比例是否线性没有实测过，不能假定。
//!
//! 用法：算出目标密位后，用 [`nearby_ticks`] 和 [`tick_y`] 得到幽灵刻度的位置，
//! 玩家滚动密位带直到游戏的四条线与它们重合 —— 那一刻密位就等于目标值。

/// 准星水平中心线的屏幕 y。
///
/// 三个独立元素互相印证：上下两条横杠(1878..1961)在 y≈960 和 y≈1200，
/// 中点 1080；方括号 `[ ]` 上下沿 1014/1145，中点 1080；
/// 中央菱形 1065..1094，中点 1079.5。
pub const RETICLE_Y: f64 = 1080.0;

/// 密位带的分辨率。实测 204 px = 50 MIL。
pub const PX_PER_MIL: f64 = 4.08;

/// 密位带两条刻度线之间的距离。实测恒定，没有更细的小刻度。
pub const TICK_SPACING_PX: f64 = 204.0;

/// 相邻两条刻度线的密位差。
pub const TICK_STEP_MIL: f64 = 50.0;

/// 密位带刻度线的水平范围（左闭右开）。实测 40 px 宽。
pub const TICK_LINE_X0: i32 = 2420;
pub const TICK_LINE_X1: i32 = 2460;

/// 幽灵数字的右界：画在刻度线**左侧**，右对齐贴着线。
/// 游戏自己的数字在线右边（x≈2480 起），分居两侧才不会叠字。
pub const NUMBER_RIGHT: i32 = TICK_LINE_X0 - 14;

/// 某条刻度线在屏幕上的 y。
///
/// 密位带是刚性的：密位越大越靠下，所以 `y(v) = 准星线 + 4.08 * (v - 目标)`。
/// 目标本身必然落在准星线上（这就是"对齐"的含义）。
pub fn tick_y(target_mil: f64, tick_mil: f64) -> f64 {
    RETICLE_Y + PX_PER_MIL * (tick_mil - target_mil)
}

/// 覆盖层要画的 `count` 条幽灵刻度：`(刻度值, 屏幕 y)`。
///
/// y 四舍五入到整像素 —— 常数本身是逐像素量出来的，覆盖层不能因为
/// 半像素取整引入偏差。
pub fn tick_rows(target_mil: f64, count: usize) -> Vec<(f64, i32)> {
    nearby_ticks(target_mil, count)
        .into_iter()
        .map(|tick_mil| (tick_mil, tick_y(target_mil, tick_mil).round() as i32))
        .collect()
}

/// 目标附近的 `count` 条刻度值，对齐到 50 的整数倍。
///
/// 必须是 50 的整数倍，因为游戏带子上只标这些值 —— 幽灵要写同样的数字，
/// 玩家才能靠"哪条线对应哪个数"来消除 204px 周期性带来的歧义。
pub fn nearby_ticks(target_mil: f64, count: usize) -> Vec<f64> {
    if count == 0 {
        return Vec::new();
    }
    let anchor = (target_mil / TICK_STEP_MIL).round() * TICK_STEP_MIL;
    let first = -(((count as i64) - 1) / 2) as f64;
    (0..count as i64)
        .map(|index| anchor + (first + index as f64) * TICK_STEP_MIL)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实截图的反算验证。
    ///
    /// 帧 B 的实测：刻度 450/500/550/600 分别在 y = 794/998/1202/1406，
    /// 用 y=1080 反推当时密位是 520.1。
    /// 把它代回公式，四条刻度应该全部复现到 1px 以内。
    #[test]
    fn reproduces_measured_ticks_from_a_real_frame() {
        let current_mil = 520.1;
        for (tick_mil, measured_y) in [
            (450.0, 794.0),
            (500.0, 998.0),
            (550.0, 1202.0),
            (600.0, 1406.0),
        ] {
            let predicted = tick_y(current_mil, tick_mil);
            assert!(
                (predicted - measured_y).abs() < 1.0,
                "刻度 {tick_mil} 预测 y={predicted:.1}，实测 {measured_y}"
            );
        }
    }

    /// 第二帧的反算，用不同的密位值，避免公式只在一点上碰巧成立。
    /// 帧 A 实测：800/850/900/950 在 y = 816/1020/1224/1428。
    #[test]
    fn reproduces_measured_ticks_from_the_other_frame() {
        let current_mil = 864.7;
        for (tick_mil, measured_y) in [
            (800.0, 816.0),
            (850.0, 1020.0),
            (900.0, 1224.0),
            (950.0, 1428.0),
        ] {
            let predicted = tick_y(current_mil, tick_mil);
            assert!(
                (predicted - measured_y).abs() < 1.0,
                "刻度 {tick_mil} 预测 y={predicted:.1}，实测 {measured_y}"
            );
        }
    }

    #[test]
    fn target_sits_on_the_reticle_line() {
        assert_eq!(tick_y(461.0, 461.0), RETICLE_Y);
        assert_eq!(tick_y(0.0, 0.0), RETICLE_Y);
    }

    #[test]
    fn higher_mil_is_lower_on_screen() {
        assert!(tick_y(500.0, 550.0) > RETICLE_Y);
        assert!(tick_y(500.0, 450.0) < RETICLE_Y);
    }

    #[test]
    fn spacing_matches_the_measured_ticks() {
        let a = tick_y(500.0, 450.0);
        let b = tick_y(500.0, 500.0);
        assert!((b - a - TICK_SPACING_PX).abs() < 1e-9);
    }

    #[test]
    fn nearby_ticks_snap_to_multiples_of_fifty() {
        assert_eq!(nearby_ticks(461.0, 4), vec![400.0, 450.0, 500.0, 550.0]);
        assert_eq!(nearby_ticks(520.1, 4), vec![450.0, 500.0, 550.0, 600.0]);
        // 正好在整数倍上时不要漂
        assert_eq!(nearby_ticks(550.0, 3), vec![500.0, 550.0, 600.0]);
        assert!(nearby_ticks(550.0, 0).is_empty());
    }

    /// 覆盖层的最终输入：像素取整后仍要落在实测位置 1px 内。
    #[test]
    fn tick_rows_round_without_drifting() {
        assert_eq!(
            tick_rows(520.1, 4),
            vec![(450.0, 794), (500.0, 998), (550.0, 1202), (600.0, 1406)]
        );
    }

    /// 幽灵图案必须把目标夹在中间，否则玩家只有单侧参照，无法判断偏移方向。
    #[test]
    fn nearby_ticks_bracket_the_target() {
        for target in [461.0, 120.0, 950.0, 1390.0, 80.0] {
            let ticks = nearby_ticks(target, 4);
            let lowest = ticks.iter().cloned().fold(f64::INFINITY, f64::min);
            let highest = ticks.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            assert!(
                lowest < target && target < highest,
                "目标 {target} 必须被 {ticks:?} 夹在中间"
            );
        }
    }
}
