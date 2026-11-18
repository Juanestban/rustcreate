//! Pen extras on Windows Ink (ADR 0004). winit already turns `WM_POINTER` into egui touch events
//! with pressure and every coalesced sample; this adds what it drops: the eraser end, the barrel
//! button and the tilt, read from `GetPointerPenInfo` in a window subclass that then passes every
//! message on to winit. It also turns off Windows Ink's press-and-hold right click and tap
//! ripples, which get in the way of painting.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static ERASER: AtomicBool = AtomicBool::new(false);
static BARREL: AtomicBool = AtomicBool::new(false);
/// `f32` bits: 0 upright ..= 1 flat.
static TILT: AtomicU32 = AtomicU32::new(0);

/// What the pen's buttons are doing right now, and how far it leans.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Buttons {
  /// The pen is flipped to its eraser end.
  pub eraser: bool,
  /// The side (barrel) button is held.
  pub barrel: bool,
  /// 0 upright ..= 1 lying flat; 0 for pens without tilt and for the mouse.
  pub tilt: f32,
}

pub fn buttons() -> Buttons {
  Buttons { eraser: ERASER.load(Ordering::Relaxed), barrel: BARREL.load(Ordering::Relaxed), tilt: f32::from_bits(TILT.load(Ordering::Relaxed)) }
}

/// The lean from vertical, 0..=1, from Windows Ink's tilt toward X and Y in degrees.
#[cfg_attr(not(windows), allow(dead_code))]
fn lean(tilt_x: i32, tilt_y: i32) -> f32 {
  let (x, y) = ((tilt_x as f32).to_radians().tan(), (tilt_y as f32).to_radians().tan());
  (x.hypot(y).atan() / std::f32::consts::FRAC_PI_2).clamp(0.0, 1.0)
}

#[cfg(not(windows))]
pub fn install(_window: &impl raw_window_handle::HasWindowHandle) {}

#[cfg(windows)]
pub use windows::install;

#[cfg(windows)]
mod windows {
  use std::ffi::c_void;
  use std::sync::atomic::Ordering;

  use raw_window_handle::{HasWindowHandle, RawWindowHandle};
  use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
  use windows_sys::Win32::UI::Controls::{
    FEEDBACK_PEN_BARRELVISUALIZATION, FEEDBACK_PEN_DOUBLETAP, FEEDBACK_PEN_PRESSANDHOLD, FEEDBACK_PEN_RIGHTTAP, FEEDBACK_PEN_TAP,
    SetWindowFeedbackSetting,
  };
  use windows_sys::Win32::UI::Input::Pointer::{GetPointerPenInfo, GetPointerType, POINTER_PEN_INFO};
  use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
  use windows_sys::Win32::UI::WindowsAndMessaging::{
    PEN_FLAG_BARREL, PEN_FLAG_ERASER, PEN_FLAG_INVERTED, PEN_MASK_TILT_X, PEN_MASK_TILT_Y, PT_PEN, SetPropW, WM_POINTERDOWN, WM_POINTERLEAVE,
    WM_POINTERUP, WM_POINTERUPDATE,
  };

  use super::{BARREL, ERASER, TILT, lean};

  // From tpcshrd.h: the flags Windows Ink reads from this window property.
  const TABLET_DISABLE_PRESSANDHOLD: usize = 0x0000_0001;
  const TABLET_DISABLE_PENTAPFEEDBACK: usize = 0x0000_0008;
  const TABLET_DISABLE_PENBARRELFEEDBACK: usize = 0x0000_0010;
  const TABLET_DISABLE_FLICKS: usize = 0x0001_0000;

  pub fn install(window: &impl HasWindowHandle) {
    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else { return };
    let hwnd = handle.hwnd.get() as HWND;
    let property: Vec<u16> = "MicrosoftTabletPenServiceProperty\0".encode_utf16().collect();
    let flags = TABLET_DISABLE_PRESSANDHOLD | TABLET_DISABLE_PENTAPFEEDBACK | TABLET_DISABLE_PENBARRELFEEDBACK | TABLET_DISABLE_FLICKS;
    let off: i32 = 0;
    // SAFETY: `hwnd` is eframe's live window; the property and settings are plain values, and the
    // subclass procedure only reads pen state before handing the message on unchanged.
    unsafe {
      SetPropW(hwnd, property.as_ptr(), flags as *mut c_void);
      for feedback in [FEEDBACK_PEN_BARRELVISUALIZATION, FEEDBACK_PEN_TAP, FEEDBACK_PEN_DOUBLETAP, FEEDBACK_PEN_PRESSANDHOLD, FEEDBACK_PEN_RIGHTTAP] {
        SetWindowFeedbackSetting(hwnd, feedback, 0, size_of::<i32>() as u32, (&raw const off).cast());
      }
      SetWindowSubclass(hwnd, Some(subclass), 1, 0);
    }
  }

  unsafe extern "system" fn subclass(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    match message {
      WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => {
        let pointer = (wparam & 0xffff) as u32;
        let mut kind = 0;
        let mut info: POINTER_PEN_INFO = unsafe { std::mem::zeroed() };
        // SAFETY: plain out-parameters for the pointer this message is about.
        if unsafe { GetPointerType(pointer, &mut kind) } != 0 && kind == PT_PEN && unsafe { GetPointerPenInfo(pointer, &mut info) } != 0 {
          ERASER.store(info.penFlags & (PEN_FLAG_ERASER | PEN_FLAG_INVERTED) != 0, Ordering::Relaxed);
          BARREL.store(info.penFlags & PEN_FLAG_BARREL != 0, Ordering::Relaxed);
          let x = if info.penMask & PEN_MASK_TILT_X != 0 { info.tiltX } else { 0 };
          let y = if info.penMask & PEN_MASK_TILT_Y != 0 { info.tiltY } else { 0 };
          TILT.store(lean(x, y).to_bits(), Ordering::Relaxed);
        }
      }
      WM_POINTERLEAVE => {
        ERASER.store(false, Ordering::Relaxed);
        BARREL.store(false, Ordering::Relaxed);
        TILT.store(0, Ordering::Relaxed);
      }
      _ => {}
    }
    // SAFETY: forwards the untouched message to the next procedure (winit's).
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
  }
}

#[cfg(test)]
mod tests {
  use super::lean;

  #[test]
  fn lean_from_tilt_angles() {
    assert_eq!(lean(0, 0), 0.0);
    assert!((lean(45, 0) - 0.5).abs() < 1e-4);
    assert!((lean(0, -90) - 1.0).abs() < 1e-4);
    assert!(lean(40, 40) > lean(40, 0), "leaning both ways leans further");
  }
}
