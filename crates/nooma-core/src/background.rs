//! Work done while a person works: below their programs, and not on battery.
//!
//! Indexing nobody waits for - the window keeping up with the folders,
//! `nooma index --watch` - shares the machine with whatever its owner is
//! doing, and it must never get in the way. Three things keep it out of it.
//! Its threads are few: half the cores, at most four
//! ([`crate::library::indexing_threads`]). They run below the person's
//! programs, so the system gives them the processor and the disk when nothing
//! else wants them. And on a laptop running on its battery the work holds
//! where it is and goes on when the power is back: an hour of reading scans
//! is an hour of battery a person would rather keep.
//!
//! A search is never held or lowered: someone is waiting for it.

use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// How long the power state is believed before the system is asked again,
/// and how often held work asks whether it may go on.
const POWER_CHECK: Duration = Duration::from_secs(5);

/// Lower the calling thread below the person's programs: background mode on
/// Windows, which lowers its disk priority with its processor priority; a
/// niceness of 10 on Linux; the background quality of service on macOS,
/// which throttles its disk too. Returns whether the system lowered it.
///
/// A thread is lowered for good: lower only one that does nothing else.
pub fn lower_this_thread() -> bool {
    sys::lower()
}

/// Whether the machine runs on its battery now: `false` on one with none, or
/// where the system will not say.
///
/// Asked of the system at most every few seconds: held work asks before
/// every file it reads.
pub fn on_battery() -> bool {
    static KNOWN: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let mut known = KNOWN.lock().unwrap_or_else(PoisonError::into_inner);
    match *known {
        Some((asked, state)) if asked.elapsed() < POWER_CHECK => state,
        _ => {
            let state = sys::on_battery();
            *known = Some((Instant::now(), state));
            state
        }
    }
}

/// Hold background work while the machine runs on its battery: return at
/// once on mains power, or once the power is back. `holding` is told when
/// the wait begins, once.
pub fn hold(holding: &dyn Fn()) {
    if !on_battery() {
        return;
    }
    holding();
    while on_battery() {
        std::thread::sleep(POWER_CHECK);
    }
}

/// Whether Windows says the machine is off the mains: its AC line status is
/// 0 on battery, 1 on mains and 255 when it does not know.
#[cfg_attr(not(windows), allow(dead_code))]
fn unplugged(ac_line_status: u8) -> bool {
    ac_line_status == 0
}

/// Whether macOS says the power comes from a battery: the type of the
/// source providing it, a UPS being a battery too.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn drawn_from_a_battery(source: &str) -> bool {
    matches!(source, "Battery Power" | "UPS Power")
}

/// Whether a battery of the machine is discharging, as Linux's power-supply
/// directory - `/sys/class/power_supply` - says. A wireless mouse's battery
/// is not the machine's: its scope is `Device`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn discharging(supplies: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(supplies) else { return false };
    entries.flatten().any(|supply| {
        let read = |name: &str| {
            std::fs::read_to_string(supply.path().join(name))
                .map(|text| text.trim().to_string())
                .unwrap_or_default()
        };
        read("type") == "Battery" && read("scope") != "Device" && read("status") == "Discharging"
    })
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;

    /// `SYSTEM_POWER_STATUS`.
    #[repr(C)]
    #[derive(Default)]
    struct PowerStatus {
        ac_line_status: u8,
        battery_flag: u8,
        battery_life_percent: u8,
        system_status_flag: u8,
        battery_life_time: u32,
        battery_full_life_time: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> *mut c_void;
        fn SetThreadPriority(thread: *mut c_void, priority: i32) -> i32;
        fn GetSystemPowerStatus(status: *mut PowerStatus) -> i32;
    }

    const THREAD_MODE_BACKGROUND_BEGIN: i32 = 0x0001_0000;
    #[cfg(test)]
    const THREAD_MODE_BACKGROUND_END: i32 = 0x0002_0000;
    const ERROR_THREAD_MODE_ALREADY_BACKGROUND: i32 = 400;

    pub(super) fn lower() -> bool {
        // SAFETY: the pseudo-handle of the calling thread, valid for as long
        // as the thread is, which is longer than the call.
        let lowered = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) } != 0;
        lowered || std::io::Error::last_os_error().raw_os_error() == Some(ERROR_THREAD_MODE_ALREADY_BACKGROUND)
    }

    /// Whether the calling thread is in background mode: entering it again
    /// is refused for being there already. Left as it was found.
    #[cfg(test)]
    pub(super) fn lowered() -> bool {
        // SAFETY: as in `lower`.
        let entered = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) } != 0;
        if entered {
            // SAFETY: as in `lower`; leaves the mode just entered.
            unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_END) };
            return false;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(ERROR_THREAD_MODE_ALREADY_BACKGROUND)
    }

    pub(super) fn on_battery() -> bool {
        let mut status = PowerStatus::default();
        // SAFETY: the function fills the structure it is pointed at, which
        // lives through the call.
        let told = unsafe { GetSystemPowerStatus(&raw mut status) } != 0;
        told && super::unplugged(status.ac_line_status)
    }
}

