// Holds the one Session off the main thread.
//
// Ingesting a big export takes ~1 second, and the page should stay responsive meanwhile.

import init, { Session } from "./wasm-pkg/shears_wasm";
import type {
  Analysis,
  ItemDetail,
  Links,
  RecipeDetail,
  Kind,
  ParseReport,
  Request,
  Response,
  Results,
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

function current(): Session {
  if (session === undefined) {
    throw new Error("no export uploaded yet");
  }
  return session;
}

function handle(request: Request): Results[Kind] {
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
      const reingested = current();
      reingested.reingest(request.shapesToml);
      return infoOf(reingested);
    }
    case "analyze":
      return JSON.parse(current().analyze_json(request.rulesToml)) as Analysis;
    case "search":
      return JSON.parse(
        current().search_json(request.query, request.recipes, request.limit),
      ) as Links;
    case "item":
      return JSON.parse(
        current().item_json(request.id, request.limit),
      ) as ItemDetail;
    case "recipe":
      return JSON.parse(
        current().recipe_json(request.id, request.limit),
      ) as RecipeDetail;
  }
}

self.onmessage = async (event: MessageEvent<Request>) => {
  await ready;
  const request = event.data;
  let response: Response;
  try {
    response = { seq: request.seq, ok: true, result: handle(request) };
  } catch (error) {
    // Only the message reaches the page, so the stack (if any) goes here.
    console.error(error);
    // Errors from Rust arrive as plain strings for now.
    response = { seq: request.seq, ok: false, error: String(error) };
  }
  postMessage(response);
};
