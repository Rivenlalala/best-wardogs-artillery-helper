//! T1 截屏可行性探针。
//!
//! 只做一件事：证明能不能从游戏窗口拿到**活的**画面。
//!
//! 两种失败要分开：
//!   - 全黑：捕获被挡（BitBlt 打在 flip-model 上，或游戏设了 WDA_EXCLUDEFROMCAPTURE）
//!   - 冻结帧：读得到但永不更新。更阴险 —— 会在看似正常的情况下读到过期坐标。
//!
//! 用法：
//!   autoartillery-win                     列出所有窗口和显示器
//!   autoartillery-win WARDOGS            对标题匹配的窗口跑探针
//!   autoartillery-win --monitor          对主显示器跑探针（对照组）

use std::thread::sleep;
use std::time::Duration;
use xcap::{Monitor, Window};

// xcap 自己 re-export 了 image，直接复用可以避免版本不一致。
use xcap::image;

const FRAMES: usize = 3;
const FRAME_INTERVAL: Duration = Duration::from_millis(700);

/// 亮度低于此值即视该像素为黑。
const BLACK_LUMA: f32 = 6.0;

/// 两个像素可分辨的通道差阈值。
const PIXEL_DELTA: u8 = 8;

type Rgba = image::RgbaImage;

fn luma(pixel: &[u8]) -> f32 {
    0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32
}

fn black_fraction(image: &Rgba) -> f64 {
    let total = (image.width() as u64) * (image.height() as u64);
    if total == 0 {
        return 1.0;
    }
    let black = image
        .pixels()
        .filter(|pixel| luma(&pixel.0) < BLACK_LUMA)
        .count() as u64;
    black as f64 / total as f64
}

fn differing_fraction(a: &Rgba, b: &Rgba) -> f64 {
    if a.dimensions() != b.dimensions() {
        return 1.0;
    }
    let total = (a.width() as u64) * (a.height() as u64);
    if total == 0 {
        return 0.0;
    }
    let differing = a
        .pixels()
        .zip(b.pixels())
        .filter(|(left, right)| {
            left.0
                .iter()
                .zip(right.0.iter())
                .any(|(l, r)| l.abs_diff(*r) > PIXEL_DELTA)
        })
        .count() as u64;
    differing as f64 / total as f64
}

fn mean_luma(image: &Rgba) -> f32 {
    let total = (image.width() as u64) * (image.height() as u64);
    if total == 0 {
        return 0.0;
    }
    let sum: f64 = image.pixels().map(|pixel| luma(&pixel.0) as f64).sum();
    (sum / total as f64) as f32
}

fn list_windows() {
    println!("=== 显示器 ===");
    match Monitor::all() {
        Ok(monitors) => {
            for monitor in monitors {
                println!(
                    "  {}  {}x{}  primary={}",
                    monitor.name().unwrap_or_else(|_| "?".into()),
                    monitor.width().unwrap_or(0),
                    monitor.height().unwrap_or(0),
                    monitor.is_primary().unwrap_or(false),
                );
            }
        }
        Err(error) => println!("  枚举失败：{error}"),
    }

    println!("\n=== 窗口（按面积降序，仅非空标题）===");
    let Ok(windows) = Window::all() else {
        println!("  枚举失败");
        return;
    };

    let mut rows: Vec<_> = windows
        .into_iter()
        .filter_map(|window| {
            let title = window.title().unwrap_or_default();
            if title.trim().is_empty() {
                return None;
            }
            let width = window.width().unwrap_or(0);
            let height = window.height().unwrap_or(0);
            Some((
                width as u64 * height as u64,
                title,
                width,
                height,
                window.is_minimized().unwrap_or(false),
            ))
        })
        .collect();
    rows.sort_by(|left, right| right.0.cmp(&left.0));

    for (_, title, width, height, minimized) in rows.iter().take(25) {
        let flag = if *minimized { "  [最小化]" } else { "" };
        println!("  {width:>5}x{height:<5} {title}{flag}");
    }
    println!("\n用标题里的一个片段当参数跑探针，例如：autoartillery-win WARDOGS");
}

