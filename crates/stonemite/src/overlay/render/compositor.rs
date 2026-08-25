//! Hardware Direct2D/DirectComposition presentation resources.
//!
//! The compositor deliberately owns no overlay domain state and exposes only
//! text measurement, scene presentation, and HWND surface lifecycle.

use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use std::rc::Rc;

use windows::core::{Error as WindowsError, Interface, Result as WindowsResult, HRESULT, PCWSTR};
use windows::Win32::Foundation::{
    D2DERR_RECREATE_TARGET, E_FAIL, E_INVALIDARG, HMODULE, HWND, RPC_E_WRONG_THREAD,
};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateDevice, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Image,
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionEffectGroup, IDCompositionTarget,
    IDCompositionVisual,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIDevice1, IDXGIFactory2, IDXGIOutput, IDXGISurface, IDXGISwapChain1,
    DXGI_ERROR_DEVICE_HUNG, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
    DXGI_ERROR_DRIVER_INTERNAL_ERROR, DXGI_PRESENT, DXGI_PRESENT_PARAMETERS, DXGI_SCALING_STRETCH,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::System::Threading::GetCurrentThreadId;

use super::super::labels::{required_width, FontSpec, LabelModel, LabelStyle, LabelTheme};
use super::super::scenes::{
    ActiveLabelScene, PipScene, StatusBannerScene, StonemiteButtonScene, ToastScene, UiTextRole,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FailureClass {
    DeviceLost,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FailureAction {
    RecoverDevice,
    PreserveGraph,
}

fn classify_hresult(code: HRESULT) -> FailureClass {
    if matches!(
        code,
        D2DERR_RECREATE_TARGET
            | DXGI_ERROR_DEVICE_HUNG
            | DXGI_ERROR_DEVICE_REMOVED
            | DXGI_ERROR_DEVICE_RESET
            | DXGI_ERROR_DRIVER_INTERNAL_ERROR
    ) {
        FailureClass::DeviceLost
    } else {
        FailureClass::Other
    }
}

fn failure_action(error: &WindowsError) -> FailureAction {
    match classify_hresult(error.code()) {
        FailureClass::DeviceLost => FailureAction::RecoverDevice,
        FailureClass::Other => FailureAction::PreserveGraph,
    }
}

fn changed_opacity(authoritative: f32, requested: f32) -> Option<f32> {
    let clamped = requested.clamp(0.0, 1.0);
    (authoritative != clamped).then_some(clamped)
}

fn recovered_operation_result(
    original_error: WindowsError,
    recovery: WindowsResult<()>,
) -> WindowsResult<()> {
    match recovery {
        Ok(()) => Err(original_error),
        Err(recovery_error) => Err(recovery_error),
    }
}

fn missing_resource(name: &str) -> WindowsError {
    WindowsError::new(E_FAIL, format!("{name} was not returned"))
}

fn invalid_operation(message: &str) -> WindowsError {
    WindowsError::new(E_INVALIDARG, message)
}

fn check_thread(owner_thread_id: u32, current_thread_id: u32) -> WindowsResult<()> {
    if owner_thread_id == current_thread_id {
        Ok(())
    } else {
        Err(WindowsError::new(
            RPC_E_WRONG_THREAD,
            format!(
                "DirectComposition compositor belongs to thread {owner_thread_id}, called from {current_thread_id}"
            ),
        ))
    }
}

fn error_context(context: &str, error: WindowsError) -> WindowsError {
    WindowsError::new(error.code(), format!("{context}: {error}"))
}

fn surface_key(hwnd: HWND) -> isize {
    hwnd.0 as isize
}

fn physical_size(width: u32, height: u32) -> (u32, u32) {
    (width.max(1), height.max(1))
}

#[derive(Clone)]
pub(super) struct TextResources {
    pub(super) factory: IDWriteFactory,
}

impl TextResources {
    unsafe fn new() -> WindowsResult<Self> {
        Ok(Self {
            factory: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
        })
    }

    pub(super) unsafe fn text_format(
        &self,
        spec: &FontSpec,
        height: i32,
        centered: bool,
    ) -> WindowsResult<IDWriteTextFormat> {
        self.text_format_with_alignment(
            spec,
            height,
            if centered {
                DWRITE_TEXT_ALIGNMENT_CENTER
            } else {
                DWRITE_TEXT_ALIGNMENT_LEADING
            },
        )
    }

    pub(super) unsafe fn text_format_with_alignment(
        &self,
        spec: &FontSpec,
        height: i32,
        alignment: DWRITE_TEXT_ALIGNMENT,
    ) -> WindowsResult<IDWriteTextFormat> {
        let family: Vec<u16> = spec
            .family
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let locale = crate::font_catalog::user_locale_name_wide();
        let format = self.factory.CreateTextFormat(
            PCWSTR(family.as_ptr()),
            None,
            DWRITE_FONT_WEIGHT(i32::from(spec.weight)),
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            height.max(1) as f32,
            PCWSTR(locale.as_ptr()),
        )?;
        format.SetTextAlignment(alignment)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        Ok(format)
    }

    unsafe fn measure_text(&self, text: &str, spec: &FontSpec, height: i32) -> WindowsResult<i32> {
        if text.is_empty() {
            return Ok(0);
        }
        let format = self.text_format(spec, height, false)?;
        let text: Vec<u16> = text.encode_utf16().collect();
        let layout = self
            .factory
            .CreateTextLayout(&text, &format, 100_000.0, 100_000.0)?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        layout.GetMetrics(&mut metrics)?;
        Ok(metrics.widthIncludingTrailingWhitespace.ceil() as i32)
    }
}

struct GraphicsResources {
    d3d_device: ID3D11Device,
    dxgi_device: IDXGIDevice,
    dxgi_factory: IDXGIFactory2,
    _d2d_device: ID2D1Device,
    d2d_context: ID2D1DeviceContext,
    class_icon_bitmaps: HashMap<&'static str, ID2D1Bitmap1>,
    stonemite_icon_bitmap: Option<ID2D1Bitmap1>,
}

impl GraphicsResources {
    unsafe fn new() -> WindowsResult<Self> {
        let mut d3d_device = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut d3d_device),
            None,
            None,
        )
        .map_err(|error| error_context("D3D11CreateDevice", error))?;
        let d3d_device = d3d_device.ok_or_else(|| missing_resource("ID3D11Device"))?;
        let dxgi_device: IDXGIDevice = d3d_device
            .cast()
            .map_err(|error| error_context("ID3D11Device::cast<IDXGIDevice>", error))?;
        if let Ok(dxgi_device1) = dxgi_device.cast::<IDXGIDevice1>() {
            let _ = dxgi_device1.SetMaximumFrameLatency(1);
        }
        let adapter = dxgi_device
            .GetAdapter()
            .map_err(|error| error_context("IDXGIDevice::GetAdapter", error))?;
        let dxgi_factory: IDXGIFactory2 = adapter
            .GetParent()
            .map_err(|error| error_context("IDXGIAdapter::GetParent<IDXGIFactory2>", error))?;
        let d2d_device = D2D1CreateDevice(&dxgi_device, None)
            .map_err(|error| error_context("D2D1CreateDevice", error))?;
        let d2d_context = d2d_device
            .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
            .map_err(|error| error_context("ID2D1Device::CreateDeviceContext", error))?;
        Ok(Self {
            d3d_device,
            dxgi_device,
            dxgi_factory,
            _d2d_device: d2d_device,
            d2d_context,
            class_icon_bitmaps: HashMap::new(),
            stonemite_icon_bitmap: None,
        })
    }

    unsafe fn stonemite_icon_bitmap(&mut self) -> WindowsResult<ID2D1Bitmap1> {
        if let Some(bitmap) = self.stonemite_icon_bitmap.as_ref() {
            return Ok(bitmap.clone());
        }
        let (width, height, pixels) = crate::tray::stonemite_icon_bgra()
            .ok_or_else(|| invalid_operation("the embedded Stonemite logo could not be decoded"))?;
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            ..Default::default()
        };
        let bitmap = self.d2d_context.CreateBitmap(
            D2D_SIZE_U { width, height },
            Some(pixels.as_ptr().cast()),
            width * 4,
            &properties,
        )?;
        self.stonemite_icon_bitmap = Some(bitmap.clone());
        Ok(bitmap)
    }

    unsafe fn class_icon_bitmap(
        &mut self,
        class_abbreviation: &str,
    ) -> WindowsResult<Option<ID2D1Bitmap1>> {
        let Some((key, icon)) = crate::class_icons::class_icon(class_abbreviation) else {
            return Ok(None);
        };
        if let Some(bitmap) = self.class_icon_bitmaps.get(key) {
            return Ok(Some(bitmap.clone()));
        }

        let pixels = icon.premultiplied_bgra();
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            ..Default::default()
        };
        let bitmap = self.d2d_context.CreateBitmap(
            D2D_SIZE_U {
                width: icon.width,
                height: icon.height,
            },
            Some(pixels.as_ptr().cast()),
            icon.width * 4,
            &properties,
        )?;
        self.class_icon_bitmaps.insert(key, bitmap.clone());
        Ok(Some(bitmap))
    }
}

