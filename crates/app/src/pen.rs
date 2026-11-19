//! Pen extras on Windows Ink (ADR 0004). winit already turns `WM_POINTER` into egui touch events
//! with pressure and every coalesced sample; this adds what it drops: the eraser end, the barrel
//! button and the tilt, read from `GetPointerPenInfo` in a window subclass that then passes every
//! message on to winit. It also turns off Windows Ink's press-and-hold right click and tap
//! ripples, which get in the way of painting.
//!
//! Older tablets that do not speak Windows Ink can use **Wintab** instead (`wintab32.dll`, from
//! the tablet's driver), switched on in the settings: its pressure, tilt, eraser end and barrel
//! button then come with the mouse events those tablets send.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static ERASER: AtomicBool = AtomicBool::new(false);
static BARREL: AtomicBool = AtomicBool::new(false);
/// `f32` bits: 0 upright ..= 1 flat.
static TILT: AtomicU32 = AtomicU32::new(0);
/// `f32` bits of Wintab's pressure while the pen is near the tablet; `NO_PRESSURE` otherwise.
static WINTAB_PRESSURE: AtomicU32 = AtomicU32::new(NO_PRESSURE);
const NO_PRESSURE: u32 = u32::MAX;

/// Wintab's pressure, 0..=1, while a Wintab pen is near the tablet.
pub fn wintab_pressure() -> Option<f32> {
  match WINTAB_PRESSURE.load(Ordering::Relaxed) {
    NO_PRESSURE => None,
    bits => Some(f32::from_bits(bits)),
  }
}

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

/// Turns Wintab on or off; false if it was asked for and there is no Wintab driver.
#[cfg(not(windows))]
pub fn set_wintab(_on: bool) -> bool {
  true
}

#[cfg(windows)]
pub use windows::{install, set_wintab};

#[cfg(windows)]
mod windows {
  use std::ffi::c_void;
  use std::sync::Mutex;
  use std::sync::atomic::{AtomicIsize, Ordering};

  use raw_window_handle::{HasWindowHandle, RawWindowHandle};
  use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
  use windows_sys::Win32::UI::Controls::{
    FEEDBACK_PEN_BARRELVISUALIZATION, FEEDBACK_PEN_DOUBLETAP, FEEDBACK_PEN_PRESSANDHOLD, FEEDBACK_PEN_RIGHTTAP, FEEDBACK_PEN_TAP,
    SetWindowFeedbackSetting,
  };
  use windows_sys::Win32::UI::Input::Pointer::{GetPointerPenInfo, GetPointerType, POINTER_PEN_INFO};
  use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
  use windows_sys::Win32::UI::WindowsAndMessaging::{
    PEN_FLAG_BARREL, PEN_FLAG_ERASER, PEN_FLAG_INVERTED, PEN_MASK_TILT_X, PEN_MASK_TILT_Y, PT_PEN, SetPropW, WM_ACTIVATE, WM_POINTERDOWN,
    WM_POINTERLEAVE, WM_POINTERUP, WM_POINTERUPDATE,
  };

  use super::{BARREL, ERASER, NO_PRESSURE, TILT, WINTAB_PRESSURE, lean, wintab};

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
    WINDOW.store(hwnd as isize, Ordering::Relaxed);
  }

  /// eframe's window, for opening a Wintab context later.
  static WINDOW: AtomicIsize = AtomicIsize::new(0);
  static WINTAB: Mutex<Option<wintab::Context>> = Mutex::new(None);

  pub fn set_wintab(on: bool) -> bool {
    let Ok(mut context) = WINTAB.lock() else { return false };
    if on == context.is_some() {
      return true;
    }
    if !on {
      *context = None;
      WINTAB_PRESSURE.store(NO_PRESSURE, Ordering::Relaxed);
      return true;
    }
    let hwnd = WINDOW.load(Ordering::Relaxed) as HWND;
    *context = (!hwnd.is_null()).then(|| wintab::Context::open(hwnd)).flatten();
    context.is_some()
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
      wintab::WT_PACKET | wintab::WT_PROXIMITY | WM_ACTIVATE => {
        // try_lock: Wintab may send messages from inside WTOpen/WTClose, which hold the lock on
        // this same thread.
        if let Ok(context) = WINTAB.try_lock()
          && let Some(context) = context.as_ref()
        {
          match message {
            wintab::WT_PACKET => {
              if let Some(pen) = context.packet(wparam as u32, lparam) {
                ERASER.store(pen.eraser, Ordering::Relaxed);
                BARREL.store(pen.barrel, Ordering::Relaxed);
                TILT.store(pen.tilt.to_bits(), Ordering::Relaxed);
                WINTAB_PRESSURE.store(pen.pressure.to_bits(), Ordering::Relaxed);
              }
            }
            // The pen left the tablet: back to the mouse's full pressure.
            wintab::WT_PROXIMITY if lparam & 0xffff == 0 => {
              WINTAB_PRESSURE.store(NO_PRESSURE, Ordering::Relaxed);
              ERASER.store(false, Ordering::Relaxed);
              BARREL.store(false, Ordering::Relaxed);
              TILT.store(0, Ordering::Relaxed);
            }
            // Wintab's own advice: bring the context to the top when the window is activated.
            WM_ACTIVATE => context.activate(wparam & 0xffff != 0),
            _ => {}
          }
        }
      }
      _ => {}
    }
    // SAFETY: forwards the untouched message to the next procedure (winit's).
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
  }
}

