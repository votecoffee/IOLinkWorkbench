use std::time::Duration;

#[cfg(windows)]
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::WAIT_OBJECT_0,
    System::Threading::{
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, SYNCHRONIZATION_SYNCHRONIZE,
        SetWaitableTimer, TIMER_MODIFY_STATE, WaitForSingleObject,
    },
};

pub struct DeadlineWait {
    #[cfg(windows)]
    timer: Option<OwnedHandle>,
}

impl DeadlineWait {
    pub fn new() -> Self {
        Self {
            #[cfg(windows)]
            timer: {
                // SAFETY: Null pointers select a private, non-inheritable timer.
                let raw = unsafe {
                    CreateWaitableTimerExW(
                        ptr::null(),
                        ptr::null(),
                        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                        SYNCHRONIZATION_SYNCHRONIZE | TIMER_MODIFY_STATE,
                    )
                };
                // SAFETY: A non-null return transfers ownership of a Windows HANDLE.
                (!raw.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(raw) })
            },
        }
    }

    pub fn label(&self) -> &'static str {
        #[cfg(windows)]
        if self.timer.is_some() {
            return "Windows high-resolution waitable timer";
        }
        "portable thread sleep"
    }

    pub fn wait(&mut self, duration: Duration) {
        #[cfg(windows)]
        if let Some(timer) = &self.timer {
            let handle = timer.as_raw_handle();
            let relative_due = -(duration.as_nanos().div_ceil(100).max(1) as i64);
            // SAFETY: The owned handle and due time live throughout both calls.
            let signaled = unsafe {
                SetWaitableTimer(handle, &relative_due, 0, None, ptr::null(), 0) != 0
                    && WaitForSingleObject(handle, 50) == WAIT_OBJECT_0
            };
            if !signaled {
                self.timer = None;
            }
            return;
        }
        std::thread::sleep(duration);
    }
}