struct DeviceResources {
    graphics: GraphicsResources,
    composition_device: IDCompositionDevice,
}

impl DeviceResources {
    unsafe fn new() -> WindowsResult<Self> {
        let graphics = GraphicsResources::new()?;
        let composition_device = DCompositionCreateDevice(&graphics.dxgi_device)
            .map_err(|error| error_context("DCompositionCreateDevice", error))?;
        Ok(Self {
            graphics,
            composition_device,
        })
    }

    fn log_removed_reason(&self, context: &str) {
        let result = unsafe { self.graphics.d3d_device.GetDeviceRemovedReason() };
        if let Err(reason) = result {
            crate::diagnostics::debug_log(&format!(
                "DirectComposition {context}; device removed reason: {reason}"
            ));
        }
    }
}

fn insert_after_prepare<K: Eq + std::hash::Hash, V>(
    registrations: &mut HashMap<K, V>,
    key: K,
    prepare: impl FnOnce() -> WindowsResult<V>,
) -> WindowsResult<()> {
    let prepared = prepare()?;
    registrations.insert(key, prepared);
    Ok(())
}

fn remove_after_commit<K: Eq + std::hash::Hash, V>(
    registrations: &mut HashMap<K, V>,
    key: &K,
    commit: impl FnOnce(&V) -> WindowsResult<()>,
) -> WindowsResult<Option<V>> {
    let Some(registration) = registrations.get(key) else {
        return Ok(None);
    };
    commit(registration)?;
    Ok(registrations.remove(key))
}

fn publish_completed_frame(
    draw_result: WindowsResult<()>,
    end_draw_result: WindowsResult<()>,
    present: impl FnOnce() -> WindowsResult<()>,
) -> WindowsResult<()> {
    match (draw_result, end_draw_result) {
        (Ok(()), Ok(())) => present(),
        (Err(draw_error), Ok(())) => Err(draw_error),
        (Ok(()), Err(end_draw_error)) => Err(end_draw_error),
        (Err(draw_error), Err(end_draw_error)) => {
            if classify_hresult(end_draw_error.code()) == FailureClass::DeviceLost {
                Err(end_draw_error)
            } else {
                Err(draw_error)
            }
        }
    }
}

fn rollback_recovery_error(
    commit_error: &WindowsError,
    rollback_error: &WindowsError,
) -> WindowsError {
    WindowsError::new(
        D2DERR_RECREATE_TARGET,
        format!(
            "composition commit failed ({commit_error}) and restoring the prior visual content failed ({rollback_error}); recreate the target"
        ),
    )
}

fn replace_after_commit<T>(
    current: &mut T,
    replacement: T,
    commit: impl FnOnce(&T, &T) -> WindowsResult<()>,
) -> WindowsResult<()> {
    commit(current, &replacement)?;
    *current = replacement;
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FrameReadiness {
    presented: bool,
}

impl FrameReadiness {
    fn mark_presented(&mut self) {
        self.presented = true;
    }

    fn require_presented(self) -> WindowsResult<()> {
        if self.presented {
            Ok(())
        } else {
            Err(invalid_operation(
                "replacement swap chain has no complete presented frame",
            ))
        }
    }
}

struct SwapChainResources {
    swap_chain: IDXGISwapChain1,
    target_bitmap: ID2D1Bitmap1,
    _width: u32,
    _height: u32,
    present_count: u64,
    readiness: FrameReadiness,
}

impl SwapChainResources {
    unsafe fn new_unpresented(
        device: &DeviceResources,
        width: u32,
        height: u32,
    ) -> WindowsResult<Self> {
        let (width, height) = physical_size(width, height);
        let description = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };
        let swap_chain = device.graphics.dxgi_factory.CreateSwapChainForComposition(
            &device.graphics.d3d_device,
            &description,
            None::<&IDXGIOutput>,
        )?;
        let surface: IDXGISurface = swap_chain.GetBuffer(0)?;
        let bitmap_properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            ..Default::default()
        };
        let target_bitmap = device
            .graphics
            .d2d_context
            .CreateBitmapFromDxgiSurface(&surface, Some(&bitmap_properties))?;
        Ok(Self {
            swap_chain,
            target_bitmap,
            _width: width,
            _height: height,
            present_count: 0,
            readiness: FrameReadiness::default(),
        })
    }

    /// Draw one complete frame into the non-visible back buffer and publish it
    /// with exactly one Present1. A failed draw or EndDraw is never presented.
    unsafe fn present_complete_frame(
        &mut self,
        device: &DeviceResources,
        draw: impl FnOnce(&ID2D1DeviceContext) -> WindowsResult<()>,
    ) -> WindowsResult<()> {
        device.graphics.d2d_context.SetTarget(&self.target_bitmap);
        device.graphics.d2d_context.BeginDraw();
        device.graphics.d2d_context.Clear(Some(&D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));

        let draw_result = draw(&device.graphics.d2d_context);
        let end_draw_result = device.graphics.d2d_context.EndDraw(None, None);
        device.graphics.d2d_context.SetTarget(None::<&ID2D1Image>);

        let present_parameters = DXGI_PRESENT_PARAMETERS::default();
        publish_completed_frame(draw_result, end_draw_result, || {
            self.swap_chain
                .Present1(1, DXGI_PRESENT(0), &present_parameters)
                .ok()
        })?;
        self.present_count += 1;
        self.readiness.mark_presented();
        Ok(())
    }
}

struct SurfaceResources {
    target: IDCompositionTarget,
    visual: IDCompositionVisual,
    opacity_effect: IDCompositionEffectGroup,
    buffer: SwapChainResources,
    attached: bool,
}

