//! 幽灵刻度覆盖层：一个全屏、点击穿透、绝不抢焦点、常驻最顶的分层窗口。
//!
//! 只在需要处绘制：密位带的四条刻度线 + 右侧数字，其余全透明。
//! 玩家滚动游戏密位带，直到游戏的刻度线与这四条重合 —— 那一刻密位就等于目标值。
//!
//! 刻度几何全部来自 [`sight`]（3840x2160 实测常数，测试已用真实截图验证到 1px）。
//! 本模块只做 Win32 摆放与画像素，不算几何。
//!
//! 约束（来自 epic 决策，不能破）：
//! • 绝不 hook DirectX —— 普通分层窗口不是注入，Steam/Discord 式渲染管线挂钩才是。
//! • 不含任何输入生成 —— 本文件没有任何 SendInput / keybd_event 之类的调用，
//!   反过来 TRANSPARENT 标志保证鼠标事件照常进游戏。
//! • 独占全屏下分层窗口画不上 —— 这是对 "只在无边框窗口时可见" 的全部处理。

/// 平台无关的外壳：建不起来就退化成空操作，调用方不用关心平台，
/// 但要把 [`Overlay::warning`] 显示出来 —— 没有控制台了，静默失败等于没告警。
pub struct Overlay {
    native: Option<native::Overlay>,
    warning: Option<String>,
}

impl Overlay {
    pub fn create() -> Self {
        match native::Overlay::create() {
            Ok(native) => Self { native: Some(native), warning: None },
            Err(why) => Self { native: None, warning: Some(why) },
        }
    }

    pub fn make_dpi_aware() {
        native::Overlay::make_dpi_aware();
    }

    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    pub fn show_ticks(&mut self, target_mil: f64, target_azimuth_deg: f64) {
        if let Some(native) = self.native.as_mut() {
            native.show_ticks(target_mil, target_azimuth_deg);
        }
    }

    pub fn clear(&mut self) {
        if let Some(native) = self.native.as_mut() {
            native.clear();
        }
    }

}

#[cfg(windows)]
mod native {
use std::slice;

use autoartillery_core::azimuth::{self, NUMBER_Y0, NUMBER_Y1, TICK_LINE_Y0, TICK_LINE_Y1};
use autoartillery_core::sight::{self, NUMBER_X0, NUMBER_X1, TICK_LINE_X0, TICK_LINE_X1};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, ANTIALIASED_QUALITY, BLENDFUNCTION, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateDIBSection, CreateFontW,
    DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, DrawTextW, GdiFlush, GetDC, HBITMAP, HDC, HFONT,
    OUT_DEFAULT_PRECIS, ReleaseDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetSystemMetrics, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN,
    SW_SHOWNOACTIVATE, ShowWindow,
    ULW_ALPHA, UpdateLayeredWindow, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

/// 刻度线：黑色描边 + 淡绿芯，比纯绿轻且细，但仍在任何游戏背景下看得见。
/// 线体 4px、描边各 1px。
const LINE_OUTLINE_HALF: i32 = 3;
const LINE_CORE_HALF: i32 = 2;
/// 注意 G 通道必须是 255（最亮通道），原因见 [`Overlay::draw_number`]。
const LINE_COLOR: u32 = 0xFFA0_FFA0;

/// 数字用同样的淡绿。GDI 文本与透明黑底混色后 alpha 字节不变，
/// 但 G 通道固定为 255（最亮），混色后的 G 字节恰好就是混色比例，
/// 直接拿它当 alpha，见 [`Overlay::draw_number`]。
const NUMBER_COLORREF: COLORREF = COLORREF(0x00A0_FFA0);
const NUMBER_FONT_HEIGHT: i32 = 64;

pub struct Overlay {
    hwnd: HWND,
    screen_dc: HDC,
    memory_dc: HDC,
    bitmap: HBITMAP,
    font: HFONT,
    /// DIB 的像素内存，预乘 ARGB，行序自上而下。
    bits: *mut u32,
    width: i32,
    height: i32,
}

impl Overlay {
    /// 建覆盖层。分辨率不符或 Win32 失败时返回 None（先打印显式警告），
    /// 程序退化为纯命令行模式。
    /// 必须在任何窗口创建前调用（包括 winit 的），否则显示缩放下坐标被虚拟化，
    /// 刻度常数按物理像素实测，对不上。旧系统没有 V2 就算了 —— 目标机是 Win10 1703+。
    pub fn make_dpi_aware() {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }

