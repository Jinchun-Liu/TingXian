/** Validate a self-contained GLB before asking the renderer to load it. No upload. */
export const MAX_GLB_BYTES = 64 * 1024 * 1024;
export function validateLocalGlb(buffer: ArrayBuffer): void {
  if (buffer.byteLength < 20) throw new Error("形象文件不是完整的 GLB 2.0 格式。");
  if (buffer.byteLength > MAX_GLB_BYTES) throw new Error("请选择不超过 64 MB 的 GLB 文件。");
  const view = new DataView(buffer);
  if (view.getUint32(0, true) !== 0x46546c67 || view.getUint32(4, true) !== 2 || view.getUint32(8, true) !== buffer.byteLength) {
    throw new Error("形象文件不是完整的 GLB 2.0 格式。");
  }
  const jsonLength = view.getUint32(12, true);
  if (view.getUint32(16, true) !== 0x4e4f534a || jsonLength % 4 || jsonLength > buffer.byteLength - 20) throw new Error("GLB 形象描述损坏。");
  for (let offset = 20 + jsonLength; offset < buffer.byteLength;) {
    if (buffer.byteLength - offset < 8) throw new Error("GLB 数据块不完整。");
    const length = view.getUint32(offset, true);
    if (length % 4 || length > buffer.byteLength - offset - 8) throw new Error("GLB 数据块不完整。");
    offset += 8 + length;
  }
  let json: { asset?: { version?: string }; buffers?: { uri?: string }[]; images?: { uri?: string }[] };
  try { json = JSON.parse(new TextDecoder().decode(new Uint8Array(buffer, 20, jsonLength))); }
  catch { throw new Error("无法读取 GLB 形象描述。"); }
  if (!json || json.asset?.version !== "2.0") throw new Error("形象需要 glTF 2.0 数据。");
  if ((json.buffers && !Array.isArray(json.buffers)) || (json.images && !Array.isArray(json.images))) throw new Error("GLB 资源列表损坏。");
  for (const entry of [...(json.buffers ?? []), ...(json.images ?? [])]) {
    if (!entry || (entry.uri !== undefined && (typeof entry.uri !== "string" || !entry.uri.startsWith("data:")))) throw new Error("请将贴图和模型打包在同一个 GLB 中；预览不下载外部素材。");
  }
}