impl SurfaceResources {
    unsafe fn new_detached(
        device: &DeviceResources,
        hwnd: HWND,
        width: u32,
        height: u32,
        opacity: f32,
    ) -> WindowsResult<Self> {
        let target = device.composition_device.CreateTargetForHwnd(hwnd, true)?;
        let visual = device.composition_device.CreateVisual()?;
        let opacity_effect = device.composition_device.CreateEffectGroup()?;
        opacity_effect.SetOpacity2(opacity.clamp(0.0, 1.0))?;
        visual.SetEffect(&opacity_effect)?;
        Ok(Self {
            target,
            visual,
            opacity_effect,
            buffer: SwapChainResources::new_unpresented(device, width, height)?,
            attached: false,
        })
    }

    unsafe fn stage_attach(&mut self) -> WindowsResult<()> {
        self.buffer.readiness.require_presented()?;
        self.visual.SetContent(&self.buffer.swap_chain)?;
        self.target.SetRoot(&self.visual)?;
        self.attached = true;
        Ok(())
    }

    unsafe fn attach_and_commit(&mut self, device: &DeviceResources) -> WindowsResult<()> {
        self.stage_attach()?;
        if let Err(error) = device.composition_device.Commit() {
            let _ = self.target.SetRoot(None::<&IDCompositionVisual>);
            self.attached = false;
            return Err(error);
        }
        Ok(())
    }

    /// Install a complete replacement only after its visual-tree transaction
    /// commits. On an ordinary failure, the old buffer and attachment state
    /// remain authoritative and can be retried.
    unsafe fn detach_and_commit(&self, device: &DeviceResources) -> WindowsResult<()> {
        self.target.SetRoot(None::<&IDCompositionVisual>)?;
        if let Err(commit_error) = device.composition_device.Commit() {
            if let Err(rollback_error) = self.target.SetRoot(&self.visual) {
                return Err(rollback_recovery_error(&commit_error, &rollback_error));
            }
            return Err(commit_error);
        }
        Ok(())
    }

    unsafe fn replace_buffer_and_commit(
        &mut self,
        device: &DeviceResources,
        replacement: SwapChainResources,
    ) -> WindowsResult<()> {
        let was_attached = self.attached;
        let visual = self.visual.clone();
        let target = self.target.clone();
        replace_after_commit(&mut self.buffer, replacement, |current, replacement| {
            replacement.readiness.require_presented()?;
            visual.SetContent(&replacement.swap_chain)?;
            if !was_attached {
                target.SetRoot(&visual)?;
            }
            if let Err(commit_error) = device.composition_device.Commit() {
                let rollback = if was_attached {
                    visual.SetContent(&current.swap_chain)
                } else {
                    target.SetRoot(None::<&IDCompositionVisual>)
                };
                if let Err(rollback_error) = rollback {
                    return Err(rollback_recovery_error(&commit_error, &rollback_error));
                }
                return Err(commit_error);
            }
            Ok(())
        })?;
        self.attached = true;
        Ok(())
    }
}

struct SurfaceRegistration {
    hwnd: HWND,
    width: u32,
    height: u32,
    opacity: f32,
    resources: Option<SurfaceResources>,
}

/// UI-thread-owned Direct2D/DirectComposition backend.
///
/// The `Rc` marker intentionally makes this type `!Send` and `!Sync`; HWND
/// lifecycle, rendering, and device recovery must remain on the overlay UI
/// thread. Timer and notification domain state stays outside this type.
pub(in crate::overlay) struct Compositor {
    text: TextResources,
    owner_thread_id: u32,
    device: Option<DeviceResources>,
    surfaces: HashMap<isize, SurfaceRegistration>,
    composition_dirty: bool,
    recovery_redraws: HashSet<isize>,
    device_generation: u64,
    _ui_thread_only: PhantomData<Rc<()>>,
}

impl Compositor {
    pub(in crate::overlay) unsafe fn new() -> WindowsResult<Self> {
        let text = TextResources::new()?;
        let mut compositor = Self {
            text,
            owner_thread_id: GetCurrentThreadId(),
            device: None,
            surfaces: HashMap::new(),
            composition_dirty: false,
            recovery_redraws: HashSet::new(),
            device_generation: 0,
            _ui_thread_only: PhantomData,
        };
        // DirectWrite remains usable when hardware composition is temporarily
        // unavailable. Surface registration will retain its HWND metadata and
        // retry full device creation without falling back to GDI or software.
        let _ = compositor.recover_device();
        Ok(compositor)
    }

    #[cfg(test)]
    unsafe fn measurement_only() -> WindowsResult<Self> {
        Ok(Self {
            text: TextResources::new()?,
            owner_thread_id: GetCurrentThreadId(),
            device: None,
            surfaces: HashMap::new(),
            composition_dirty: false,
            recovery_redraws: HashSet::new(),
            device_generation: 0,
            _ui_thread_only: PhantomData,
        })
    }

    fn ensure_owner_thread(&self) -> WindowsResult<()> {
        check_thread(self.owner_thread_id, unsafe { GetCurrentThreadId() })
    }

    pub(in crate::overlay) fn has_surface(&self, hwnd: HWND) -> WindowsResult<bool> {
        self.ensure_owner_thread()?;
        Ok(self.surfaces.contains_key(&surface_key(hwnd)))
    }

    pub(in crate::overlay) fn surface_is_attached(&self, hwnd: HWND) -> WindowsResult<bool> {
        self.ensure_owner_thread()?;
        Ok(self
            .surfaces
            .get(&surface_key(hwnd))
            .and_then(|registration| registration.resources.as_ref())
            .is_some_and(|resources| resources.attached))
    }

    /// Retry hardware/device resources after a transient initialization or
    /// device-loss failure. Registrations remain detached until role redraws.
    pub(in crate::overlay) unsafe fn recover_if_needed(&mut self) -> WindowsResult<()> {
        self.ensure_owner_thread()?;
        if self.device.is_none()
            || self
                .surfaces
                .values()
                .any(|registration| registration.resources.is_none())
        {
            self.recover_device()?;
        }
        Ok(())
    }

    /// HWNDs whose recovered resources remain detached until their current
    /// role snapshot has produced one complete frame.
    pub(in crate::overlay) fn recovery_redraw_hwnds(&self) -> WindowsResult<Vec<HWND>> {
        self.ensure_owner_thread()?;
        Ok(self
            .recovery_redraws
            .iter()
            .filter_map(|key| self.surfaces.get(key).map(|surface| surface.hwnd))
            .collect())
    }

    pub(in crate::overlay) unsafe fn measure_text(
        &self,
        text: &str,
        spec: &FontSpec,
        height: i32,
    ) -> WindowsResult<i32> {
        self.ensure_owner_thread()?;
        self.text.measure_text(text, spec, height)
    }

    pub(in crate::overlay) unsafe fn measure_label_width(
        &self,
        model: &LabelModel<'_>,
        style: LabelStyle,
        theme: &LabelTheme,
        max_width: i32,
    ) -> WindowsResult<i32> {
        let text_width =
            self.measure_text(model.text, &theme.name_font, style.name_font_height(theme))?;
        Ok(required_width(
            text_width,
            style,
            theme,
            model.class.is_some(),
            max_width,
        ))
    }