    pub fn create() -> Result<Self, String> {
        let (width, height) =
            unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        // 刻度常数按 3840x2160 + 当时 HUD 缩放实测，别的分辨率上位置不可信，
        // 与其画错不如不画。
        if (width, height) != (3840, 2160) {
            return Err(format!("no overlay: screen {width}x{height} is not 3840x2160"));
        }

        Self::create_window(width, height).map_err(|error| format!("no overlay: {error}"))
    }

    fn create_window(width: i32, height: i32) -> windows::core::Result<Self> {
        unsafe {
            let hinstance: windows::Win32::Foundation::HINSTANCE = GetModuleHandleW(None)?.into();

            let class_name = w!("autoartillery-overlay");
            let class = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinstance,
                lpszClassName: class_name,
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err(windows::core::Error::from_thread());
            }

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
                class_name,
                w!(""),
                WS_POPUP,
                0,
                0,
                width,
                height,
                None,
                None,
                Some(hinstance),
                None,
            )?;

            let screen_dc = GetDC(None);
            let memory_dc = CreateCompatibleDC(Some(screen_dc));

            let mut bits_ptr = std::ptr::null_mut();
            let header = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                // 负高度 = 自上而下的行序，与屏幕坐标同向。
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            };
            let info = BITMAPINFO { bmiHeader: header, ..Default::default() };
            let bitmap = CreateDIBSection(
                Some(memory_dc),
                &info,
                DIB_RGB_COLORS,
                &mut bits_ptr,
                None,
                0,
            )?;
            SelectObject(memory_dc, bitmap.into());

