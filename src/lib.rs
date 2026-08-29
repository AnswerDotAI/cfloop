use std::ffi::c_void;

use pyo3::prelude::*;

type EventRef = *mut c_void;
type EventTargetRef = *mut c_void;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn RunApplicationEventLoop();
    fn QuitApplicationEventLoop();
    fn GetEventDispatcherTarget() -> EventTargetRef;
    fn ReceiveNextEvent(num_types: u64, list: *const c_void, timeout: f64, pull_event: u8, out_event: *mut EventRef) -> i32;
    fn SendEventToEventTarget(event: EventRef, target: EventTargetRef) -> i32;
    fn ReleaseEvent(event: EventRef);
    fn CreateEvent(allocator: *const c_void, class: u32, kind: u32, when: f64, attributes: u32, out_event: *mut EventRef) -> i32;
    fn PostEventToQueue(queue: *mut c_void, event: EventRef, priority: i16) -> i32;
    fn GetMainEventQueue() -> *mut c_void;
}

/// Pump Carbon events by hand for up to `timeout` seconds: wait for one event, dispatch it,
/// then drain the rest of the queue without waiting. Returns the count dispatched. The receive
/// blocks in the run loop, so main-loop timers and sources also fire during a pump.
#[pyfunction]
fn pump(py: Python, timeout: f64) -> u32 {
    py.detach(|| unsafe {
        let target = GetEventDispatcherTarget();
        let (mut n, mut wait) = (0, timeout);
        let mut evt: EventRef = std::ptr::null_mut();
        while ReceiveNextEvent(0, std::ptr::null(), wait, 1, &mut evt) == 0 {
            SendEventToEventTarget(evt, target);
            ReleaseEvent(evt);
            n += 1;
            wait = 0.0;
        }
        n
    })
}

type CFIndex = isize;
type CFRunLoopRef = *mut c_void;
type CFRunLoopTimerRef = *mut c_void;
type CFStringRef = *const c_void;

#[repr(C)]
struct CFRunLoopTimerContext {
    version: CFIndex,
    info: *mut c_void,
    retain: Option<extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<extern "C" fn(*const c_void)>,
    copy_description: Option<extern "C" fn(*const c_void) -> CFStringRef>,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRunLoopGetMain() -> CFRunLoopRef;
    fn CFAbsoluteTimeGetCurrent() -> f64;
    fn CFRunLoopTimerCreate(
        allocator: *const c_void,
        fire_date: f64,
        interval: f64,
        flags: u64,
        order: CFIndex,
        callout: extern "C" fn(CFRunLoopTimerRef, *mut c_void),
        context: *mut CFRunLoopTimerContext,
    ) -> CFRunLoopTimerRef;
    fn CFRunLoopAddTimer(rl: CFRunLoopRef, timer: CFRunLoopTimerRef, mode: CFStringRef);
    fn CFRelease(cf: *const c_void);
    fn CFSocketCreateWithNative(
        allocator: *const c_void,
        sock: i32,
        callback_types: u64,
        callout: extern "C" fn(*mut c_void, u64, *const c_void, *const c_void, *mut c_void),
        context: *mut c_void,
    ) -> *mut c_void;
    fn CFSocketCreateRunLoopSource(allocator: *const c_void, s: *mut c_void, order: CFIndex) -> *mut c_void;
    fn CFSocketInvalidate(s: *mut c_void);
    fn CFSocketGetSocketFlags(s: *mut c_void) -> u64;
    fn CFSocketSetSocketFlags(s: *mut c_void, flags: u64);
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: *mut c_void, mode: CFStringRef);
    static kCFRunLoopCommonModes: CFStringRef;
}

extern "C" fn timer_fired(_timer: CFRunLoopTimerRef, info: *mut c_void) {
    let cb = unsafe { Box::from_raw(info as *mut Py<PyAny>) };
    Python::attach(|py| {
        if let Err(e) = cb.call0(py) { e.print(py); }
        drop(cb);
    });
    post_wake_inner(); // the callback may have scheduled asyncio work, so pop any pump in progress
}