    pub(super) unsafe fn stonemite_icon_bitmap(&mut self) -> WindowsResult<ID2D1Bitmap1> {
        self.ensure_owner_thread()?;
        if self.device.is_none() {
            self.recover_device()?;
        }
        let result = self
            .device
            .as_mut()
            .ok_or_else(|| missing_resource("DirectComposition device"))?
            .graphics
            .stonemite_icon_bitmap();
        match result {
            Ok(bitmap) => Ok(bitmap),
            Err(error) => {
                self.log_failure("Stonemite icon upload failed", &error);
                if failure_action(&error) == FailureAction::RecoverDevice {
                    self.recover_device()?;
                }
                Err(error)
            }
        }
    }

    /// Return a device-generation-owned class icon bitmap to concrete scene renderers.
    pub(super) unsafe fn class_icon_bitmap(
        &mut self,
        class_abbreviation: &str,
    ) -> WindowsResult<Option<ID2D1Bitmap1>> {
        self.ensure_owner_thread()?;
        if self.device.is_none() {
            self.recover_device()?;
        }
        let result = self
            .device
            .as_mut()
            .ok_or_else(|| missing_resource("DirectComposition device"))?
            .graphics
            .class_icon_bitmap(class_abbreviation);
        match result {
            Ok(bitmap) => Ok(bitmap),
            Err(error) => {
                self.log_failure("class icon upload failed", &error);
                if failure_action(&error) == FailureAction::RecoverDevice {
                    self.recover_device()?;
                }
                Err(error)
            }
        }
    }

    pub(in crate::overlay) unsafe fn render_active_label(
        &mut self,
        hwnd: HWND,
        scene: &ActiveLabelScene<'_>,
    ) -> WindowsResult<()> {
        let icon = match scene.label.model.class {
            Some(class) => self.class_icon_bitmap(class)?,
            None => None,
        };
        self.set_surface_opacity(hwnd, scene.surface_opacity())?;
        let text = self.text.clone();
        self.replace_surface_frame(
            hwnd,
            scene.canvas.width().max(1) as u32,
            scene.canvas.height().max(1) as u32,
            |context| super::scene_d2d::draw_active_label(context, &text, icon.as_ref(), scene),
        )?;
        self.flush()
    }

    pub(in crate::overlay) unsafe fn render_pip_scene(
        &mut self,
        hwnd: HWND,
        scene: &PipScene<'_>,
    ) -> WindowsResult<()> {
        let label_width = self.measure_label_width(
            &scene.label.model,
            scene.label.style,
            scene.label.theme,
            scene.canvas.width().max(1),
        )?;
        let preview_width = if let Some(notification) = &scene.notification {
            self.measure_text(
                &notification.text,
                &UiTextRole::NotificationPreview.font(),
                UiTextRole::NotificationPreview.height(scene.scale, 0),
            )?
        } else {
            0
        };
        let layout = scene.layout(label_width, preview_width);
        let icon = match scene.label.model.class {
            Some(class) => self.class_icon_bitmap(class)?,
            None => None,
        };
        // PiP content applies its alpha once inside a Direct2D layer while
        // chrome remains opaque, so visual opacity must never multiply it.
        self.set_surface_opacity(hwnd, 1.0)?;
        let text = self.text.clone();
        self.replace_surface_frame(
            hwnd,
            scene.canvas.width().max(1) as u32,
            scene.canvas.height().max(1) as u32,
            |context| {
                super::scene_d2d::draw_pip_scene(context, &text, icon.as_ref(), scene, &layout)
            },
        )?;
        self.flush()
    }

    pub(in crate::overlay) unsafe fn render_stonemite_button(
        &mut self,
        hwnd: HWND,
        scene: &StonemiteButtonScene,
    ) -> WindowsResult<()> {
        let icon = self.stonemite_icon_bitmap()?;
        self.set_surface_opacity(hwnd, 1.0)?;
        self.replace_surface_frame(
            hwnd,
            scene.bounds.width().max(1) as u32,
            scene.bounds.height().max(1) as u32,
            |context| super::scene_d2d::draw_stonemite_button(context, &icon, scene),
        )?;
        self.flush()
    }

    pub(in crate::overlay) unsafe fn render_status_banner(
        &mut self,
        hwnd: HWND,
        scene: &StatusBannerScene<'_>,
    ) -> WindowsResult<()> {
        self.set_surface_opacity(hwnd, scene.surface_opacity())?;
        let text = self.text.clone();
        self.replace_surface_frame(
            hwnd,
            scene.bounds.width().max(1) as u32,
            scene.bounds.height().max(1) as u32,
            |context| super::scene_d2d::draw_status_banner(context, &text, scene),
        )?;
        self.flush()
    }

    pub(in crate::overlay) unsafe fn render_toast(
        &mut self,
        hwnd: HWND,
        scene: &ToastScene<'_>,
    ) -> WindowsResult<()> {
        self.set_surface_opacity(hwnd, scene.surface_opacity())?;
        let text = self.text.clone();
        self.replace_surface_frame(
            hwnd,
            scene.bounds.width().max(1) as u32,
            scene.bounds.height().max(1) as u32,
            |context| super::scene_d2d::draw_toast(context, &text, scene),
        )?;
        self.flush()
    }

    /// Register detached size/opacity resources only. The caller must render a
    /// complete role scene, which presents, attaches, and commits before the
    /// HWND is shown. No transparent frame is ever committed as final content.
    pub(in crate::overlay) unsafe fn register_surface(
        &mut self,
        hwnd: HWND,
        width: u32,
        height: u32,
        opacity: f32,
    ) -> WindowsResult<()> {
        self.ensure_owner_thread()?;
        let key = surface_key(hwnd);
        let (width, height) = physical_size(width, height);
        let opacity = opacity.clamp(0.0, 1.0);

        if let Some(existing) = self.surfaces.get(&key) {
            if existing.width != width || existing.height != height {
                return Err(invalid_operation(
                    "registered surface size changes require a complete role-renderer frame",
                ));
            }
            if existing.resources.is_some() {
                if (existing.opacity - opacity).abs() > f32::EPSILON {
                    self.set_surface_opacity(hwnd, opacity)?;
                }
                return Ok(());
            }
            // A pending registration left by device loss is retried below as a
            // fresh transaction. Ordinary creation failures never leave one.
            self.surfaces.remove(&key);
        }

        if self.device.is_none() {
            self.surfaces.insert(
                key,
                SurfaceRegistration {
                    hwnd,
                    width,
                    height,
                    opacity,
                    resources: None,
                },
            );
            return match self.recover_device() {
                Ok(()) => Ok(()),
                Err(error) => {
                    if failure_action(&error) == FailureAction::PreserveGraph {
                        self.surfaces.remove(&key);
                    }
                    Err(error)
                }
            };
        }

        let device = self
            .device
            .as_ref()
            .ok_or_else(|| missing_resource("DirectComposition device"))?;
        let result = insert_after_prepare(&mut self.surfaces, key, || {
            let resources = SurfaceResources::new_detached(device, hwnd, width, height, opacity)?;
            Ok(SurfaceRegistration {
                hwnd,
                width,
                height,
                opacity,
                resources: Some(resources),
            })
        });
        match result {
            Ok(()) => {
                // Initial registration uses the same detached complete-scene
                // handshake as device recovery.
                self.recovery_redraws.insert(key);
                Ok(())
            }
            Err(error) => {
                self.log_failure("surface registration failed", &error);
                if failure_action(&error) == FailureAction::RecoverDevice {
                    self.surfaces.insert(
                        key,
                        SurfaceRegistration {
                            hwnd,
                            width,
                            height,
                            opacity,
                            resources: None,
                        },
                    );
                    let recovery = self.recover_device();
                    recovered_operation_result(error, recovery)
                } else {
                    Err(error)
                }
            }
        }
    }

