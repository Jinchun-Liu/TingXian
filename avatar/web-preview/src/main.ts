import "./style.css";
import { AvatarRenderer, type ControlFrame } from "./vendor/TalkingAvatarRenderer";
import { MAX_GLB_BYTES, validateLocalGlb } from "./local-glb";

const element = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const controls = element<HTMLFieldSetElement>("controls");
const status = element("status"), error = element("error"), diagnostics = element("diagnostics");
const avatar = element<HTMLSelectElement>("avatar"), mouth = element<HTMLSelectElement>("mouth");
const pause = element<HTMLButtonElement>("pause"), measure = element<HTMLButtonElement>("measure");
const ranges = ["yaw", "pitch", "gaze-x", "gaze-y"].map(id => element<HTMLInputElement>(id));
const closed = element<HTMLInputElement>("eyes-closed");
let paused = matchMedia("(prefers-reduced-motion: reduce)").matches;
let generation = 0, blob: string | undefined;
let loaded = false, loadedChoice = "male", unavailable = false;
let measurementTimer: ReturnType<typeof setTimeout> | undefined;
let loadTimer: ReturnType<typeof setTimeout> | undefined;
let state: ControlFrame = { visemes: {}, mode: "listening", gazeX: 0, gazeY: 0, pitch: 0, yaw: 0 };
let activeMeasurement: { start: number; rows: { at: number; submitMs: number; width: number; height: number }[] } | undefined;
const measurement = element("measurement");
const percentile = (values: number[], p: number) => [...values].sort((a,b) => a-b)[Math.min(values.length - 1, Math.floor(values.length * p))];

function cancelMeasurement(reason: string) {
  if (!activeMeasurement) return;
  activeMeasurement = undefined;
  clearTimeout(measurementTimer);
  measurement.textContent = reason;
  measure.disabled = controls.disabled || paused;
}
let renderer: AvatarRenderer;
try { renderer = new AvatarRenderer(element("viewport"), () => ({ ...state, blink: closed.checked ? 1 : paused ? 0 : undefined }), row => {
  const run = activeMeasurement;
  if (!run) return;
  if (run.rows.length && (row.width !== run.rows[0].width || row.height !== run.rows[0].height)) {
    cancelMeasurement("测量已中断：预览尺寸变化"); return;
  }
  run.rows.push(row);
  if (row.at - run.start < 10000) return;
  if (run.rows.length < 2 || row.at <= run.rows[0].at) {
    cancelMeasurement("有效帧不足，请重新检查预览"); return;
  }
  const intervals = run.rows.slice(1).map((value, index) => value.at - run.rows[index].at);
  const seconds = (run.rows.at(-1)!.at - run.rows[0].at) / 1000;
  const report = { scope: "browser_render_submission_cadence", frames: run.rows.length, seconds,
    submission_fps: intervals.length / seconds, interval_p95_ms: percentile(intervals, .95),
    cpu_submit_p95_ms: percentile(run.rows.map(r => r.submitMs), .95),
    width: row.width, height: row.height, virtual_output: false, capture: false, audio: false,
    gpu_completion_timing: false, capabilities: renderer.capabilities, intervals_ms: intervals };
  diagnostics.textContent = JSON.stringify(report, null, 2);
  measurement.textContent = `绘制提交 ${report.submission_fps.toFixed(1)} 次/秒 · P95 间隔 ${report.interval_p95_ms.toFixed(1)} ms`;
  activeMeasurement = undefined;
  clearTimeout(measurementTimer);
  measure.disabled = paused;
}); } catch (reason) {
  unavailable = true;
  status.textContent = "预览无法启动";
  error.textContent = `无法创建图形预览：${reason instanceof Error ? reason.message : String(reason)}`;
  error.hidden = false;
  throw reason;
}
renderer.setPaused(paused);
pause.textContent = paused ? "继续动作" : "暂停动作";

function update() {
  for (const input of ranges) element(`${input.id}-value`).textContent = input.value + (input.id === "yaw" || input.id === "pitch" ? "°" : "");
  state = { ...state, yaw: +ranges[0].value, pitch: +ranges[1].value,
    gazeX: +ranges[2].value, gazeY: +ranges[3].value,
    visemes: mouth.value === "sil" ? {} : { [mouth.value]: 1 } };
  renderer.redraw();
}
for (const input of [...ranges, closed, mouth]) input.addEventListener("input", update);
element("reset").addEventListener("click", () => {
  for (const input of ranges) input.value = "0";
  mouth.value = "sil"; closed.checked = false; update();
});
pause.addEventListener("click", () => {
  paused = !paused; renderer.setPaused(paused);
  cancelMeasurement("测量已中断：动作暂停");
  pause.textContent = paused ? "继续动作" : "暂停动作";
  status.textContent = paused ? "已暂停" : "本地预览";
  measure.disabled = paused; update();
});
measure.addEventListener("click", () => {
  if (paused || document.hidden || controls.disabled || unavailable) return;
  activeMeasurement = { start: performance.now(), rows: [] };
  measure.disabled = true; measurement.textContent = "正在测量，请保持预览可见…";
  diagnostics.textContent = "";
  measurementTimer = setTimeout(() => cancelMeasurement("测量超时，请重新检查预览"), 14000);
});
document.addEventListener("visibilitychange", () => {
  if (document.hidden) cancelMeasurement("测量已中断：预览不在前台");
});

