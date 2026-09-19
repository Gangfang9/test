# Native UI migration parity checklist

The native Windows client must reach feature parity before the browser UI and
local HTTP control layer are removed from release builds.

## Device management

- List USB ADB devices and authorization state.
- Refresh devices and restart ADB.
- Start, stop, reconnect, and recover timed-out projection sessions.
- Show device identity, name, display size, connection state, and errors.
- Keep the single-device MVP restriction explicit.

## Projection window

- Preserve the Windows-style title bar and right-aligned window controls.
- Preserve the right-side vertical Android control toolbar.
- Keep the title bar above the projected frame and the Android toolbar outside
  the frame on its right; neither area may cover the phone image.
- Keep the Windows minimize, maximize/restore, and close behavior instead of
  macOS/iOS-style traffic-light controls.
- Preserve back, home, recent apps, display power, volume, fullscreen, and pin.
- Preserve capture-only rotation without changing the phone orientation.
- Recalculate the projected viewport, mouse/touch coordinates, mapping overlay,
  and toolbar placement after 0/90/180/270-degree capture rotation.
- Preserve keyboard/mouse mapping, FPS view, fire, and 360-degree drag.
- Preserve clean return to the device page when projection closes.
- A second projection in the same process must reuse/recreate its resources
  cleanly without occupied ports, an endless loading spinner, or a stale window.

## Mapping management and editor

- List, create, copy, rename, delete, import, export, enable, and restore mappings.
- Preserve the live device background and coordinate conversion.
- Preserve dragging, copying, deleting, and per-button size controls.
- Preserve random offset settings and algorithms.
- Support SingleTap, RepeatTap, MultipleTap, Swipe, DirectionPad,
  MouseCastSpell, PadCastSpell, CancelCast, Observation, FPS, Fire, RawInput,
  and Script mappings.
- Preserve pointer-id allocation and validation diagnostics.
- Preserve mapping activation and hot reload.

## Script system

- Preserve standalone script mappings and before/after hooks.
- Preserve syntax validation, diagnostics, editor assistance, and execution.
- Never save invalid scripts silently.

## Settings

- Preserve every setting that currently changes real runtime behavior.
- Preserve resolution, FPS, bitrate, codec, orientation, input, display, audio,
  stay-awake, timeout, mapping opacity, and data-directory operations.
- Do not add placeholder controls that have no implementation.

## Migration rule

Each page is implemented against direct Rust services and verified before its
browser equivalent is disabled. The web server, Axios calls, and WebSocket
state bridge are removed only after all checklist items pass in the native UI.