/// Run Carbon's application event loop on the calling thread until `quit_app`. Call on the
/// main thread: Carbon events and main-queue work only dispatch there. The asyncio arrangement
/// (`cfloop.run`) does not use this; it is for callers who want Carbon to own the thread.
#[pyfunction]
fn run_app(py: Python) { py.detach(|| unsafe { RunApplicationEventLoop() }); }

/// Quit `run_app`. Cross-thread this is a silent no-op (the quit event must post from the
/// loop's own thread), so call it from a callback the loop runs, e.g. via `call_later`.
#[pyfunction]
fn quit_app() { unsafe { QuitApplicationEventLoop() } }

/// Fire `callback` once on the main run loop, `delay` seconds from now, in common modes.
#[pyfunction]
fn call_later(delay: f64, callback: Py<PyAny>) {
    let info = Box::into_raw(Box::new(callback)) as *mut c_void;
    let mut ctx = CFRunLoopTimerContext { version: 0, info, retain: None, release: None, copy_description: None };
    unsafe {
        let t = CFRunLoopTimerCreate(std::ptr::null(), CFAbsoluteTimeGetCurrent() + delay, 0.0, 0, 0, timer_fired, &mut ctx);
        CFRunLoopAddTimer(CFRunLoopGetMain(), t, kCFRunLoopCommonModes);
        CFRelease(t as *const c_void);
    }
}

const WAKE_CLASS: u32 = 0x63666c70; // 'cflp'

fn post_wake_inner() {
    unsafe {
        let mut evt: EventRef = std::ptr::null_mut();
        if CreateEvent(std::ptr::null(), WAKE_CLASS, 0, 0.0, 0, &mut evt) == 0 {
            PostEventToQueue(GetMainEventQueue(), evt, 1); // kEventPriorityStandard
            ReleaseEvent(evt);
        }
    }
}

/// Post a no-op Carbon event, popping a `pump` in progress. The wake for anything that
/// schedules asyncio work from inside the pump without being a Carbon event itself.
#[pyfunction]
fn post_wake() { post_wake_inner() }

extern "C" fn fd_readable(_s: *mut c_void, _cbtype: u64, _addr: *const c_void, _data: *const c_void, _info: *mut c_void) { post_wake_inner() }

/// Watches an fd via a CFSocket source on the main run loop: when it turns readable during a
/// `pump`, a wake event pops the pump so the selector can collect. CFSocket read callbacks
/// auto-re-enable and fire on level (probed in DEV.md), so there is no arming state to lose:
/// a missed wake regenerates on the next wait while the fd stays readable. Its predecessor,
/// the one-shot CFFileDescriptor, wedged permanently under cross-thread load.
#[pyclass(unsendable)]
struct FdWatch { sock: *mut c_void, source: *mut c_void }

#[pymethods]
impl FdWatch {
    #[new]
    fn new(fd: i32) -> Self {
        unsafe {
            let sock = CFSocketCreateWithNative(std::ptr::null(), fd, 1, fd_readable, std::ptr::null_mut()); // kCFSocketReadCallBack
            CFSocketSetSocketFlags(sock, CFSocketGetSocketFlags(sock) & !0x80); // the kqueue fd is the selector's to close, not CFSocket's
            let source = CFSocketCreateRunLoopSource(std::ptr::null(), sock, 0);
            CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
            FdWatch { sock, source }
        }
    }
    fn invalidate(&self) {
        unsafe {
            CFSocketInvalidate(self.sock);
            CFRelease(self.source as *const c_void);
            CFRelease(self.sock as *const c_void);
        }
    }
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(run_app, m)?)?;
    m.add_function(wrap_pyfunction!(pump, m)?)?;
    m.add_function(wrap_pyfunction!(quit_app, m)?)?;
    m.add_function(wrap_pyfunction!(call_later, m)?)?;
    m.add_function(wrap_pyfunction!(post_wake, m)?)?;
    m.add_class::<FdWatch>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