#[cfg(target_os = "linux")]
mod sys {
    /// The niceness background threads run at: below every program started
    /// as usual, above those started to wait for an idle machine.
    const NICE: libc::c_int = 10;

    fn this_thread() -> libc::id_t {
        // SAFETY: no arguments, and it cannot fail.
        let tid = unsafe { libc::gettid() };
        libc::id_t::try_from(tid).unwrap_or(0)
    }

    fn niceness() -> libc::c_int {
        // SAFETY: plain values; on Linux the process id of a thread names the
        // thread alone.
        unsafe { libc::getpriority(libc::PRIO_PROCESS, this_thread()) }
    }

    pub(super) fn lower() -> bool {
        // Never raised: a nooma started nicer than this stays as nice.
        let nice = niceness().max(NICE);
        // SAFETY: as in `niceness`.
        unsafe { libc::setpriority(libc::PRIO_PROCESS, this_thread(), nice) == 0 }
    }

    #[cfg(test)]
    pub(super) fn lowered() -> bool {
        niceness() >= NICE
    }

    pub(super) fn on_battery() -> bool {
        super::discharging(std::path::Path::new("/sys/class/power_supply"))
    }
}

#[cfg(target_os = "macos")]
mod sys {
    use std::ffi::{CStr, c_char, c_void};

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPSCopyPowerSourcesInfo() -> *const c_void;
        fn IOPSGetProvidingPowerSourceType(snapshot: *const c_void) -> *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringGetCString(string: *const c_void, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFRelease(object: *const c_void);
    }

    /// `kCFStringEncodingUTF8`.
    const UTF8: u32 = 0x0800_0100;

    pub(super) fn lower() -> bool {
        // SAFETY: changes the calling thread's own class, nothing else.
        unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0) == 0 }
    }

    #[cfg(test)]
    pub(super) fn lowered() -> bool {
        let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
        let mut priority = 0;
        // SAFETY: reads the calling thread's own class into the two values,
        // which live through the call.
        let read = unsafe { libc::pthread_get_qos_class_np(libc::pthread_self(), &raw mut class, &raw mut priority) } == 0;
        read && class as u32 == libc::qos_class_t::QOS_CLASS_BACKGROUND as u32
    }

    pub(super) fn on_battery() -> bool {
        let mut name: [c_char; 64] = [0; 64];
        // SAFETY: the snapshot is released once, after the type it names is
        // copied out of it; a null snapshot or type is no answer.
        let read = unsafe {
            let snapshot = IOPSCopyPowerSourcesInfo();
            if snapshot.is_null() {
                return false;
            }
            let source = IOPSGetProvidingPowerSourceType(snapshot);
            let read = !source.is_null() && CFStringGetCString(source, name.as_mut_ptr(), name.len() as isize, UTF8) != 0;
            CFRelease(snapshot);
            read
        };
        // SAFETY: CFStringGetCString wrote a terminated string into the buffer.
        read && super::drawn_from_a_battery(unsafe { CStr::from_ptr(name.as_ptr()) }.to_str().unwrap_or(""))
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod sys {
    pub(super) fn lower() -> bool {
        false
    }

    #[cfg(test)]
    pub(super) fn lowered() -> bool {
        false
    }

    pub(super) fn on_battery() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    fn a_lowered_thread_runs_below_the_others() {
        std::thread::spawn(|| {
            assert!(!sys::lowered(), "a thread starts at the usual priority");
            assert!(lower_this_thread());
            assert!(sys::lowered(), "the system did not lower the thread");
            assert!(lower_this_thread(), "lowering a lowered thread is no failure");
        })
        .join()
        .unwrap();
        std::thread::spawn(|| assert!(!sys::lowered(), "another thread was lowered with it"))
            .join()
            .unwrap();
    }

    #[test]
    fn the_mains_is_told_from_the_battery() {
        assert!(unplugged(0));
        assert!(!unplugged(1));
        assert!(!unplugged(255), "not knowing is not being on battery");
        assert!(drawn_from_a_battery("Battery Power"));
        assert!(drawn_from_a_battery("UPS Power"));
        assert!(!drawn_from_a_battery("AC Power"));
        assert!(!drawn_from_a_battery(""));
    }

    #[test]
    fn a_discharging_battery_of_the_machine_is_being_on_battery() {
        let supplies = tempfile::tempdir().unwrap();
        let supply = |name: &str, fields: &[(&str, &str)]| {
            let dir = supplies.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            for (field, value) in fields {
                std::fs::write(dir.join(field), format!("{value}\n")).unwrap();
            }
        };
        assert!(!discharging(supplies.path()), "a machine with no supplies is not on battery");
        assert!(!discharging(&supplies.path().join("missing")));
        supply("AC", &[("type", "Mains"), ("online", "1")]);
        supply("BAT0", &[("type", "Battery"), ("status", "Charging")]);
        // A mouse running down is not the machine running down.
        supply("hid-mouse-battery", &[("type", "Battery"), ("scope", "Device"), ("status", "Discharging")]);
        assert!(!discharging(supplies.path()));
        supply("BAT0", &[("type", "Battery"), ("status", "Discharging")]);
        assert!(discharging(supplies.path()));
    }
}
