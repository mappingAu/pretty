//! Desktop notification delivery for OSC 9 / OSC 777.

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn send_notification(title: &str, body: &str) -> Result<(), String> {
    use notify_rust::{Notification, Timeout};
    Notification::new()
        .appname("Pretty")
        .summary(title)
        .body(body)
        .timeout(Timeout::Milliseconds(8000))
        .show()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn send_notification(title: &str, body: &str) -> Result<(), String> {
    log::warn!("desktop notifications unsupported on this platform: {title}: {body}");
    Err("desktop notifications are unsupported on this platform".into())
}
