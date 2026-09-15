//! HUD 读数：把屏幕上一小块像素变成数字。
//!
//! 这一层不依赖任何图像库，只吃灰度缓冲。裁剪与解码留在 `win` 层，
//! 匹配逻辑因此可以在开发机上 `cargo test` —— 目标机不是开发机，
//! 把每次调参都送过去跑一轮太贵。
//!
//! 前提假设：HUD 数字是**等宽**的。游戏 HUD 基本都是，
//! 这让切分退化成固定宽度切片，不需要连通域分析。

/// 灰度帧，通常是 HUD 区域裁剪后的小图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gray {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Gray {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        assert_eq!(
            pixels.len(),
            (width as usize) * (height as usize),
            "灰度缓冲尺寸与宽高不符"
        );
        Self {
            width,
            height,
            pixels,
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> u8 {
        self.pixels[(y * self.width + x) as usize]
    }

    /// 切一个子矩形。越界返回 None。
    pub fn crop(&self, x: u32, y: u32, width: u32, height: u32) -> Option<Gray> {
        if x + width > self.width || y + height > self.height {
            return None;
        }
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for row in 0..height {
            let start = ((y + row) * self.width + x) as usize;
            pixels.extend_from_slice(&self.pixels[start..start + width as usize]);
        }
        Some(Gray::new(width, height, pixels))
    }

    /// 等宽切分。区域宽度不是 cell_width 整数倍时，尾部余数丢弃。
    pub fn cells(&self, cell_width: u32) -> Vec<Gray> {
        if cell_width == 0 {
            return Vec::new();
        }
        let count = self.width / cell_width;
        (0..count)
            .map(|index| {
                let mut pixels = Vec::with_capacity((cell_width * self.height) as usize);
                for y in 0..self.height {
                    let start = (y * self.width + index * cell_width) as usize;
                    pixels.extend_from_slice(&self.pixels[start..start + cell_width as usize]);
                }
                Gray::new(cell_width, self.height, pixels)
            })
            .collect()
    }

    /// 平均亮度。用于诊断（全黑检测、是否反色）。
    pub fn mean(&self) -> f32 {
        if self.pixels.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.pixels.iter().map(|value| *value as u64).sum();
        (sum as f64 / self.pixels.len() as f64) as f32
    }

    /// 亮度标准差。接近 0 说明是纯色块（空白格）。
    pub fn deviation(&self) -> f32 {
        if self.pixels.is_empty() {
            return 0.0;
        }
        let mean = self.mean() as f64;
        let variance: f64 = self
            .pixels
            .iter()
            .map(|value| {
                let delta = *value as f64 - mean;
                delta * delta
            })
            .sum::<f64>()
            / self.pixels.len() as f64;
        variance.sqrt() as f32
    }
}

/// 归一化互相关，返回 [-1, 1]。
///
/// 用 NCC 而不是绝对差，是因为 HUD 的绝对亮度会变：HDR、伽马、
/// 场景明暗、天气都会改它。NCC 对线性亮度/对比度变化不敏感，
/// 而字形形状信息全部保留。
pub fn ncc(left: &[u8], right: &[u8]) -> f32 {
    assert_eq!(left.len(), right.len(), "NCC 要求两个缓冲等长");
    if left.is_empty() {
        return 0.0;
    }

    let count = left.len() as f64;
    let mean_left = left.iter().map(|value| *value as f64).sum::<f64>() / count;
    let mean_right = right.iter().map(|value| *value as f64).sum::<f64>() / count;

    let mut covariance = 0.0;
    let mut variance_left = 0.0;
    let mut variance_right = 0.0;
    for (a, b) in left.iter().zip(right.iter()) {
        let da = *a as f64 - mean_left;
        let db = *b as f64 - mean_right;
        covariance += da * db;
        variance_left += da * da;
        variance_right += db * db;
    }

    // 常量块（空白格）方差为 0，相关系数在数学上无定义。
    // 两个都是常量且亮度接近时算完全匹配，否则算不匹配 ——
    // 这样空白格不会碰巧匹配上某个字形。
    if variance_left <= f64::EPSILON || variance_right <= f64::EPSILON {
        return if (mean_left - mean_right).abs() < 1.0 {
            1.0
        } else {
            0.0
        };
    }

    (covariance / (variance_left.sqrt() * variance_right.sqrt())) as f32
}

