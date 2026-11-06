import { test } from "node:test";
import assert from "node:assert/strict";
import { Euler, Quaternion, Vector3 } from "three";
import { worldHeadPose } from "../src/world-head";

const parent = new Quaternion().setFromEuler(new Euler(0, 0, Math.PI / 2));
const identity = new Quaternion();
const close = (actual: Vector3, expected: Vector3) => assert.ok(actual.distanceTo(expected) < 1e-10);
test("yaw turns forward toward world X even with a 90 degree rotated parent", () => {
  const local = worldHeadPose(identity, parent, 0, 12);
  close(new Vector3(0, 0, 1).applyQuaternion(parent.clone().multiply(local)), new Vector3(Math.sin(Math.PI/15), 0, Math.cos(Math.PI/15)));
});
test("pitch stays on world X and clamps oversized inputs", () => {
  const local = worldHeadPose(identity, parent, 100, 0);
  close(new Vector3(0, 0, 1).applyQuaternion(parent.clone().multiply(local)), new Vector3(0, -Math.sin(Math.PI*8/180), Math.cos(Math.PI*8/180)));
});
test("neutral or invalid controls restore rest without changing the cached pose", () => {
  const rest = new Quaternion().setFromEuler(new Euler(.2, -.1, .4));
  const original = rest.clone(); const originalParent = parent.clone();
  for (const [pitch, yaw] of [[0,0], [NaN,Infinity]]) assert.ok(1-Math.abs(worldHeadPose(rest,parent,pitch,yaw).dot(rest)) < 1e-10);
  assert.deepEqual(rest, original); assert.deepEqual(parent, originalParent);
});
