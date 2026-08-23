use windows::Win32::Foundation::{HWND, RPC_E_CHANGED_MODE};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

use super::labels::{Color, LabelTheme};
use super::render::Compositor;
use super::toast::ToastState;
use crate::diagnostics::debug_log;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ActiveSceneKey {
    pub(super) text: String,
    pub(super) class: Option<String>,
    pub(super) color: u32,
    pub(super) number: usize,
    pub(super) label_height: i32,
    pub(super) label_alpha: u8,
    pub(super) dpi_bits: u64,
    pub(super) theme: LabelTheme,
    pub(super) timer_label: Option<String>,
    pub(super) timer_start: Option<std::time::Instant>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BannerSceneKey {
    pub(super) text: String,
    pub(super) background: Color,
    pub(super) label_height: i32,
    pub(super) label_alpha: u8,
    pub(super) dpi_bits: u64,
}

pub(super) struct PipWindowEntry {
    pub(super) hwnd: HWND,
    pub(super) label_hwnd: HWND,
    pub(super) pid: u32,
    pub(super) thumb: isize,
    pub(super) label: String,
    pub(super) class: Option<String>,
    pub(super) number: usize,
    pub(super) hovered: bool,
}

pub(super) struct ComApartment {
    pub(super) usable: bool,
    uninitialize: bool,
}

impl ComApartment {
    pub(super) unsafe fn initialize() -> Self {
        match CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() {
            Ok(()) => Self {
                usable: true,
                uninitialize: true,
            },
            Err(error) if error.code() == RPC_E_CHANGED_MODE => Self {
                usable: true,
                uninitialize: false,
            },
            Err(error) => {
                debug_log(&format!(
                    "DirectComposition COM initialization failed: {error}"
                ));
                Self {
                    usable: false,
                    uninitialize: false,
                }
            }
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { CoUninitialize() };
        }
    }
}

/// Physical overlay windows and their authored composition state.
pub(super) struct PresentationState {
    /// The hardware compositor drops before its balanced COM apartment owner.
    pub(super) compositor: Option<Compositor>,
    pub(super) com_apartment: ComApartment,
    pub(super) pip_windows: Vec<PipWindowEntry>,
    pub(super) pending_composition_destroys: Vec<HWND>,
    pub(super) active_label_hwnd: HWND,
    pub(super) active_label_text: String,
    pub(super) active_label_class: Option<String>,
    pub(super) active_label_color: u32,
    pub(super) active_label_number: usize,
    pub(super) active_label_hovered: bool,
    pub(super) active_scene_key: Option<ActiveSceneKey>,
    pub(super) banner_scene_key: Option<BannerSceneKey>,
    pub(super) broadcast_label_hwnd: HWND,
    pub(super) label_height: i32,
    pub(super) label_alpha: u8,
    pub(super) label_theme: LabelTheme,
    pub(super) thumbnail_alpha: u8,
    pub(super) toast: ToastState,
}
