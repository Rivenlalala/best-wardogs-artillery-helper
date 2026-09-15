//! 从游戏里复制出来的坐标文本。
//!
//! 游戏聊天框显示形如 `x99.01, y110.58` 的坐标，玩家整段选中复制即可。
//! 所以读取环节就是解析一个字符串 —— 不需要截屏，也不需要字形匹配。
//!
//! 实测过的真实样本（3840x2160 截图，队伍频道输入框）：
//! `x99.01, y110.58`，前面带 `队伍` 徽标，后面跟文本光标。

use crate::Point;

/// 解析一行坐标文本。
///
/// 容错范围：允许前后有别的文字（频道徽标等）、允许 `x`/`y` 后跟 `:` 或 `=`、
/// 允许任意空白、允许用逗号当小数点（`x99,01`）。
///
/// 但文本里出现**多于一对** x/y 时返回 None。复制到聊天记录而不是单个输入框时
/// 会发生这种情况，那时候取第一对很可能是过期的坐标 —— 宁可不出答案，
/// 也不要让玩家照着错值调炮。
pub fn parse_point(text: &str) -> Option<Point> {
    let found = scan(text);

    let xs: Vec<f64> = found
        .iter()
        .filter(|(label, _)| *label == 'x')
        .map(|(_, value)| *value)
        .collect();
    let ys: Vec<f64> = found
        .iter()
        .filter(|(label, _)| *label == 'y')
        .map(|(_, value)| *value)
        .collect();

    if xs.len() != 1 || ys.len() != 1 {
        return None;
    }
    Some(Point::new(xs[0], ys[0]))
}

/// 单遍扫描，收集所有 `x`/`y` 标签后的数值，以及每个数值结束的位置。
fn scan(text: &str) -> Vec<(char, f64)> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut index = 0;

    while index < chars.len() {
        let label = chars[index].to_ascii_lowercase();
        // 前面的字符是字母说明这是单词的一部分（例如 `max` 里的 x），不是坐标标签。
        let is_label = matches!(label, 'x' | 'y')
            && !(index > 0 && chars[index - 1].is_ascii_alphabetic());

        if is_label
            && let Some((value, next)) = number_at(&chars, index + 1)
        {
            found.push((label, value));
            index = next;
            continue;
        }
        index += 1;
    }

    found
}

/// 从 `start` 起解析一个数值，返回 (值, 结束位置)。
///
/// 逗号在这里有两种身份：`x99,01` 里是小数点，`x99.01, y110.58` 里是坐标分隔符。
/// 区分方式与上游 wardogs-calculator 一致 —— 小数点必须紧跟数字。
fn number_at(chars: &[char], start: usize) -> Option<(f64, usize)> {
    let mut index = start;
    while index < chars.len()
        && (chars[index].is_whitespace() || chars[index] == ':' || chars[index] == '=')
    {
        index += 1;
    }

    let mut text = String::new();
    if index < chars.len() && matches!(chars[index], '+' | '-') {
        text.push(chars[index]);
        index += 1;
    }

    let mut digits = 0;
    while index < chars.len() && chars[index].is_ascii_digit() {
        text.push(chars[index]);
        digits += 1;
        index += 1;
    }
    if digits == 0 {
        return None;
    }

    if index + 1 < chars.len()
        && matches!(chars[index], '.' | ',')
        && chars[index + 1].is_ascii_digit()
    {
        text.push('.');
        index += 1;
        while index < chars.len() && chars[index].is_ascii_digit() {
            text.push(chars[index]);
            index += 1;
        }
    }

    text.parse::<f64>().ok().map(|value| (value, index))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    /// 真实样本，从游戏截图里逐字读出来的。
    #[test]
    fn parses_the_real_game_string() {
        assert_eq!(parse_point("x99.01, y110.58"), Some(point(99.01, 110.58)));
    }

    #[test]
    fn tolerates_channel_badge_prefix() {
        // 队伍 = 队伍频道徽标，可能跟着一起被选中
        assert_eq!(
            parse_point("队伍 x99.01, y110.58"),
            Some(point(99.01, 110.58))
        );
        assert_eq!(
            parse_point("[队伍]x99.01, y110.58"),
            Some(point(99.01, 110.58))
        );
    }

    #[test]
    fn tolerates_separators_and_spacing() {
        for text in [
            "x99.01,y110.58",
            "x99.01, y110.58",
            "x: 99.01, y: 110.58",
            "x=99.01 y=110.58",
            "  x99.01  ,  y110.58  ",
            "x99.01,\r\ny110.58",
        ] {
            assert_eq!(
                parse_point(text),
                Some(point(99.01, 110.58)),
                "应当解析 {text:?}"
            );
        }
    }

    #[test]
    fn tolerates_decimal_comma() {
        assert_eq!(parse_point("x99,01, y110,58"), Some(point(99.01, 110.58)));
    }

    /// 逗号是小数点还是分隔符，靠"后面必须紧跟数字"区分。
    #[test]
    fn comma_is_a_separator_when_not_followed_by_digits() {
        assert_eq!(parse_point("x99, y110"), Some(point(99.0, 110.0)));
        assert_eq!(parse_point("x1000, y2000"), Some(point(1000.0, 2000.0)));
    }

    #[test]
    fn parses_integers_and_negatives() {
        assert_eq!(parse_point("x99, y110"), Some(point(99.0, 110.0)));
        assert_eq!(parse_point("x-12.5, y-3.25"), Some(point(-12.5, -3.25)));
        assert_eq!(parse_point("x+12.5, y+3.25"), Some(point(12.5, 3.25)));
    }

    #[test]
    fn rejects_missing_half() {
        assert_eq!(parse_point("x99.01"), None);
        assert_eq!(parse_point("y110.58"), None);
        assert_eq!(parse_point(""), None);
        assert_eq!(parse_point("hello"), None);
    }

    /// `max` 里的 x 不是坐标标签。前面是字母就必须跳过。
    ///
    /// 注意这个断言的方向：只有守卫**生效**时才会得到 Some。
    /// 如果 `max` 的 x 被当成标签，会多解析出一个 5.0，xs 变成长度 2，
    /// 于是整体返回 None —— 错误的断言（期望 None）在守卫坏掉时反而会通过。
    #[test]
    fn rejects_labels_inside_words() {
        assert_eq!(parse_point("max 99.01, y110.58"), None);
        assert_eq!(parse_point("box 99.01, key 110.58"), None);
        assert_eq!(
            parse_point("max 5, x99.01, y110.58"),
            Some(point(99.01, 110.58)),
            "`max` 里的 x 必须被忽略，只剩下真正的一对"
        );
    }

    /// 复制到聊天记录而不是单个输入框时会有多对坐标。
    /// 这时候取第一对很可能是过期的 —— 拒绝，不猜。
    #[test]
    fn refuses_ambiguous_multiple_pairs() {
        assert_eq!(
            parse_point("x1.00, y2.00\nx99.01, y110.58"),
            None,
            "两对坐标必须拒绝，不能悄悄取第一对"
        );
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_point("x, y"), None);
        assert_eq!(parse_point("xy"), None);
        assert_eq!(parse_point("x99.01, z110.58"), None);
    }

    /// 比例尺与游戏内显示位数：实测 2 位小数对应 coordinateMetersPerUnit = 100，
    /// 解析必须保住这两位，不能被浮点往返吃掉。
    #[test]
    fn keeps_two_decimal_places() {
        let parsed = parse_point("x99.01, y110.58").unwrap();
        assert_eq!(format!("{:.2}", parsed.x), "99.01");
        assert_eq!(format!("{:.2}", parsed.y), "110.58");
    }
}