            let font = number_font();
            let overlay = Self {
                hwnd,
                screen_dc,
                memory_dc,
                bitmap,
                font,
                bits: bits_ptr as *mut u32,
                width,
                height,
            };
            overlay.composite();
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            Ok(overlay)
        }
    }

    /// 显示目标密位 T 与目标方位角的幽灵刻度：密位带四条横线，方位罗盘条五条竖线。
    pub fn show_ticks(&mut self, target_mil: f64, target_azimuth_deg: f64) {
        self.clear_buffer();
        for (tick_mil, y) in sight::tick_rows(target_mil, 4) {
            self.draw_rect(TICK_LINE_X0 - 2, TICK_LINE_X1 + 2, y - LINE_OUTLINE_HALF, y + LINE_OUTLINE_HALF, 0xFF00_0000);
            self.draw_rect(TICK_LINE_X0, TICK_LINE_X1, y - LINE_CORE_HALF, y + LINE_CORE_HALF, LINE_COLOR);
            self.draw_number(y, &format!("{tick_mil:.0}"));
        }
        for (tick_deg, x) in azimuth::tick_columns(target_azimuth_deg, 5) {
            self.draw_rect(x - LINE_OUTLINE_HALF, x + LINE_OUTLINE_HALF, TICK_LINE_Y0 - 2, TICK_LINE_Y1 + 2, 0xFF00_0000);
            self.draw_rect(x - LINE_CORE_HALF, x + LINE_CORE_HALF, TICK_LINE_Y0, TICK_LINE_Y1, LINE_COLOR);
            self.draw_number_at_x(x, &format!("{tick_deg:.0}"));
        }
        self.composite();
    }

    /// 清空显示。旧解算的刻度留在屏上会误导下一发。
    pub fn clear(&mut self) {
        self.clear_buffer();
        self.composite();
    }

    fn clear_buffer(&mut self) {
        unsafe {
            slice::from_raw_parts_mut(self.bits, (self.width * self.height) as usize).fill(0);
        }
    }

    /// 把预乘 ARGB 内存交给系统合成。位置尺寸在创建时就定死。
    fn composite(&self) {
        unsafe {
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let size = SIZE { cx: self.width, cy: self.height };
            if let Err(error) = UpdateLayeredWindow(
                self.hwnd,
                Some(self.screen_dc),
                Some(&POINT { x: 0, y: 0 }),
                Some(&size),
                Some(self.memory_dc),
                Some(&POINT { x: 0, y: 0 }),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            ) {
                eprintln!("覆盖层刷新失败：{error}");
            }
        }
    }

    fn draw_rect(&self, x0: i32, x1: i32, y0: i32, y1: i32, argb: u32) {
        for y in y0.max(0)..y1.min(self.height) {
            let row = unsafe { slice::from_raw_parts_mut(self.bits.add((y * self.width) as usize), self.width as usize) };
            for pixel in &mut row[x0.max(0) as usize..x1.min(self.width) as usize] {
                *pixel = argb;
            }
        }
    }

    /// 在刻度线右侧画数字。
    ///
    /// GDI 不认识 alpha：文字与透明黑底混色后，混色比例只留在颜色通道里，
    /// alpha 字节原样是 0。补救：NUMBER_COLORREF 的 G 通道固定为 255，
    /// 混色后的 G 字节就是混色比例，直接拿它当 alpha，得到的正好是
    /// 规范的预乘像素。这就是数字颜色必须把 G 定在 255 的原因，
    /// R/B 可以任意调轻但不能超过 G。
    fn draw_number(&self, line_y: i32, text: &str) {
        let bounds = RECT {
            left: NUMBER_X0,
            top: line_y - 2 * NUMBER_FONT_HEIGHT,
            right: NUMBER_X1,
            bottom: line_y + 2 * NUMBER_FONT_HEIGHT,
        };
        self.draw_text_recovering_alpha(bounds, text);
    }

    /// 在刻度线上方画数字，水平居中在 `line_x`。用于方位罗盘条——罗盘条是竖线
    /// 沿 x 排开，跟密位带的横线沿 y 排开正好转了 90 度，所以居中轴也跟着换。
    fn draw_number_at_x(&self, line_x: i32, text: &str) {
        let half_width = 2 * NUMBER_FONT_HEIGHT;
        let bounds = RECT {
            left: line_x - half_width,
            top: NUMBER_Y0,
            right: line_x + half_width,
            bottom: NUMBER_Y1,
        };
        self.draw_text_recovering_alpha(bounds, text);
    }

    /// GDI 不认识 alpha：文字与透明黑底混色后，混色比例只留在颜色通道里，
    /// alpha 字节原样是 0。补救：NUMBER_COLORREF 的 G 通道固定为 255，
    /// 混色后的 G 字节就是混色比例，直接拿它当 alpha，得到的正好是
    /// 规范的预乘像素。这就是数字颜色必须把 G 定在 255 的原因，
    /// R/B 可以任意调轻但不能超过 G。
    fn draw_text_recovering_alpha(&self, mut bounds: RECT, text: &str) {
        let mut utf16: Vec<u16> = text.encode_utf16().collect();
        let x0 = bounds.left.max(0) as usize;
        let x1 = bounds.right.min(self.width) as usize;
        unsafe {
            let previous = SelectObject(self.memory_dc, self.font.into());
            SetBkMode(self.memory_dc, TRANSPARENT);
            SetTextColor(self.memory_dc, NUMBER_COLORREF);
            DrawTextW(
                self.memory_dc,
                &mut utf16,
                &mut bounds,
                DT_SINGLELINE | DT_CENTER | DT_VCENTER | DT_NOPREFIX,
            );
            let _ = GdiFlush();
            SelectObject(self.memory_dc, previous);

            for y in bounds.top.max(0)..bounds.bottom.min(self.height) {
                let row = slice::from_raw_parts_mut(self.bits.add((y * self.width) as usize), self.width as usize);
                for pixel in &mut row[x0..x1] {
                    let blended = *pixel & 0x00FF_FFFF;
                    let coverage = (blended >> 8) & 0xFF;
                    *pixel = (coverage << 24) | blended;
                }
            }
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            // 先删 DC 再删位图：选入 DC 的对象删除会失败。
            let _ = DeleteDC(self.memory_dc);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteObject(self.font.into());
            // GetDC(None) 取的是屏幕 DC，ReleaseDC 的 hwnd 参数必须同样传 None。
            let _ = ReleaseDC(None, self.screen_dc);
        }
    }
}

/// 数字字体。字形常量按 windows 0.62 的裸 u32 参数传。
fn number_font() -> HFONT {
    unsafe {
        CreateFontW(
            -NUMBER_FONT_HEIGHT,
            0,
            0,
            0,
            600,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            DEFAULT_PITCH.0 as u32,
            w!("Segoe UI"),
        )
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // 覆盖层完全无交互：点击被 WS_EX_TRANSPARENT 挡在窗口外，绘制走
    // UpdateLayeredWindow 不经过 WM_PAINT，所以一切都归默认处理。
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}
}

#[cfg(not(windows))]
mod native {
    /// 非 Windows 平台没有覆盖层。
    pub struct Overlay;

    impl Overlay {
        pub fn make_dpi_aware() {}

        pub fn create() -> Result<Self, String> {
            Err("no overlay: windows only".to_string())
        }

        pub fn show_ticks(&mut self, _target_mil: f64, _target_azimuth_deg: f64) {}
        pub fn clear(&mut self) {}
    }
}
