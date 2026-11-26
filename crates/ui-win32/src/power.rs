use windows::{
    core::Result,
    Win32::{Foundation::HWND, System::RemoteDesktop::*, UI::WindowsAndMessaging::*},
};
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PowerSessionEvent {
    Suspended,
    Resumed,
    SessionLocked,
    SessionUnlocked,
}
pub fn register_session_notifications(hwnd: HWND) -> Result<()> {
    unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) }
}
pub fn unregister_session_notifications(hwnd: HWND) -> Result<()> {
    unsafe { WTSUnRegisterSessionNotification(hwnd) }
}
pub fn classify_window_message(msg: u32, wparam: usize) -> Option<PowerSessionEvent> {
    use PowerSessionEvent::*;
    match (msg, u32::try_from(wparam).ok()?) {
        (WM_POWERBROADCAST, PBT_APMSUSPEND) => Some(Suspended),
        (WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND) => Some(Resumed),
        (WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK) => Some(SessionLocked),
        (WM_WTSSESSION_CHANGE, WTS_SESSION_UNLOCK) => Some(SessionUnlocked),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn message_family_matters() {
        use PowerSessionEvent::*;
        for (msg, code, event) in [
            (WM_POWERBROADCAST, PBT_APMSUSPEND, Suspended),
            (WM_POWERBROADCAST, PBT_APMRESUMESUSPEND, Resumed),
            (WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC, Resumed),
            (WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK, SessionLocked),
            (WM_WTSSESSION_CHANGE, WTS_SESSION_UNLOCK, SessionUnlocked),
        ] {
            assert_eq!(classify_window_message(msg, code as usize), Some(event));
        }
        assert_eq!(classify_window_message(WM_USER, 7), None);
        assert_eq!(classify_window_message(WM_POWERBROADCAST, 0xffff), None);
    }
}