    /// Replace a size-dependent swap chain only after a concrete role renderer
    /// has drawn and presented the complete resized frame. This callback is
    /// render-module-internal and is never exposed to overlay domain code.
    pub(super) unsafe fn replace_surface_frame(
        &mut self,
        hwnd: HWND,
        width: u32,
        height: u32,
        draw: impl FnOnce(&ID2D1DeviceContext) -> WindowsResult<()>,
    ) -> WindowsResult<()> {
        self.ensure_owner_thread()?;
        let key = surface_key(hwnd);
        let (width, height) = physical_size(width, height);
        let registration = self
            .surfaces
            .get(&key)
            .ok_or_else(|| missing_resource("surface registration"))?;
        if registration.width == width && registration.height == height {
            let result = (|| -> WindowsResult<()> {
                let device = self
                    .device
                    .as_ref()
                    .ok_or_else(|| missing_resource("DirectComposition device"))?;
                let resources = self
                    .surfaces
                    .get_mut(&key)
                    .and_then(|registration| registration.resources.as_mut())
                    .ok_or_else(|| missing_resource("composition surface"))?;
                resources.buffer.present_complete_frame(device, draw)?;
                if !resources.attached {
                    resources.attach_and_commit(device)?;
                }
                Ok(())
            })();
            return match result {
                Ok(()) => {
                    self.recovery_redraws.remove(&key);
                    Ok(())
                }
                Err(error) => {
                    self.log_failure("same-size frame presentation failed", &error);
                    if failure_action(&error) == FailureAction::RecoverDevice {
                        self.recover_device()?;
                    }
                    Err(error)
                }
            };
        }
        if registration.resources.is_none() {
            return Err(missing_resource("composition surface"));
        }
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| missing_resource("DirectComposition device"))?;

