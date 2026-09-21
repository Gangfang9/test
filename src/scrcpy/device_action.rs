use tokio::sync::broadcast;

use crate::scrcpy::{
    constant::{KeyEventAction, Keycode, MetaState},
    control_msg::ScrcpyControlMsg,
};

/// Send a key Down + Up sequence immediately.
pub fn inject_keycode(cs_tx: &broadcast::Sender<ScrcpyControlMsg>, keycode: Keycode) {
    if let Err(error) = cs_tx.send(ScrcpyControlMsg::InjectKeycode {
        action: KeyEventAction::Down,
        keycode: keycode.clone(),
        repeat: 0,
        metastate: MetaState::NONE,
    }) {
        log::error!("[DeviceAction] failed to send key down for {keycode:?}: {error}");
        return;
    }
    if let Err(error) = cs_tx.send(ScrcpyControlMsg::InjectKeycode {
        action: KeyEventAction::Up,
        keycode: keycode.clone(),
        repeat: 0,
        metastate: MetaState::NONE,
    }) {
        log::error!("[DeviceAction] failed to send key up for {keycode:?}: {error}");
    }
}

/// Turn the device display on (mode: true) or off (mode: false).
pub fn set_display_power(cs_tx: &broadcast::Sender<ScrcpyControlMsg>, mode: bool) {
    let _ = cs_tx.send(ScrcpyControlMsg::SetDisplayPower { mode });
}