/// Wintab 1.4 (`wintab32.dll`, installed by the tablet driver), loaded at run time: only the
/// calls and fields used here, from the specification's `wintab.h`.
#[cfg(windows)]
mod wintab {
  use std::ffi::c_void;

  use windows_sys::Win32::Foundation::{HWND, LPARAM};
  use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

  const WT_DEFBASE: u32 = 0x7FF0;
  pub const WT_PACKET: u32 = WT_DEFBASE;
  pub const WT_PROXIMITY: u32 = WT_DEFBASE + 5;
  const WTI_DEFSYSCTX: u32 = 4;
  const WTI_DEVICES: u32 = 100;
  const DVC_NPRESSURE: u32 = 15;
  const DVC_ORIENTATION: u32 = 17;
  const CXO_SYSTEM: u32 = 0x0001;
  const CXO_MESSAGES: u32 = 0x0004;
  const PK_STATUS: u32 = 0x0002;
  const PK_BUTTONS: u32 = 0x0040;
  const PK_NORMAL_PRESSURE: u32 = 0x0400;
  const PK_ORIENTATION: u32 = 0x1000;
  /// The packet below: these fields, in this order (the order of their bits).
  const PACKET_DATA: u32 = PK_STATUS | PK_BUTTONS | PK_NORMAL_PRESSURE | PK_ORIENTATION;
  /// pkStatus: the pen is upside down, eraser end on the tablet.
  const TPS_INVERT: u32 = 0x0010;

  #[repr(C)]
  #[derive(Default, Clone, Copy)]
  struct Axis {
    min: i32,
    max: i32,
    units: u32,
    resolution: u32,
  }

  #[repr(C)]
  struct LogContext {
    name: [u16; 40],
    options: u32,
    status: u32,
    locks: u32,
    msg_base: u32,
    device: u32,
    pkt_rate: u32,
    pkt_data: u32,
    pkt_mode: u32,
    move_mask: u32,
    btn_dn_mask: u32,
    btn_up_mask: u32,
    in_org: [i32; 3],
    in_ext: [i32; 3],
    out_org: [i32; 3],
    out_ext: [i32; 3],
    sens: [u32; 3],
    sys_mode: i32,
    sys_org: [i32; 2],
    sys_ext: [i32; 2],
    sys_sens: [u32; 2],
  }

  #[repr(C)]
  #[derive(Default)]
  struct Packet {
    status: u32,
    buttons: u32,
    pressure: u32,
    /// Azimuth, altitude and twist, in tenths of a degree.
    orientation: [i32; 3],
  }

  type Info = unsafe extern "system" fn(u32, u32, *mut c_void) -> u32;
  type Open = unsafe extern "system" fn(HWND, *mut LogContext, i32) -> *mut c_void;
  type Close = unsafe extern "system" fn(*mut c_void) -> i32;
  type GetPacket = unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32;
  type Enable = unsafe extern "system" fn(*mut c_void, i32) -> i32;

  /// The pen as one packet saw it.
  pub struct Pen {
    pub pressure: f32,
    pub tilt: f32,
    pub eraser: bool,
    pub barrel: bool,
  }

  /// An open Wintab context on the app's window, closed when dropped.
  pub struct Context {
    handle: *mut c_void,
    close: Close,
    packet: GetPacket,
    enable: Enable,
    overlap: Enable,
    max_pressure: f32,
    /// The altitude axis' top (pen upright), when the tablet reports tilt.
    max_altitude: Option<f32>,
  }

