import { parseBoundaryFiles } from "./boundaryParser";
self.onmessage = async (event: MessageEvent<File[]>) => {
  try { self.postMessage({ geometry: await parseBoundaryFiles(event.data) }); }
  catch (error) { self.postMessage({ error: error instanceof Error ? error.message : String(error) }); }
};
