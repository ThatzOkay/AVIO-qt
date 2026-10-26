//! Commands the frontend uses to send input to whichever AA session is running.

use std::sync::Arc;

use super::session_handle::AaSessionHandle;
use super::stack::channels::input_channel::{button_key, touch_action};
use super::stack::session::session::SessionCommand;

/// `x`/`y` are normalised 0..1 coordinates within the rendered video area. `phase` is
/// "down"/"move"/"up".
pub async fn aa_send_pointer(
    handle: &Arc<AaSessionHandle>,
    x: f64,
    y: f64,
    phase: String,
) -> Result<(), String> {
    let action = match phase.as_str() {
        "down" => touch_action::DOWN,
        "move" => touch_action::MOVED,
        "up" => touch_action::UP,
        other => return Err(format!("unknown touch phase: {other}")),
    };
    let (touch_w, touch_h) = handle.touch_size().await;
    let px = (x.clamp(0.0, 1.0) * touch_w as f64).round() as u32;
    let py = (y.clamp(0.0, 1.0) * touch_h as f64).round() as u32;
    println!("[AA touch] sending {action:?} at {px},{py} (norm {x:.3},{y:.3})");
    handle
        .send(SessionCommand::Touch {
            action,
            x: px,
            y: py,
        })
        .await;
    Ok(())
}

/// `touches` are `(x, y, id)` in normalised 0..1 coordinates plus each pointer's stable
/// per-finger id (the browser's `Touch.identifier`, stable across its own down->move->up).
/// `phase` is "down"/"pointerdown"/"move"/"pointerup"/"up" — the "pointer" variants cover
/// adding/removing a secondary finger without resetting the whole gesture. `action_index` is the
/// position within `touches` of whichever pointer triggered `phase` (ignored for "down"/"move"/"up").
pub async fn aa_send_touch(
    handle: &Arc<AaSessionHandle>,
    touches: Vec<(f64, f64, u32)>,
    phase: String,
    action_index: u32,
) -> Result<(), String> {
    let action = match phase.as_str() {
        "down" => touch_action::DOWN,
        "pointerdown" => touch_action::POINTER_DOWN,
        "move" => touch_action::MOVED,
        "pointerup" => touch_action::POINTER_UP,
        "up" => touch_action::UP,
        other => return Err(format!("unknown touch phase: {other}")),
    };

    let (touch_w, touch_h) = handle.touch_size().await;
    let touch_points: Vec<(u32, u32, u32)> = touches
        .into_iter()
        .map(|(x, y, id)| {
            let px = (x.clamp(0.0, 1.0) * touch_w as f64).round() as u32;
            let py = (y.clamp(0.0, 1.0) * touch_h as f64).round() as u32;
            (px, py, id)
        })
        .collect();

    handle
        .send(SessionCommand::MultiTouch {
            action,
            points: touch_points,
            action_index,
        })
        .await;

    Ok(())
}

/// Resumes projected AA content after the phone kicked the display back to its native/host UI
/// (see the "aa-status" event with payload "host-ui").
///
/// A bare `RequestVideoFocus` isn't reliably honored once the phone is holding NATIVE focus -
/// pairing it with a HOME button press first more reliably relinquishes that focus back to
/// projection.
pub async fn aa_resume(handle: &Arc<AaSessionHandle>) -> Result<(), String> {
    let key_codes = [button_key::HOME];
    handle
        .send(SessionCommand::Button {
            key_codes: key_codes.to_vec(),
            down: true,
            longpress: false,
        })
        .await;
    handle
        .send(SessionCommand::Button {
            key_codes: key_codes.to_vec(),
            down: false,
            longpress: false,
        })
        .await;
    let reached_session = handle.send(SessionCommand::RequestVideoFocus).await;
    println!(
        "[AA resume] HOME + RequestVideoFocus sent, reached a live session: {reached_session}"
    );
    Ok(())
}
