// Runs the wasm converter off the main thread so large packs do not freeze the page.
import init, { convert, repair, supportedVersions } from "./pkg/packconv_wasm.js";

const ready = init().then(() => {
  postMessage({ type: "ready", versions: supportedVersions() });
});
ready.catch((error) => postMessage({ type: "fatal", message: String(error) }));

onmessage = async ({ data }) => {
  await ready;
  try {
    const input = new Uint8Array(data.buffer);
    const result = data.mode === "repair" ? repair(input) : convert(input, data.to, data.from || undefined);
    const zip = result.zip;
    const report = JSON.parse(result.report);
    result.free();
    postMessage({ type: "done", zip, report }, [zip.buffer]);
  } catch (error) {
    postMessage({ type: "error", message: error instanceof Error ? error.message : String(error) });
  }
};
