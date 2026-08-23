use windows::Win32::Foundation::{BOOL, HWND};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForSystem, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};

/// Return the monitor-effective scale even for DPI-unaware EQ windows.
pub(super) unsafe fn dpi_scale(hwnd: HWND) -> f64 {
    let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
    let mut dpi_x = 0u32;
    let mut dpi_y = 0u32;
    if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_ok() && dpi_x > 0 {
        return dpi_x as f64 / 96.0;
    }
    f64::from(GetDpiForSystem().max(96)) / 96.0
}

pub(super) fn scale(value: i32, factor: f64) -> i32 {
    (value as f64 * factor).round() as i32
}

pub(super) unsafe fn client_animations_enabled() -> bool {
    let mut enabled = BOOL(1);
    SystemParametersInfoW(
        SPI_GETCLIENTAREAANIMATION,
        0,
        Some((&mut enabled as *mut BOOL).cast()),
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
    )
    .is_err()
        || enabled.as_bool()
}