/// 一个字形模板。等宽场景下所有模板尺寸相同。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glyph {
    pub ch: char,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Glyph {
    pub fn from_gray(ch: char, gray: &Gray) -> Self {
        Self {
            ch,
            width: gray.width,
            height: gray.height,
            pixels: gray.pixels.clone(),
        }
    }

    /// 与一格比较。尺寸不符返回 None（调用方应视为配置错误）。
    pub fn score(&self, cell: &Gray) -> Option<f32> {
        if self.width != cell.width || self.height != cell.height {
            return None;
        }
        Some(ncc(&self.pixels, &cell.pixels))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellRead {
    pub ch: char,
    pub score: f32,
}

/// 逐格识别，每格取分数最高的模板。
///
/// 返回 None 表示有模板尺寸与格子对不上 —— 那是配置错误，不是"读不出来"。
/// 单个格子返回 None 表示该格没有可用匹配（例如空白格）。
pub fn read_cells(
    region: &Gray,
    templates: &[Glyph],
    cell_width: u32,
) -> Option<Vec<Option<CellRead>>> {
    if templates.is_empty() {
        return None;
    }

    let mut cells = Vec::new();
    for cell in region.cells(cell_width) {
        let mut best: Option<CellRead> = None;
        let mut saw_any = false;
        for template in templates {
            let Some(score) = template.score(&cell) else {
                continue;
            };
            saw_any = true;
            if best.is_none_or(|current| score > current.score) {
                best = Some(CellRead {
                    ch: template.ch,
                    score,
                });
            }
        }
        if !saw_any {
            return None;
        }
        cells.push(best);
    }

    Some(cells)
}

/// 把逐格结果拼成数值。
///
/// 任何一格读失败、分数低于 `min_score`、或出现非法字符，一律返回 None。
/// **不猜** —— 一个错的小数点在炮兵表里就是几百米。
pub fn parse_number(cells: &[Option<CellRead>], min_score: f32) -> Option<f64> {
    let mut text = String::new();
    for cell in cells {
        let read = cell.as_ref()?;
        if read.score < min_score {
            return None;
        }
        if !read.ch.is_ascii_digit() && read.ch != '.' && read.ch != '-' {
            return None;
        }
        text.push(read.ch);
    }

    if !text.chars().any(|ch| ch.is_ascii_digit()) {
        return None;
    }
    text.parse::<f64>().ok()
}

/// 连续 N 帧读到同一个值才采信。
///
/// 防的是瞬时误读：动画中间帧、数字切换的过渡帧、临时遮挡。
/// 一次读取失败会把连续计数清零，而不是沿用旧值 ——
/// 沿用旧值等于把过期坐标当成当前坐标。
#[derive(Debug, Clone)]
pub struct StabilityGate {
    required: usize,
    last: Option<f64>,
    streak: usize,
}

impl StabilityGate {
    pub fn new(required: usize) -> Self {
        assert!(required >= 1, "至少要求一帧");
        Self {
            required,
            last: None,
            streak: 0,
        }
    }

    /// 投入一次观测。None 表示这一帧读失败。
    /// 返回 Some(值) 表示已连续 `required` 帧一致，可以采信。
    pub fn observe(&mut self, observation: Option<f64>) -> Option<f64> {
        match observation {
            Some(value) => {
                if self.last == Some(value) {
                    self.streak += 1;
                } else {
                    self.last = Some(value);
                    self.streak = 1;
                }
                (self.streak >= self.required).then_some(value)
            }
            None => {
                self.last = None;
                self.streak = 0;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use font8x8::{BASIC_FONTS, UnicodeFonts};

    const BACKGROUND: u8 = 40;
    const FOREGROUND: u8 = 220;

    /// 用 font8x8 把字符串渲染成灰度图，当作合成 HUD。
    /// 真实字形要从目标机截图里抠，但算法验证不需要真字形。
    fn render(text: &str, scale: u32) -> Gray {
        let glyph_side = 8 * scale;
        let width = glyph_side * text.chars().count() as u32;
        let mut pixels = vec![BACKGROUND; (width * glyph_side) as usize];

        for (index, ch) in text.chars().enumerate() {
            let bitmap = BASIC_FONTS.get(ch).expect("font8x8 覆盖基本 ASCII");
            for (row, bits) in bitmap.iter().enumerate() {
                for col in 0..8u32 {
                    if bits & (1 << col) == 0 {
                        continue;
                    }
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let x = index as u32 * glyph_side + col * scale + dx;
                            let y = row as u32 * scale + dy;
                            pixels[(y * width + x) as usize] = FOREGROUND;
                        }
                    }
                }
            }
        }

        Gray::new(width, glyph_side, pixels)
    }

    /// 模拟 HUD 亮度变化：线性变换 y = a*x + b，不截断。
    fn relight(gray: &Gray, scale: f32, offset: f32) -> Gray {
        let pixels = gray
            .pixels
            .iter()
            .map(|value| (*value as f32 * scale + offset).clamp(0.0, 255.0) as u8)
            .collect();
        Gray::new(gray.width, gray.height, pixels)
    }

    fn templates(scale: u32) -> Vec<Glyph> {
        "0123456789.-"
            .chars()
            .map(|ch| Glyph::from_gray(ch, &render(&ch.to_string(), scale)))
            .collect()
    }

    #[test]
    fn ncc_of_identical_buffers_is_one() {
        let gray = render("7", 3);
        assert!((ncc(&gray.pixels, &gray.pixels) - 1.0).abs() < 1e-6);
    }

    /// 这条测试是选用 NCC 的全部理由：HUD 的绝对亮度会变，
    /// 形状不变。绝对差在这里会崩，NCC 不会。
    #[test]
    fn ncc_survives_brightness_and_contrast_change() {
        let base = render("4", 4);
        for (scale, offset) in [(1.0, 30.0), (0.6, 0.0), (0.6, 30.0), (1.2, -20.0)] {
            let shifted = relight(&base, scale, offset);
            let score = ncc(&base.pixels, &shifted.pixels);
            assert!(
                score > 0.99,
                "scale={scale} offset={offset} 只让 NCC 掉到 {score}"
            );
        }
    }

    #[test]
    fn reads_a_rendered_number() {
        let hud = render("83.64", 4);
        let templates = templates(4);
        let cells = read_cells(&hud, &templates, 32).expect("模板尺寸应匹配");
        assert_eq!(cells.len(), 5);
        assert_eq!(
            cells.iter().map(|c| c.unwrap().ch).collect::<String>(),
            "83.64"
        );
        assert_eq!(parse_number(&cells, 0.8), Some(83.64));
    }

    #[test]
    fn reads_number_after_relighting() {
        let hud = render("12.30", 4);
        let dimmed = relight(&hud, 0.6, 30.0);
        let templates = templates(4);
        let cells = read_cells(&dimmed, &templates, 32).unwrap();
        assert_eq!(parse_number(&cells, 0.8), Some(12.30));
    }

    /// 区域裁剪偏一个像素就会掉分。说明 HUD 区域必须逐个标定到像素，
    /// 不能靠估计 —— 也说明分数阈值不能设得太贴近 1.0。
    #[test]
    fn one_pixel_misalignment_costs_score() {
        // 渲染两格，取第一格作为对齐样本，再从 x=1 处取同样尺寸的窗口作偏移样本。
        let hud = render("55", 4);
        let glyph_side = 32;
        let aligned = hud.crop(0, 0, glyph_side, glyph_side).unwrap();
        let misaligned = hud.crop(1, 0, glyph_side, glyph_side).unwrap();
        let template = Glyph::from_gray('5', &aligned);

        let aligned_score = template.score(&aligned).unwrap();
        let misaligned_score = template.score(&misaligned).unwrap();
        assert!(
            aligned_score > 0.99,
            "对齐时应该接近 1，实际 {aligned_score}"
        );
        assert!(
            misaligned_score < aligned_score,
            "偏一像素应该掉分：{misaligned_score} vs {aligned_score}"
        );
    }

    #[test]
    fn crop_out_of_bounds_is_refused() {
        let gray = render("1", 2);
        assert!(gray.crop(0, 0, gray.width, gray.height).is_some());
        assert!(gray.crop(1, 0, gray.width, gray.height).is_none());
    }

    /// 空白格方差为 0，不能碰巧匹配上任何字形。
    #[test]
    fn blank_cell_matches_nothing() {
        let templates = templates(4);
        let blank = Gray::new(32, 32, vec![BACKGROUND; 32 * 32]);
        let cells = read_cells(&blank, &templates, 32).unwrap();
        assert_eq!(cells.len(), 1);
        let read = cells[0].unwrap();
        assert!(
            read.score < 0.5,
            "空白格不该匹配到 {:?}，分数 {}",
            read.ch,
            read.score
        );
        assert_eq!(parse_number(&cells, 0.8), None);
    }

    /// 低分必须拒绝，而不是猜一个最像的。
    #[test]
    fn low_confidence_is_refused_not_guessed() {
        let hud = render("83.64", 4);
        let templates = templates(4);
        let cells = read_cells(&hud, &templates, 32).unwrap();
        assert_eq!(parse_number(&cells, 0.99), Some(83.64));
        // 阈值设到不可能达到的高度，必须整段拒绝
        assert_eq!(parse_number(&cells, 1.5), None);
    }

    #[test]
    fn illegal_characters_are_refused() {
        let cells = vec![
            Some(CellRead {
                ch: '8',
                score: 0.95,
            }),
            Some(CellRead {
                ch: ' ',
                score: 0.9,
            }),
        ];
        assert_eq!(parse_number(&cells, 0.8), None);
    }

    #[test]
    fn non_numeric_text_is_refused() {
        let cells = vec![Some(CellRead {
            ch: '.',
            score: 0.99,
        })];
        assert_eq!(parse_number(&cells, 0.8), None);
    }

    #[test]
    fn gate_needs_consecutive_agreement() {
        let mut gate = StabilityGate::new(3);
        assert_eq!(gate.observe(Some(83.64)), None);
        assert_eq!(gate.observe(Some(83.64)), None);
        assert_eq!(gate.observe(Some(83.64)), Some(83.64));
        // 一旦稳定，继续一致仍然采信
        assert_eq!(gate.observe(Some(83.64)), Some(83.64));
    }

    #[test]
    fn gate_rejects_flickering_reads() {
        let mut gate = StabilityGate::new(3);
        for value in [83.64, 88.64, 83.64, 88.64, 83.64] {
            assert_eq!(gate.observe(Some(value)), None, "闪变不该被采信");
        }
    }

    /// 读失败清零连续计数。沿用旧值等于把过期坐标当当前坐标 ——
    /// 这正是 T1 里说的"冻结帧"在读数层的等价物。
    #[test]
    fn gate_resets_on_failed_read() {
        let mut gate = StabilityGate::new(3);
        assert_eq!(gate.observe(Some(5.0)), None);
        assert_eq!(gate.observe(Some(5.0)), None);
        assert_eq!(gate.observe(None), None);
        assert_eq!(gate.observe(Some(5.0)), None, "失败后必须重新计数");
        assert_eq!(gate.observe(Some(5.0)), None);
        assert_eq!(gate.observe(Some(5.0)), Some(5.0));
    }
}
