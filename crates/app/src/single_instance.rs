use windows::{
    core::{w, Error},
    Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, LPARAM, WPARAM},
        System::Threading::CreateMutexW,
        UI::WindowsAndMessaging::{PostMessageW, RegisterWindowMessageW, HWND_BROADCAST},
    },
};

pub fn activate_message_id() -> u32 {
    unsafe { RegisterWindowMessageW(w!("TokenPulse.ActivateExisting.v1")) }
}

pub enum InstanceGuard {
    Primary(HANDLE),
    AlreadyRunning,
}

impl InstanceGuard {
    pub fn acquire() -> Result<Self, String> {
        let message = activate_message_id();
        if message == 0 {
            return Err(Error::from_win32().to_string());
        }
        unsafe {
            let handle = CreateMutexW(None, false, w!("Local\\TokenPulse-SingleInstance-v1"))
                .map_err(|e| e.to_string())?;
            let exists = GetLastError() == ERROR_ALREADY_EXISTS;
            let guard = Self::Primary(handle);
            if exists {
                PostMessageW(HWND_BROADCAST, message, WPARAM(0), LPARAM(0))
                    .map_err(|e| e.to_string())?;
                Ok(Self::AlreadyRunning)
            } else {
                Ok(guard)
            }
        }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if let Self::Primary(handle) = self {
            // The mutex was created without initial ownership.
            unsafe {
                let _ = CloseHandle(*handle);
            }
        }
    }
}
