import { defineConfig, type Plugin } from "vite";
import { readFileSync, realpathSync } from "node:fs";
import { resolve } from "node:path";
import { execFileSync } from "node:child_process";

const revision = "d3e4dcf6609d733097466eb17b3fa63fc720140a";
function localFixtures(): Plugin {
  const source = process.env.TINGXIAN_AVATAR_SOURCE;
  if (!source) throw new Error("Set TINGXIAN_AVATAR_SOURCE to the pinned talking-avatar checkout.");
  const root = realpathSync(source);
  const actual = execFileSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
  const changed = execFileSync("git", ["-C", root, "status", "--porcelain", "--", "avatars", "model"], { encoding: "utf8" }).trim();
  if (actual !== revision || changed) throw new Error("Avatar fixtures are not the reviewed clean revision.");
  const files = new Map([
    ["fixtures/male.glb", readFileSync(resolve(root, "avatars/male.glb"))],
    ["fixtures/female.glb", readFileSync(resolve(root, "avatars/female.glb"))],
    ["fixtures/LICENSE.txt", readFileSync(resolve(root, "avatars/LICENSE"))],
    ["fixtures/viseme-model.bin", readFileSync(resolve(root, "model/viseme-model.bin"))],
    ["tingxian.png", readFileSync(new URL("../../frontend/public/icon_128x128.png", import.meta.url))],
  ]);
  for (const name of ["TalkingAvatar-MIT.txt", "Rocketbox-MIT.txt", "Three-MIT.txt", "Meshoptimizer-MIT.txt"]) {
    files.set(`licenses/${name}`, readFileSync(new URL(`./licenses/${name}`, import.meta.url)));
  }
  return {
    name: "tingxian-local-fixtures",
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const path = (request.url ?? "").split("?")[0].replace(/^\//, "");
        const bytes = files.get(path);
        if (!bytes) return next();
        response.setHeader("Content-Type", path.endsWith(".glb") ? "model/gltf-binary" : path.endsWith(".bin") ? "application/octet-stream" : path.endsWith(".png") ? "image/png" : "text/plain; charset=utf-8");
        response.end(bytes);
      });
    },
    generateBundle() {
      for (const [fileName, source] of files) this.emitFile({ type: "asset", fileName, source });
    },
  };
}
export default defineConfig({
  plugins: [localFixtures()],
  server: { host: "127.0.0.1", port: 19270, strictPort: true, headers: {
    "Permissions-Policy": "camera=(), microphone=(), display-capture=()",
  } },
  build: { emptyOutDir: false },
});