function restoreControls() {
  controls.disabled = !loaded || unavailable;
  measure.disabled = controls.disabled || paused;
  ranges[0].disabled = ranges[1].disabled = !renderer.capabilities.head;
  ranges[2].disabled = ranges[3].disabled = !renderer.capabilities.gaze;
  closed.disabled = !renderer.capabilities.blink; mouth.disabled = !renderer.capabilities.mouth;
}
function fail(reason: unknown) {
  clearTimeout(loadTimer);
  status.textContent = loaded ? "保留上一形象" : "形象加载失败";
  avatar.value = loadedChoice;
  restoreControls();
  error.textContent = reason instanceof Error ? reason.message : String(reason); error.hidden = false;
}
function beginLoad() {
  const token = ++generation; renderer.cancelPendingLoad();
  clearTimeout(loadTimer);
  cancelMeasurement("测量已中断：形象切换");
  measurement.textContent = "尚未测量";
  controls.disabled = measure.disabled = true; error.hidden = true;
  status.textContent = "正在加载形象…";
  loadTimer = setTimeout(() => {
    if (token !== generation) return;
    generation += 1; renderer.cancelPendingLoad(); fail(new Error("形象加载超时，可重新选择形象。"));
  }, 20000);
  return token;
}
async function load(url: string, name: string, choice: string, token: number) {
  try {
    await renderer.load(url);
    if (token !== generation) return;
    clearTimeout(loadTimer);
    loaded = true; loadedChoice = choice;
    if (choice === "local" && !avatar.querySelector('option[value="local"]')) avatar.add(new Option("本地形象", "local"));
    avatar.value = choice;
    const previous = blob;
    blob = url.startsWith("blob:") ? url : undefined;
    if (previous) URL.revokeObjectURL(previous);
    element("model-name").textContent = name;
    status.textContent = paused ? "已暂停" : "本地预览";
    restoreControls();
    diagnostics.textContent = JSON.stringify({ capabilities: renderer.capabilities, capture: false, audio: false }, null, 2);
    update();
  } catch (reason) {
    if (token !== generation) return;
    fail(reason);
  } finally {
    if (url.startsWith("blob:") && blob !== url) URL.revokeObjectURL(url);
  }
}
avatar.addEventListener("change", () => {
  if (!["male", "female"].includes(avatar.value)) { avatar.value = loadedChoice; return; }
  void load(`/fixtures/${avatar.value}.glb`, avatar.selectedOptions[0].textContent ?? "示例形象", avatar.value, beginLoad());
});
element<HTMLInputElement>("model-file").addEventListener("change", async event => {
  const input = event.currentTarget as HTMLInputElement;
  const file = input.files?.[0]; if (!file) return;
  const token = beginLoad();
  try {
    if (file.size > MAX_GLB_BYTES) throw new Error("请选择不超过 64 MB 的 GLB 文件。");
    const buffer = await file.arrayBuffer(); validateLocalGlb(buffer);
    if (token !== generation) return;
    const url = URL.createObjectURL(new Blob([buffer], { type: "model/gltf-binary" }));
    await load(url, file.name, "local", token);
  } catch (reason) {
    if (token === generation) fail(reason);
  } finally { input.value = ""; }
});
element("viewport").addEventListener("webglcontextlost", () => {
  unavailable = true; renderer.setPaused(true); generation += 1; renderer.cancelPendingLoad();
  cancelMeasurement("测量已中断：图形上下文丢失");
  fail(new Error("图形上下文已丢失，请刷新页面恢复预览。"));
  status.textContent = "预览已停止";
}, true);
window.addEventListener("pagehide", event => {
  cancelMeasurement("测量已中断：离开预览");
  if (event.persisted) renderer.setPaused(true);
  else { clearTimeout(loadTimer); renderer.dispose(); if (blob) URL.revokeObjectURL(blob); }
});
window.addEventListener("pageshow", event => { if (event.persisted && !unavailable) renderer.setPaused(paused); });
void load("/fixtures/male.glb", "示例 · 男性", "male", beginLoad());
