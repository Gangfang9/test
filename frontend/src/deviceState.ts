import { requestGet, type AdbDevice, type ControlledDevice } from "./utils";
import { setAdbDevices, setControlledDevices, setProjection } from "./store/other";
import type { AppDispatch } from "./store/store";

export interface ProjectionStatus {
  phase: "idle" | "starting" | "streaming" | "stopping" | "failed";
  scid: string | null;
  message: string;
}
let generation = 0;
let busy = false;
export function beginDeviceOperation() {
  if (busy) return false;
  busy = true;
  generation += 1;
  return true;
}
export function endDeviceOperation() {
  busy = false;
  generation += 1;
}
export async function syncDeviceState(dispatch: AppDispatch, force = false) {
  if (busy && !force) return null;
  const current = ++generation;
  const result = await requestGet<{
    controlled_devices: ControlledDevice[];
    adb_devices: AdbDevice[];
    projection?: ProjectionStatus;
  }>("/api/device/device_list");
  if (current !== generation) return null;
  dispatch(setControlledDevices(result.data.controlled_devices));
  dispatch(setAdbDevices(result.data.adb_devices));
  if (result.data.projection) dispatch(setProjection(result.data.projection));
  return result;
}
