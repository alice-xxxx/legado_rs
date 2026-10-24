#[cfg(target_os = "windows")]
mod platform {
    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::{CloseHandle, HANDLE},
            System::{
                Power::{
                    PowerClearRequest, PowerCreateRequest, PowerRequestDisplayRequired,
                    PowerRequestSystemRequired, PowerSetRequest,
                },
                Threading::{
                    POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
                },
            },
        },
    };
    pub struct Guard(isize);
    impl Guard {
        pub fn new() -> Result<Self, String> {
            let mut reason: Vec<u16> = "Keep the application display awake\0"
                .encode_utf16()
                .collect();
            let context = REASON_CONTEXT {
                Version: 0,
                Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
                Reason: REASON_CONTEXT_0 {
                    SimpleReasonString: PWSTR(reason.as_mut_ptr()),
                },
            };
            // A power request is handle-owned, so its lifetime is independent of the calling thread.
            unsafe {
                let handle = PowerCreateRequest(&context).map_err(|error| error.to_string())?;
                let guard = Self(handle.0 as isize);
                PowerSetRequest(handle, PowerRequestDisplayRequired)
                    .map_err(|error| error.to_string())?;
                PowerSetRequest(handle, PowerRequestSystemRequired)
                    .map_err(|error| error.to_string())?;
                Ok(guard)
            }
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                let handle = HANDLE(self.0 as *mut _);
                let _ = PowerClearRequest(handle, PowerRequestDisplayRequired);
                let _ = PowerClearRequest(handle, PowerRequestSystemRequired);
                let _ = CloseHandle(handle);
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use core_foundation::{
        base::TCFType,
        string::{CFString, CFStringRef},
    };
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(
            kind: CFStringRef,
            level: u32,
            reason: CFStringRef,
            id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }
    pub struct Guard(u32);
    impl Guard {
        pub fn new() -> Result<Self, String> {
            let kind = CFString::new("PreventUserIdleDisplaySleep");
            let reason = CFString::new("Keep the application display awake");
            let mut id = 0;
            let status = unsafe {
                IOPMAssertionCreateWithName(
                    kind.as_concrete_TypeRef(),
                    255,
                    reason.as_concrete_TypeRef(),
                    &mut id,
                )
            };
            if status != 0 {
                return Err(format!("Cannot inhibit display sleep: {status}"));
            }
            Ok(Self(id))
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                IOPMAssertionRelease(self.0);
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use gio::{
        glib::variant::ToVariant, prelude::*, BusType, Cancellable, DBusCallFlags, DBusProxy,
        DBusProxyFlags,
    };
    pub struct Guard {
        proxy: DBusProxy,
        cookie: u32,
        release: &'static str,
    }
    impl Guard {
        pub fn new() -> Result<Self, String> {
            // KDE/XFCE use ScreenSaver; GNOME exposes the session inhibitor.
            for (service, path, interface, args, release) in [
                (
                    "org.freedesktop.ScreenSaver",
                    "/org/freedesktop/ScreenSaver",
                    "org.freedesktop.ScreenSaver",
                    ("screen-controls", "Keep the application display awake").to_variant(),
                    "UnInhibit",
                ),
                (
                    "org.gnome.SessionManager",
                    "/org/gnome/SessionManager",
                    "org.gnome.SessionManager",
                    (
                        "screen-controls",
                        0u32,
                        "Keep the application display awake",
                        8u32,
                    )
                        .to_variant(),
                    "Uninhibit",
                ),
            ] {
                if let Ok(proxy) = DBusProxy::for_bus_sync(
                    BusType::Session,
                    DBusProxyFlags::DO_NOT_LOAD_PROPERTIES,
                    None,
                    service,
                    path,
                    interface,
                    None::<&Cancellable>,
                ) {
                    if let Ok(value) = proxy.call_sync(
                        "Inhibit",
                        Some(&args),
                        DBusCallFlags::NONE,
                        2000,
                        None::<&Cancellable>,
                    ) {
                        if let Some((cookie,)) = value.get::<(u32,)>() {
                            return Ok(Self {
                                proxy,
                                cookie,
                                release,
                            });
                        }
                    }
                }
            }
            Err("No desktop screen-saver inhibitor is available".into())
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = self.proxy.call_sync(
                self.release,
                Some(&(self.cookie,).to_variant()),
                DBusCallFlags::NONE,
                500,
                None::<&Cancellable>,
            );
        }
    }
}

pub use platform::Guard;