fn probe(pattern: &str) {
    let needle = pattern.to_lowercase();

    let target = if pattern == "--monitor" {
        None
    } else {
        let Ok(windows) = Window::all() else {
            eprintln!("枚举窗口失败");
            return;
        };
        let mut matches: Vec<_> = windows
            .into_iter()
            .filter(|window| {
                window
                    .title()
                    .map(|title| title.to_lowercase().contains(&needle))
                    .unwrap_or(false)
            })
            .collect();

        if matches.is_empty() {
            eprintln!("没有标题匹配 {pattern:?} 的窗口。先不带参数运行看列表。");
            return;
        }

        matches.sort_by_key(|window| {
            std::cmp::Reverse(
                window.width().unwrap_or(0) as u64 * window.height().unwrap_or(0) as u64,
            )
        });
        if matches.len() > 1 {
            println!("匹配到 {} 个窗口，取面积最大的：", matches.len());
            for window in &matches {
                println!("  {}", window.title().unwrap_or_default());
            }
        }
        Some(matches.remove(0))
    };

    match &target {
        Some(window) => {
            let title = window.title().unwrap_or_default();
            let (width, height) = (window.width().unwrap_or(0), window.height().unwrap_or(0));
            println!("目标窗口：{title:?}  {width}x{height}");
            if window.is_minimized().unwrap_or(false) {
                println!("警告：窗口处于最小化状态，捕获必然失败。请先切回游戏。");
            }
        }
        None => println!("目标：主显示器"),
    }

    println!();
    println!("即将抓 {FRAMES} 帧，每帧间隔 {} ms。", FRAME_INTERVAL.as_millis());
    println!("请在开始之后**移动视角或调整炮位**，让画面产生真实变化 ——");
    println!("否则静止的 HUD 会让'冻结帧'和'正常工作'无法区分。");
    println!();

    let mut frames: Vec<Rgba> = Vec::new();
    for index in 0..FRAMES {
        sleep(FRAME_INTERVAL);
        let captured = match &target {
            Some(window) => window.capture_image(),
            None => Monitor::all()
                .and_then(|monitors| {
                    monitors
                        .into_iter()
                        .find(|monitor| monitor.is_primary().unwrap_or(false))
                        .ok_or_else(|| xcap::XCapError::new("找不到主显示器"))
                })
                .and_then(|monitor| monitor.capture_image()),
        };

        match captured {
            Ok(image) => {
                println!(
                    "帧 {}: {}x{}  平均亮度 {:.1}  全黑像素占比 {:.1}%",
                    index + 1,
                    image.width(),
                    image.height(),
                    mean_luma(&image),
                    black_fraction(&image) * 100.0
                );
                frames.push(image);
            }
            Err(error) => {
                println!("帧 {}: 捕获失败 —— {error}", index + 1);
                return;
            }
        }
    }

    println!();
    for index in 1..frames.len() {
        println!(
            "帧 1 与帧 {} 的差异像素占比: {:.2}%",
            index + 1,
            differing_fraction(&frames[0], &frames[index]) * 100.0
        );
    }

    for (index, frame) in frames.iter().enumerate() {
        let path = format!("capture-frame-{}.png", index + 1);
        match frame.save(&path) {
            Ok(()) => println!("已保存 {path}"),
            Err(error) => println!("保存 {path} 失败：{error}"),
        }
    }

    println!();
    println!("=== 判定 ===");
    let all_black = frames.iter().all(|frame| black_fraction(frame) > 0.99);
    let any_change = (1..frames.len()).any(|index| differing_fraction(&frames[0], &frames[index]) > 0.001);

    if all_black {
        println!("全黑。捕获被挡。");
        println!("可能原因：仍在独占全屏（BitBlt 打在 flip-model 上），");
        println!("或游戏对窗口设了 WDA_EXCLUDEFROMCAPTURE。");
        println!("先切无边框窗口重跑；仍全黑则需换 Desktop Duplication。");
    } else if !any_change {
        println!("取到画面但 {} 帧完全一致。", frames.len());
        println!("如果你在采集期间确实移动了视角，这是**冻结帧** —— 比全黑更危险，");
        println!("因为它看起来正常，但坐标永远不会更新。");
        println!("如果画面本来就是静止的（没动视角、HUD 数字没变），这不构成结论，请重跑并制造变化。");
    } else {
        println!("通过。取到非全黑画面，且帧间存在真实变化。");
        println!("先打开 capture-frame-*.png 确认那是游戏画面而不仅是窗口边框。");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first() {
        None => list_windows(),
        Some(pattern) => probe(pattern),
    }
}