        let result = (|| -> WindowsResult<()> {
            let mut replacement = SwapChainResources::new_unpresented(device, width, height)?;
            replacement.present_complete_frame(device, draw)?;
            replacement.readiness.require_presented()?;

            let resources = self
                .surfaces
                .get_mut(&key)
                .and_then(|registration| registration.resources.as_mut())
                .ok_or_else(|| missing_resource("composition surface"))?;
            resources.replace_buffer_and_commit(device, replacement)?;
            let registration = self
                .surfaces
                .get_mut(&key)
                .expect("registration survives replacement commit");
            registration.width = width;
            registration.height = height;
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.recovery_redraws.remove(&key);
                Ok(())
            }
            Err(error) => {
                self.log_failure("surface replacement failed", &error);
                if failure_action(&error) == FailureAction::RecoverDevice {
                    // Authoritative dimensions still describe the old complete
                    // frame, so recovery recreates that frame size. The caller
                    // receives the error and can render/retry the resize.
                    self.recover_device()?;
                }
                Err(error)
            }
        }
    }

    pub(in crate::overlay) unsafe fn set_surface_opacity(
        &mut self,
        hwnd: HWND,
        opacity: f32,
    ) -> WindowsResult<()> {
        self.ensure_owner_thread()?;
        let key = surface_key(hwnd);
        let registration = self
            .surfaces
            .get(&key)
            .ok_or_else(|| missing_resource("surface registration"))?;
        let Some(opacity) = changed_opacity(registration.opacity, opacity) else {
            return Ok(());
        };
        let has_attached_resources = registration
            .resources
            .as_ref()
            .is_some_and(|resources| resources.attached);
        let result = if let Some(resources) = registration.resources.as_ref() {
            resources.opacity_effect.SetOpacity2(opacity)
        } else {
            Ok(())
        };
        match result {
            Ok(()) => {
                self.surfaces
                    .get_mut(&key)
                    .expect("registration survives opacity update")
                    .opacity = opacity;
                if has_attached_resources {
                    self.composition_dirty = true;
                }
                Ok(())
            }
            Err(error) => {
                self.log_failure("surface opacity update failed", &error);
                if failure_action(&error) == FailureAction::RecoverDevice {
                    self.surfaces
                        .get_mut(&key)
                        .expect("registration survives device loss")
                        .opacity = opacity;
                    let recovery = self.recover_device();
                    recovered_operation_result(error, recovery)
                } else {
                    Err(error)
                }
            }
        }
    }

    /// Commit pending visual-tree or opacity property changes once. Swap-chain
    /// frame presentation does not require a composition commit.
    pub(in crate::overlay) unsafe fn flush(&mut self) -> WindowsResult<()> {
        self.ensure_owner_thread()?;
        if !self.composition_dirty {
            return Ok(());
        }
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| missing_resource("DirectComposition device"))?;
        match device.composition_device.Commit() {
            Ok(()) => {
                self.composition_dirty = false;
                Ok(())
            }
            Err(error) => self.finish_or_recover("composition commit failed", Err(error)),
        }
    }

    pub(in crate::overlay) unsafe fn unregister_surface(
        &mut self,
        hwnd: HWND,
    ) -> WindowsResult<()> {
        self.ensure_owner_thread()?;
        let key = surface_key(hwnd);
        let device = self.device.as_ref();
        let result = remove_after_commit(&mut self.surfaces, &key, |registration| {
            match (device, registration.resources.as_ref()) {
                (Some(device), Some(resources)) if resources.attached => {
                    resources.detach_and_commit(device)
                }
                _ => Ok(()),
            }
        });
        match result {
            Ok(_) => {
                self.recovery_redraws.remove(&key);
                Ok(())
            }
            Err(error) => {
                self.log_failure("surface teardown failed", &error);
                if failure_action(&error) == FailureAction::RecoverDevice {
                    // The old device graph is no longer authoritative. Rebuild
                    // the retained registrations, then drop this detached one.
                    self.recover_device()?;
                    self.surfaces.remove(&key);
                    self.recovery_redraws.remove(&key);
                    Ok(())
                } else {
                    Err(error)
                }
            }
        }
    }

    pub(in crate::overlay) unsafe fn shutdown(&mut self) -> WindowsResult<()> {
        self.ensure_owner_thread()?;
        let mut first_error = None;
        for registration in self.surfaces.values_mut() {
            if let Some(resources) = registration.resources.take() {
                if let Err(error) = resources.target.SetRoot(None::<&IDCompositionVisual>) {
                    first_error.get_or_insert(error);
                }
                self.composition_dirty = true;
            }
        }
        self.surfaces.clear();
        self.recovery_redraws.clear();
        if self.composition_dirty {
            if let Some(device) = self.device.as_ref() {
                if let Err(error) = device.composition_device.Commit() {
                    first_error.get_or_insert(error);
                }
            }
        }
        self.composition_dirty = false;
        self.device = None;
        if let Some(error) = first_error {
            self.log_failure("compositor shutdown failed", &error);
            Err(error)
        } else {
            Ok(())
        }
    }

    /// Drop all device-dependent objects while retaining HWND registrations,
    /// then recreate the complete device stack and every registered surface.
    pub(in crate::overlay) unsafe fn recover_device(&mut self) -> WindowsResult<()> {
        self.ensure_owner_thread()?;
        if let Some(device) = self.device.as_ref() {
            device.log_removed_reason("device recovery requested");
        }
        for registration in self.surfaces.values_mut() {
            registration.resources = None;
        }
        self.device = None;
        self.composition_dirty = false;
        self.recovery_redraws.clear();

        let device = match DeviceResources::new() {
            Ok(device) => device,
            Err(error) => {
                self.log_failure("device recreation failed", &error);
                return Err(error);
            }
        };
        let mut recreated = HashMap::with_capacity(self.surfaces.len());
        for (&key, registration) in &self.surfaces {
            match SurfaceResources::new_detached(
                &device,
                registration.hwnd,
                registration.width,
                registration.height,
                registration.opacity,
            ) {
                Ok(resources) => {
                    recreated.insert(key, resources);
                }
                Err(error) => {
                    self.log_failure("surface recreation failed", &error);
                    return Err(error);
                }
            }
        }
        // Do not attach or commit transparent recovery buffers. The overlay
        // receives the HWND handshake below and supplies current domain scenes.
        self.device = Some(device);
        for (key, resources) in recreated {
            self.surfaces
                .get_mut(&key)
                .expect("registration survives device recovery")
                .resources = Some(resources);
            self.recovery_redraws.insert(key);
        }
        self.device_generation = self.device_generation.wrapping_add(1);
        Ok(())
    }

    unsafe fn finish_or_recover(
        &mut self,
        context: &str,
        result: WindowsResult<()>,
    ) -> WindowsResult<()> {
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                self.log_failure(context, &error);
                match failure_action(&error) {
                    FailureAction::RecoverDevice => {
                        let recovery = self.recover_device();
                        recovered_operation_result(error, recovery)
                    }
                    FailureAction::PreserveGraph => Err(error),
                }
            }
        }
    }

    fn log_failure(&self, context: &str, error: &WindowsError) {
        crate::diagnostics::debug_log(&format!("DirectComposition {context}: {error}"));
        if classify_hresult(error.code()) == FailureClass::DeviceLost {
            if let Some(device) = self.device.as_ref() {
                device.log_removed_reason(context);
            }
        }
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        unsafe {
            if let Err(error) = self.shutdown() {
                self.log_failure("drop skipped or failed teardown", &error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use windows::core::w;
    use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_POPUP,
    };

    struct TestComApartment;

    impl TestComApartment {
        unsafe fn new() -> Self {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .expect("initialize test COM apartment");
            Self
        }
    }

    impl Drop for TestComApartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }

    struct TestWindow(HWND);

    impl TestWindow {
        unsafe fn new(width: i32, height: i32) -> Self {
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!("Stonemite compositor test"),
                WS_POPUP,
                0,
                0,
                width,
                height,
                None,
                None,
                None,
                None,
            )
            .expect("create hidden composition test HWND");
            Self(hwnd)
        }
    }

    impl Drop for TestWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }

    fn sample_model() -> LabelModel<'static> {
        LabelModel {
            text: "Bilka",
            class: None,
            number: 1,
            background: super::super::super::labels::Color {
                red: 74,
                green: 134,
                blue: 212,
            },
            badge_background: super::super::super::labels::Color {
                red: 48,
                green: 104,
                blue: 176,
            },
        }
    }

    #[test]
    fn directwrite_measurement_does_not_require_gpu_resources() {
        unsafe {
            let compositor = Compositor::measurement_only().expect("create DirectWrite factory");
            assert!(compositor.device.is_none());
            let width = compositor
                .measure_label_width(
                    &sample_model(),
                    LabelStyle::new(1.0, 48),
                    &LabelTheme::default(),
                    500,
                )
                .expect("measure label with DirectWrite");
            assert!(width > 64);
        }
    }

    #[test]
    fn creates_hardware_device_and_shared_direct2d_context() {
        unsafe {
            let _com = TestComApartment::new();
            let mut resources = GraphicsResources::new().expect("create hardware graphics device");
            assert!(!Interface::as_raw(&resources.d3d_device).is_null());
            assert!(!Interface::as_raw(&resources.dxgi_device).is_null());
            assert!(!Interface::as_raw(&resources._d2d_device).is_null());
            assert!(!Interface::as_raw(&resources.d2d_context).is_null());

            let first = resources
                .class_icon_bitmap("WAR")
                .expect("upload class icon")
                .expect("warrior icon");
            let second = resources
                .class_icon_bitmap("WAR")
                .expect("reuse class icon")
                .expect("warrior icon");
            assert_eq!(Interface::as_raw(&first), Interface::as_raw(&second));
            let (_, icon) = crate::class_icons::class_icon("WAR").expect("warrior CPU icon");
            let pixel_size = first.GetPixelSize();
            assert_eq!(
                (pixel_size.width, pixel_size.height),
                (icon.width, icon.height)
            );
            assert_eq!(resources.class_icon_bitmaps.len(), 1);
            assert!(resources
                .class_icon_bitmap("UNKNOWN")
                .expect("unknown icon lookup")
                .is_none());
        }
    }

    #[test]
    fn creates_every_role_specific_scene_on_a_hardware_direct2d_target() {
        use super::super::super::notifications::{Kind, Notification};
        use super::super::super::scenes::{
            ActiveLabelScene, LabelScene, PipInteractionScene, PipScene, StatusBannerScene,
            TimerScene, ToastScene, UiTextRole,
        };
        use windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED;

        unsafe {
            let _com = TestComApartment::new();
            let mut graphics = GraphicsResources::new().expect("create hardware graphics device");
            let text = TextResources::new().expect("create DirectWrite resources");
            let properties = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                ..Default::default()
            };
            let target = graphics
                .d2d_context
                .CreateBitmap(
                    D2D_SIZE_U {
                        width: 480,
                        height: 320,
                    },
                    None,
                    0,
                    &properties,
                )
                .expect("create offscreen scene target");
            let icon = graphics
                .class_icon_bitmap("WAR")
                .expect("upload class icon")
                .expect("warrior icon");
            graphics.d2d_context.SetTarget(&target);
            graphics.d2d_context.BeginDraw();
            graphics.d2d_context.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));

            let theme = LabelTheme::with_name_font("Wingdings".to_owned(), 100, 700);
            let label = LabelScene {
                model: LabelModel {
                    text: "Bilka",
                    class: Some("WAR"),
                    number: 1,
                    background: super::super::super::labels::Color::from_colorref(0x00D4864A),
                    badge_background: super::super::super::labels::Color::from_colorref(0x00B06830),
                },
                style: LabelStyle::new(1.0, 48),
                theme: &theme,
                alpha: 204,
            };
            let timer = TimerScene {
                label: "Mez",
                remaining_text: "9.9s",
                progress: 0.25,
            };
            let active = ActiveLabelScene {
                canvas: super::super::super::labels::Rect::new(0, 0, 240, 94),
                label,
                timer: Some(timer),
            };
            super::super::scene_d2d::draw_active_label(
                &graphics.d2d_context,
                &text,
                Some(&icon),
                &active,
            )
            .expect("draw active label scene");

            let notification = Notification::push(
                None,
                Kind::GroupInvite,
                "Honka invited you to a group".to_owned(),
                0x0060B06A,
                1_000,
                true,
            )
            .visual_snapshot(1_001, true);
            let pip = PipScene {
                canvas: super::super::super::labels::Rect::new(0, 0, 480, 320),
                border_width: 3,
                scale: 1.0,
                label,
                timer: Some(timer),
                notification: Some(notification.clone()),
                interaction: PipInteractionScene {
                    hovered: true,
                    ..Default::default()
                },
            };
            let preview_width = text
                .measure_text(
                    &notification.text,
                    &UiTextRole::NotificationPreview.font(),
                    UiTextRole::NotificationPreview.height(1.0, 0),
                )
                .expect("measure notification text");
            let pip_layout = pip.layout(240, preview_width);
            super::super::scene_d2d::draw_pip_scene(
                &graphics.d2d_context,
                &text,
                Some(&icon),
                &pip,
                &pip_layout,
            )
            .expect("draw complete PiP scene");
            assert_eq!(
                graphics.d2d_context.GetAntialiasMode(),
                D2D1_ANTIALIAS_MODE_ALIASED,
                "moving notification trace must restore crisp scene antialiasing",
            );

            super::super::scene_d2d::draw_status_banner(
                &graphics.d2d_context,
                &text,
                &StatusBannerScene {
                    bounds: super::super::super::labels::Rect::new(0, 0, 260, 48),
                    text: "Broadcasting · Mouse Clutch",
                    background: super::super::super::labels::Color::from_colorref(0x002030CC),
                    alpha: 204,
                    scale: 1.0,
                    logical_label_height: 48,
                },
            )
            .expect("draw status banner");
            super::super::scene_d2d::draw_toast(
                &graphics.d2d_context,
                &text,
                &ToastScene {
                    bounds: super::super::super::labels::Rect::new(0, 0, 320, 64),
                    text: "Could not accept invitation",
                    background: super::super::super::labels::Color::from_colorref(0x00403020),
                    alpha: 220,
                    scale: 1.0,
                    logical_height: 64,
                },
            )
            .expect("draw toast scene");

            graphics
                .d2d_context
                .EndDraw(None, None)
                .expect("finish offscreen scene frame");
            graphics.d2d_context.SetTarget(None::<&ID2D1Image>);
        }
    }

    #[test]
    #[ignore = "requires an interactive DWM desktop; SSH window stations reject DCompositionCreateDevice"]
    fn composition_surface_lifecycle_on_interactive_desktop() {
        unsafe {
            let _com = TestComApartment::new();
            let first = TestWindow::new(128, 80);
            let second = TestWindow::new(72, 44);
            let mut compositor = Compositor::new().expect("create compositor");

            compositor
                .register_surface(first.0, 96, 64, 1.0)
                .expect("register first detached surface");
            let initial = compositor
                .surfaces
                .get(&surface_key(first.0))
                .and_then(|registration| registration.resources.as_ref())
                .expect("initial composition resources");
            assert_eq!((initial.buffer._width, initial.buffer._height), (96, 64));
            assert_eq!(initial.buffer.present_count, 0);
            assert!(!initial.buffer.readiness.presented);
            assert!(!initial.attached);
            assert_eq!(compositor.recovery_redraw_hwnds().unwrap(), vec![first.0]);
            assert!(!Interface::as_raw(&initial.target).is_null());
            assert!(!Interface::as_raw(&initial.visual).is_null());
            assert!(!Interface::as_raw(&initial.buffer.swap_chain).is_null());
            assert!(!Interface::as_raw(&initial.buffer.target_bitmap).is_null());

            compositor
                .replace_surface_frame(first.0, 96, 64, |context| {
                    let brush = context.CreateSolidColorBrush(
                        &D2D1_COLOR_F {
                            r: 0.1,
                            g: 0.3,
                            b: 0.7,
                            a: 1.0,
                        },
                        None,
                    )?;
                    context.FillRectangle(
                        &D2D_RECT_F {
                            left: 0.0,
                            top: 0.0,
                            right: 96.0,
                            bottom: 64.0,
                        },
                        &brush,
                    );
                    Ok(())
                })
                .expect("first complete scene presents before attachment");
            let attached = compositor
                .surfaces
                .get(&surface_key(first.0))
                .and_then(|registration| registration.resources.as_ref())
                .expect("attached composition resources");
            assert_eq!(attached.buffer.present_count, 1);
            assert!(attached.attached);
            assert!(compositor.recovery_redraw_hwnds().unwrap().is_empty());

            compositor
                .set_surface_opacity(first.0, 0.5)
                .expect("queue opacity");
            assert!(compositor.composition_dirty);
            compositor.flush().expect("commit opacity batch");
            assert!(!compositor.composition_dirty);

            compositor
                .replace_surface_frame(first.0, 96, 64, |context| {
                    let brush = context.CreateSolidColorBrush(
                        &D2D1_COLOR_F {
                            r: 0.4,
                            g: 0.2,
                            b: 0.8,
                            a: 1.0,
                        },
                        None,
                    )?;
                    context.FillRectangle(
                        &D2D_RECT_F {
                            left: 0.0,
                            top: 0.0,
                            right: 96.0,
                            bottom: 64.0,
                        },
                        &brush,
                    );
                    Ok(())
                })
                .expect("same-size complete frame presentation");
            assert_eq!(
                compositor
                    .surfaces
                    .get(&surface_key(first.0))
                    .and_then(|surface| surface.resources.as_ref())
                    .expect("same-size resources")
                    .buffer
                    .present_count,
                2
            );

            compositor
                .replace_surface_frame(first.0, 128, 80, |context| {
                    let brush = context.CreateSolidColorBrush(
                        &D2D1_COLOR_F {
                            r: 0.15,
                            g: 0.45,
                            b: 0.85,
                            a: 1.0,
                        },
                        None,
                    )?;
                    context.FillRectangle(
                        &D2D_RECT_F {
                            left: 0.0,
                            top: 0.0,
                            right: 128.0,
                            bottom: 80.0,
                        },
                        &brush,
                    );
                    Ok(())
                })
                .expect("present nontransparent replacement before atomic content switch");
            let resized = compositor
                .surfaces
                .get(&surface_key(first.0))
                .and_then(|registration| registration.resources.as_ref())
                .expect("replacement composition resources");
            assert_eq!((resized.buffer._width, resized.buffer._height), (128, 80));
            assert_eq!(resized.buffer.present_count, 1);
            assert_eq!(resized.buffer.readiness, FrameReadiness { presented: true });
            let descriptor = resized
                .buffer
                .swap_chain
                .GetDesc1()
                .expect("read composition swap-chain descriptor");
            assert_eq!(descriptor.Format, DXGI_FORMAT_B8G8R8A8_UNORM);
            assert_eq!(descriptor.AlphaMode, DXGI_ALPHA_MODE_PREMULTIPLIED);
            assert_eq!(descriptor.BufferCount, 2);
            assert_eq!(descriptor.Scaling, DXGI_SCALING_STRETCH);
            assert_eq!(descriptor.SwapEffect, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL);

            compositor
                .register_surface(second.0, 72, 44, 0.75)
                .expect("register second surface");
            let previous_generation = compositor.device_generation;
            compositor
                .recover_device()
                .expect("recreate device and every registered surface");
            assert_eq!(compositor.device_generation, previous_generation + 1);
            assert_eq!(compositor.surfaces.len(), 2);
            assert_eq!(compositor.recovery_redraw_hwnds().unwrap().len(), 2);
            assert!(compositor.surfaces.values().all(|registration| {
                registration.resources.as_ref().is_some_and(|resources| {
                    resources.buffer.present_count == 0
                        && !resources.buffer.readiness.presented
                        && !resources.attached
                })
            }));

            for (hwnd, width, height) in [(first.0, 128, 80), (second.0, 72, 44)] {
                compositor
                    .replace_surface_frame(hwnd, width, height, |context| {
                        let brush = context.CreateSolidColorBrush(
                            &D2D1_COLOR_F {
                                r: 0.2,
                                g: 0.6,
                                b: 0.3,
                                a: 1.0,
                            },
                            None,
                        )?;
                        context.FillRectangle(
                            &D2D_RECT_F {
                                left: 0.0,
                                top: 0.0,
                                right: width as f32,
                                bottom: height as f32,
                            },
                            &brush,
                        );
                        Ok(())
                    })
                    .expect("redraw recovered surface before attachment");
            }
            assert!(compositor.recovery_redraw_hwnds().unwrap().is_empty());
            assert!(compositor.surfaces.values().all(|registration| {
                registration.resources.as_ref().is_some_and(|resources| {
                    resources.buffer.present_count == 1 && resources.attached
                })
            }));

            compositor
                .unregister_surface(first.0)
                .expect("first teardown");
            compositor
                .unregister_surface(first.0)
                .expect("idempotent teardown");
            compositor.shutdown().expect("first shutdown");
            compositor.shutdown().expect("idempotent shutdown");
        }
    }

    #[test]
    fn failed_registration_prepare_does_not_mutate_healthy_registrations() {
        let mut registrations = HashMap::from([(1_u32, "healthy")]);
        let result = insert_after_prepare(&mut registrations, 2, || {
            Err::<&str, _>(WindowsError::new(
                E_FAIL,
                "ordinary target creation failure",
            ))
        });

        assert!(result.is_err());
        assert_eq!(registrations, HashMap::from([(1_u32, "healthy")]));
    }

    #[test]
    fn failed_unregister_commit_preserves_authoritative_registration() {
        let mut registrations = HashMap::from([(7_u32, "attached resources")]);
        let result = remove_after_commit(&mut registrations, &7, |_| {
            Err(WindowsError::new(E_FAIL, "detach commit failed"))
        });
        assert!(result.is_err());
        assert_eq!(registrations.get(&7), Some(&"attached resources"));

        let removed = remove_after_commit(&mut registrations, &7, |_| Ok(()))
            .expect("successful detach transaction");
        assert_eq!(removed, Some("attached resources"));
        assert!(!registrations.contains_key(&7));
    }

    #[test]
    fn failed_draw_or_end_draw_never_reaches_present() {
        let present_calls = Cell::new(0);
        let draw_failure = publish_completed_frame(
            Err(WindowsError::new(E_FAIL, "draw failed")),
            Ok(()),
            || {
                present_calls.set(present_calls.get() + 1);
                Ok(())
            },
        );
        assert!(draw_failure.is_err());
        assert_eq!(present_calls.get(), 0);

        let end_draw_failure = publish_completed_frame(
            Ok(()),
            Err(WindowsError::new(E_FAIL, "EndDraw failed")),
            || {
                present_calls.set(present_calls.get() + 1);
                Ok(())
            },
        );
        assert!(end_draw_failure.is_err());
        assert_eq!(present_calls.get(), 0);

        let combined_failure = publish_completed_frame(
            Err(WindowsError::new(E_FAIL, "draw failed")),
            Err(WindowsError::from(D2DERR_RECREATE_TARGET)),
            || {
                present_calls.set(present_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("device loss from EndDraw must not be masked by the draw error");
        assert_eq!(combined_failure.code(), D2DERR_RECREATE_TARGET);
        assert_eq!(present_calls.get(), 0);
    }

    #[test]
    fn detached_resize_commit_failure_preserves_buffer_and_dimensions() {
        let mut readiness = FrameReadiness::default();
        assert!(readiness.require_presented().is_err());
        readiness.mark_presented();
        assert!(readiness.require_presented().is_ok());

        let mut authoritative = (64_u32, 48_u32, "old resources");
        let result = replace_after_commit(
            &mut authoritative,
            (128, 80, "replacement resources"),
            |_, _| Err(WindowsError::new(E_FAIL, "commit failed")),
        );
        assert!(result.is_err());
        assert_eq!(authoritative, (64, 48, "old resources"));
    }

    #[test]
    fn ui_thread_check_rejects_a_different_thread_id() {
        assert!(check_thread(42, 42).is_ok());
        let error = check_thread(42, 99).expect_err("wrong thread must fail closed");
        assert_eq!(error.code(), RPC_E_WRONG_THREAD);
    }

    #[test]
    fn classifies_only_target_and_device_loss_hresult_values_as_device_loss() {
        assert_eq!(
            classify_hresult(D2DERR_RECREATE_TARGET),
            FailureClass::DeviceLost
        );
        assert_eq!(
            classify_hresult(DXGI_ERROR_DEVICE_HUNG),
            FailureClass::DeviceLost
        );
        assert_eq!(
            classify_hresult(DXGI_ERROR_DEVICE_REMOVED),
            FailureClass::DeviceLost
        );
        assert_eq!(
            classify_hresult(DXGI_ERROR_DEVICE_RESET),
            FailureClass::DeviceLost
        );
        assert_eq!(
            classify_hresult(DXGI_ERROR_DRIVER_INTERNAL_ERROR),
            FailureClass::DeviceLost
        );
        assert_eq!(classify_hresult(E_FAIL), FailureClass::Other);
        assert_eq!(
            failure_action(&WindowsError::from(DXGI_ERROR_DEVICE_REMOVED)),
            FailureAction::RecoverDevice
        );
        assert_eq!(
            failure_action(&WindowsError::from(E_FAIL)),
            FailureAction::PreserveGraph
        );

        let rollback_error = rollback_recovery_error(
            &WindowsError::new(E_FAIL, "commit failed"),
            &WindowsError::new(E_FAIL, "rollback failed"),
        );
        assert_eq!(
            failure_action(&rollback_error),
            FailureAction::RecoverDevice,
            "an indeterminate visual tree must be recreated before any later commit"
        );
    }

    #[test]
    fn unchanged_clamped_opacity_requires_no_composition_mutation() {
        assert_eq!(changed_opacity(0.5, 0.5), None);
        assert_eq!(changed_opacity(1.0, 4.0), None);
        assert_eq!(changed_opacity(0.0, -2.0), None);
        assert_eq!(changed_opacity(0.5, 0.75), Some(0.75));
    }

    #[test]
    fn successful_recovery_still_reports_the_original_operation_failure() {
        let original = WindowsError::from(DXGI_ERROR_DEVICE_REMOVED);
        let result = recovered_operation_result(original, Ok(()))
            .expect_err("recovery requires a fresh complete role redraw");
        assert_eq!(result.code(), DXGI_ERROR_DEVICE_REMOVED);

        let recovery_error = WindowsError::new(E_FAIL, "recreation failed");
        let result = recovered_operation_result(
            WindowsError::from(DXGI_ERROR_DEVICE_REMOVED),
            Err(recovery_error),
        )
        .expect_err("failed recovery must remain failed");
        assert_eq!(result.code(), E_FAIL);
    }

    #[test]
    fn zero_sized_requests_are_normalized_to_nonzero_physical_pixels() {
        assert_eq!(physical_size(0, 0), (1, 1));
        assert_eq!(physical_size(20, 0), (20, 1));
    }
}
