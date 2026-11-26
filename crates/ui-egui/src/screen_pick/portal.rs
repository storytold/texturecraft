//! Wayland: an app cannot read the screen or see clicks on other windows, so once the pointer leaves the app's windows the
//! desktop's own colour picker (the xdg-desktop-portal Screenshot.PickColor call) takes the click and reports the colour

use std::collections::HashMap;
use std::future::{Future, poll_fn};
use std::pin::{Pin, pin};
use std::sync::mpsc::Sender;
use std::task::Poll;
use std::time::Duration;

use zbus::AsyncDrop;
use zbus::blocking::{Proxy, connection::Builder};
use zbus::export::futures_core::Stream;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::{PickOutcome, PickShared};

/// How often the thread checks whether the pointer has left the app
const POINTER_POLL_INTERVAL: Duration = Duration::from_millis(30);
/// Bound portal method replies without timing out the user's interaction
const METHOD_REPLY_TIMEOUT: Duration = Duration::from_secs(5);

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
/// The token that names this app's request, so its answer can be awaited before the call is made
const TOKEN: &str = "texturecraft_eyedropper";
/// The portal's answer code for a pick the user made
const RESPONSE_SUCCESS: u32 = 0;
/// The portal's answer code for a pick the user cancelled; any other code is the desktop's own failure
const RESPONSE_CANCELLED: u32 = 1;

/// Waits for the pointer to leave the app's windows, then lets the desktop's picker take one click
pub(super) fn run(shared: &PickShared, outcomes: &Sender<PickOutcome>, ctx: &egui::Context) {
    while !shared.pointer_outside() {
        if shared.stopped() {
            return;
        }

        std::thread::sleep(POINTER_POLL_INTERVAL);
    }

    // The pick can end in the poll that sees the pointer leave
    if shared.stopped() {
        return;
    }

    let outcome = pick_color(shared).unwrap_or(PickOutcome::Unavailable);
    if shared.stopped() {
        return;
    }

    let _ = outcomes.send(outcome);
    ctx.request_repaint();
}

fn pick_color(shared: &PickShared) -> Option<PickOutcome> {
    let connection = Builder::session().ok()?.method_timeout(METHOD_REPLY_TIMEOUT).build().ok()?;
    let sender = connection.unique_name()?.trim_start_matches(':').replace('.', "_");
    let request_path = format!("{PORTAL_PATH}/request/{sender}/{TOKEN}");
    let request = Proxy::new(&connection, PORTAL, request_path, REQUEST_INTERFACE).ok()?;
    let screenshot = Proxy::new(&connection, PORTAL, PORTAL_PATH, "org.freedesktop.portal.Screenshot").ok()?;
    let mut responses = zbus::block_on(request.inner().receive_signal("Response")).ok()?;

    let Some(handle) = request_handle(&screenshot, &request, shared) else {
        zbus::block_on(responses.async_drop());
        return None;
    };

    let answer = zbus::block_on(poll_fn(|context| {
        if shared.watch_portal_stop(context.waker()) {
            return Poll::Ready(None);
        }

        Pin::new(&mut responses).poll_next(context)
    }));
    drop(shared.take_portal_waker());

    // A closed request does not send a response of its own
    if shared.stopped()
        && let Ok(active_request) = Proxy::new(&connection, PORTAL, handle.as_str(), REQUEST_INTERFACE)
    {
        let _ = active_request.call_noreply("Close", &());
    }

    zbus::block_on(responses.async_drop());
    let answer = answer?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = answer.body().deserialize().ok()?;
    if let Some(ended) = ended_without_colour(code) {
        return Some(ended);
    }

    let (red, green, blue): (f64, f64, f64) = results.get("color")?.try_clone().ok()?.try_into().ok()?;
    Some(PickOutcome::Picked([red, green, blue].map(to_byte)))
}

/// Start the pick, closing the predicted request if the UI cancels before the method replies
fn request_handle(screenshot: &Proxy<'_>, request: &Proxy<'_>, shared: &PickShared) -> Option<OwnedObjectPath> {
    let arguments = ("", HashMap::from([("handle_token", Value::from(TOKEN))]));
    let mut pending = pin!(screenshot.inner().call("PickColor", &arguments));
    let mut polled = false;
    let reply = zbus::block_on(poll_fn(|context| {
        if shared.watch_portal_stop(context.waker()) {
            return Poll::Ready(None);
        }

        polled = true;
        pending.as_mut().poll(context).map(Some)
    }));
    drop(shared.take_portal_waker());

    match reply {
        Some(result) => result.ok(),
        None if !polled => None,
        None => {
            if request.call::<_, _, ()>("Close", &()).is_ok() {
                return None;
            }

            // Some portals export the request late, so use the returned handle if early Close fails
            zbus::block_on(pending).ok()
        }
    }
}

/// How a portal answer of `code` ends the pick when it brings no colour: a cancel ends it, a failure leaves the pick on the model
fn ended_without_colour(code: u32) -> Option<PickOutcome> {
    match code {
        RESPONSE_SUCCESS => None,
        RESPONSE_CANCELLED => Some(PickOutcome::Cancelled),
        _ => Some(PickOutcome::Unavailable),
    }
}

/// A portal colour component (sRGB, 0 to 1) as 8 bits
fn to_byte(component: f64) -> u8 {
    if component.is_nan() {
        return 0;
    }

    (component.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_cancel_code_ends_the_pick_and_a_failure_keeps_it_on_the_model() {
        assert_eq!(ended_without_colour(RESPONSE_SUCCESS), None, "a pick goes on to its colour");
        assert_eq!(ended_without_colour(RESPONSE_CANCELLED), Some(PickOutcome::Cancelled));
        assert_eq!(ended_without_colour(2), Some(PickOutcome::Unavailable), "the desktop's own failure");
    }
}
