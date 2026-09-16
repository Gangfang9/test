# LE KeyMapper MVP

This branch is a deliberately narrow first product cut built on Scrcpy Mask.

## Scope

- One physical Android device connected over USB ADB
- scrcpy video mirroring
- WASD virtual direction pad
- Backquote (`) to enter or leave FPS view mode
- Left mouse button to fire while FPS view control remains active
- Visual editing for Direction Pad, FPS, and Fire mappings

Wireless ADB, audio, multi-device control, account services, external network access,
and additional automation mappings are outside this MVP.

## Quick start

1. Enable Developer options and USB debugging on the Android phone.
2. Connect the phone by USB and accept the computer authorization prompt.
3. Start LE KeyMapper and refresh the device list.
4. Click the control action for the USB device.
5. Open **Mappings** and adjust the built-in `default.json` positions to match the game UI.
6. Activate the mapping, press backquote to enter FPS mode, move with WASD, and fire with the left mouse button.

The built-in mapping uses a 1920×1080 reference canvas. Coordinates are scaled by
the existing Scrcpy Mask mapping engine for the active display.

## Design reference

The default layout follows behavioral patterns observed in the locally installed
QuickAssistant configuration files: normalized screen-relative placement, a dedicated
WASD joystick region, independent X/Y FPS sensitivity, a dedicated fire touch, and a
toggle key for mapping mode. No QuickAssistant executable code, APK code, proprietary
assets, account services, or branded game profiles are included.

## Privacy and safety defaults

- The web UI binds to `127.0.0.1` by default.
- Startup update checks are disabled.
- Clipboard synchronization is disabled by default.
- Network ADB identifiers and Android emulators are rejected by the MVP API.
- A second controlled device is rejected until the first is disconnected.

## Upstream

This project remains derived from AkiChase/scrcpy-mask and Genymobile/scrcpy under
the Apache License 2.0. Keep the upstream license and attribution when distributing builds.
