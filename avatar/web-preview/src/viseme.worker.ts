import { verifiedVisemeModel, analyzePcm } from "./viseme-analysis";
import { readPcmWav } from "./pcm-wav";

// Worker termination is the cancellation boundary. No WebAudio or media API.
self.onmessage = async ({data}: MessageEvent<{buffer:ArrayBuffer}>) => {
  try {
    const clip=readPcmWav(data.buffer);
    const response=await fetch("/fixtures/viseme-model.bin",{credentials:"omit",cache:"force-cache"});
    if (!response.ok) throw new Error("无法读取本地口型模型。");
    const model=await verifiedVisemeModel(await response.arrayBuffer());
    self.postMessage({result:analyzePcm(model,clip)});
  } catch (error) { self.postMessage({error:error instanceof Error?error.message:String(error)}); }
};