  // SAFETY: the context handle is only used from the window's thread (its messages) and the
  // settings, both behind `WINTAB`'s mutex.
  unsafe impl Send for Context {}

  impl Context {
    pub fn open(hwnd: HWND) -> Option<Self> {
      let name: Vec<u16> = "wintab32.dll\0".encode_utf16().collect();
      // SAFETY: loading the driver's DLL by name; every function is checked for presence and
      // called with the signature the Wintab specification gives it.
      unsafe {
        let library = LoadLibraryW(name.as_ptr());
        if library.is_null() {
          return None;
        }
        let function = |name: &[u8]| GetProcAddress(library, name.as_ptr());
        let info: Info = std::mem::transmute(function(b"WTInfoW\0")?);
        let open: Open = std::mem::transmute(function(b"WTOpenW\0")?);
        let close: Close = std::mem::transmute(function(b"WTClose\0")?);
        let packet: GetPacket = std::mem::transmute(function(b"WTPacket\0")?);
        let enable: Enable = std::mem::transmute(function(b"WTEnable\0")?);
        let overlap: Enable = std::mem::transmute(function(b"WTOverlap\0")?);
        if info(0, 0, std::ptr::null_mut()) == 0 {
          return None;
        }
        let mut context: LogContext = std::mem::zeroed();
        if info(WTI_DEFSYSCTX, 0, (&raw mut context).cast()) == 0 {
          return None;
        }
        let mut pressure = Axis::default();
        info(WTI_DEVICES, DVC_NPRESSURE, (&raw mut pressure).cast());
        let mut orientation = [Axis::default(); 3];
        let tilt = info(WTI_DEVICES, DVC_ORIENTATION, orientation.as_mut_ptr().cast()) != 0 && orientation[1].max > 0;
        for (slot, c) in context.name.iter_mut().zip("rustcreate".encode_utf16()) {
          *slot = c;
        }
        // Messages to our window; the system cursor still follows the pen.
        context.options |= CXO_MESSAGES | CXO_SYSTEM;
        context.msg_base = WT_DEFBASE;
        context.pkt_data = PACKET_DATA;
        context.pkt_mode = 0;
        context.move_mask = PACKET_DATA;
        context.btn_up_mask = context.btn_dn_mask;
        let handle = open(hwnd, &raw mut context, 1);
        if handle.is_null() {
          return None;
        }
        Some(Self {
          handle,
          close,
          packet,
          enable,
          overlap,
          max_pressure: pressure.max.max(1) as f32,
          max_altitude: tilt.then_some(orientation[1].max as f32),
        })
      }
    }

    /// The packet a `WT_PACKET` message announces (`serial`, from this context).
    pub fn packet(&self, serial: u32, context: LPARAM) -> Option<Pen> {
      if context as *mut c_void != self.handle {
        return None;
      }
      let mut packet = Packet::default();
      // SAFETY: `packet` has exactly the fields `PACKET_DATA` asks for.
      if unsafe { (self.packet)(self.handle, serial, (&raw mut packet).cast()) } == 0 {
        return None;
      }
      Some(Pen {
        pressure: (packet.pressure as f32 / self.max_pressure).clamp(0.0, 1.0),
        // Altitude is 90° upright: lean is how far from that.
        tilt: self.max_altitude.map_or(0.0, |max| (1.0 - packet.orientation[1].unsigned_abs() as f32 / max).clamp(0.0, 1.0)),
        eraser: packet.status & TPS_INVERT != 0,
        // Button 0 is the tip; 1 the lower barrel button.
        barrel: packet.buttons & 0b10 != 0,
      })
    }

    pub fn activate(&self, active: bool) {
      // SAFETY: a live context handle.
      unsafe {
        (self.enable)(self.handle, active as i32);
        if active {
          (self.overlap)(self.handle, 1);
        }
      }
    }
  }

  impl Drop for Context {
    fn drop(&mut self) {
      // SAFETY: closes the context this value opened, once.
      unsafe {
        (self.close)(self.handle);
      }
    }
  }

  #[cfg(test)]
  mod tests {
    #[test]
    fn structs_match_wintab_h() {
      // LOGCONTEXTW is 212 bytes; this packet 24.
      assert_eq!(size_of::<super::LogContext>(), 212);
      assert_eq!(size_of::<super::Packet>(), 24);
    }
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
