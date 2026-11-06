import { Euler, MathUtils, Quaternion } from "three";

/** Preview yaw/pitch stay in world axes even when a rig rotates its bones. */
export function worldHeadPose(rest: Quaternion, parent: Quaternion, pitch: number, yaw: number): Quaternion {
  const bounded = (value: number, limit: number) => Number.isFinite(value) ? MathUtils.clamp(value, -limit, limit) : 0;
  const delta = new Quaternion().setFromEuler(new Euler(
    MathUtils.degToRad(bounded(pitch, 8)), MathUtils.degToRad(bounded(yaw, 12)), 0,
  ));
  return parent.clone().invert().multiply(delta).multiply(parent).multiply(rest);
}
