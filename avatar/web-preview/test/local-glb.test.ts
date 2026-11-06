import { test } from "node:test";
import assert from "node:assert/strict";
import { validateLocalGlb } from "../src/local-glb";

function glb(value: unknown, tail = new Uint8Array()): ArrayBuffer {
  const encoded = new TextEncoder().encode(JSON.stringify(value));
  const length = Math.ceil(encoded.length / 4) * 4;
  const buffer = new ArrayBuffer(20 + length + tail.length);
  const view = new DataView(buffer);
  [0x46546c67, 2, buffer.byteLength, length, 0x4e4f534a].forEach((n, i) => view.setUint32(i * 4, n, true));
  new Uint8Array(buffer, 20, length).fill(32);
  new Uint8Array(buffer, 20, encoded.length).set(encoded);
  new Uint8Array(buffer, 20 + length).set(tail);
  return buffer;
}
test("self-contained GLB with embedded binary or data image is accepted", () => {
  const tail = new Uint8Array(12); const view = new DataView(tail.buffer);
  view.setUint32(0, 4, true); view.setUint32(4, 0x004e4942, true);
  validateLocalGlb(glb({asset:{version:"2.0"}, buffers:[{byteLength:4}], images:[{uri:"data:image/png;base64,AA=="}]}, tail));
});
test("external and relative image/buffer references fail before loader fetch", () => {
  for (const uri of ["https://example.com/a.png", "../a.bin", "/a.bin", "file:///D:/a.bin", "blob:test", ""]) {
    for (const key of ["images", "buffers"]) assert.throws(() => validateLocalGlb(glb({asset:{version:"2.0"}, [key]:[{uri}]})), /同一个 GLB/);
  }
});
test("truncated header and binary chunks are rejected", () => {
  assert.throws(() => validateLocalGlb(new ArrayBuffer(10)));
  const truncated = glb({asset:{version:"2.0"}}, new Uint8Array(4));
  assert.throws(() => validateLocalGlb(truncated), /不完整/);
  const tail = new Uint8Array(8); new DataView(tail.buffer).setUint32(0, 100, true);
  assert.throws(() => validateLocalGlb(glb({asset:{version:"2.0"}}, tail)), /不完整/);
});
test("declared length and version must match the file", () => {
  const buffer = glb({asset:{version:"2.0"}});
  new DataView(buffer).setUint32(8, buffer.byteLength + 4, true);
  assert.throws(() => validateLocalGlb(buffer), /完整/);
  assert.throws(() => validateLocalGlb(glb({asset:{version:"1.0"}})), /2.0/);
});
test("malformed asset or resource structures fail with usable errors", () => {
  for (const value of [null, {}, {asset:{version:"2.0"},images:{}}, {asset:{version:"2.0"},images:[null]}, {asset:{version:"2.0"},buffers:[{uri:3}]}]) {
    assert.throws(() => validateLocalGlb(glb(value)), Error);
  }
});
