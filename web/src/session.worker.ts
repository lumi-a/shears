// Holds the one Session off the main thread.
//
// Ingesting a big export takes ~1 second, and the page should stay responsive meanwhile.

import init, { Session } from "./wasm-pkg/shears_wasm";
import type {
  ParseReport,
  Request,
  Response,
  SessionInfo,
} from "./session-protocol";

const ready = init();
let session: Session | undefined;

function infoOf(current: Session): SessionInfo {
  return {
    rootDirectory: current.root_directory,
    fileCount: current.file_count,
    itemCount: current.item_count,
    recipeCount: current.recipe_count,
    report: JSON.parse(current.report_json()) as ParseReport,
  };
}

function handle(request: Request): SessionInfo {
  switch (request.kind) {
    case "open": {
      // Only replace and free the old session once the new one succeeded.
      const opened = Session.open(
        new Uint8Array(request.bytes),
        request.shapesToml,
      );
      session?.free();
      session = opened;
      return infoOf(opened);
    }
    case "reingest": {
      if (session === undefined) {
        throw new Error("no export uploaded yet");
      }
      session.reingest(request.shapesToml);
      return infoOf(session);
    }
  }
}

self.onmessage = async (event: MessageEvent<Request>) => {
  await ready;
  const request = event.data;
  let response: Response;
  try {
    response = { id: request.id, ok: true, info: handle(request) };
  } catch (error) {
    // Errors from Rust arrive as plain strings for now.
    response = { id: request.id, ok: false, error: String(error) };
  }
  postMessage(response);
};
