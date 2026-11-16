// The page's handle on the Session living in session.worker.ts.
//
// Every call is a message to the worker and answered in order.

import type {
  Analysis,
  Kind,
  RequestBody,
  Response,
  Results,
  SessionInfo,
} from "./session-protocol";

export type {
  Analysis,
  AnalysisDiff,
  Diagnostic,
  ParseReport,
  SessionInfo,
} from "./session-protocol";

interface Pending {
  resolve: (result: Results[Kind]) => void;
  reject: (error: Error) => void;
}

export class SessionClient {
  #worker = new Worker(new URL("./session.worker.ts", import.meta.url), {
    type: "module",
  });
  #nextId = 0;
  #pending = new Map<number, Pending>();

  constructor() {
    this.#worker.onmessage = (event: MessageEvent<Response>) => {
      const response = event.data;
      const pending = this.#pending.get(response.id);
      this.#pending.delete(response.id);
      if (response.ok) {
        pending?.resolve(response.result);
      } else {
        pending?.reject(new Error(response.error));
      }
    };
    // Only for failures outside a request's own try/catch, e.g. the worker
    // script not loading at all. Fail everything waiting.
    this.#worker.onerror = (event) => {
      for (const pending of this.#pending.values()) {
        pending.reject(new Error(event.message || "the worker failed"));
      }
      this.#pending.clear();
    };
  }

  // Uploads an export archive and ingests it with shapesToml (parsing.toml).
  // Replaces the current session only if this succeeds.
  async open(archive: Blob, shapesToml: string): Promise<SessionInfo> {
    const bytes = await archive.arrayBuffer();
    // Transferred (not copied) because the page doesn't need the bytes afterwards.
    return this.#send({ kind: "open", bytes, shapesToml }, [bytes]);
  }

  // Ingests the current export again with a new parsing.toml. Keeps the old
  // result if shapesToml doesn't parse.
  reingest(shapesToml: string): Promise<SessionInfo> {
    return this.#send({ kind: "reingest", shapesToml });
  }

  // Applies rulesToml (rules.toml) to the current graph. Fails if it doesn't
  // parse.
  analyze(rulesToml: string): Promise<Analysis> {
    return this.#send({ kind: "analyze", rulesToml });
  }

  // The worker answers a request of kind K with Results[K]. TypeScript can't
  // see that across postMessage, hence the cast.
  #send<K extends Kind>(
    body: Extract<RequestBody, { kind: K }>,
    transfer: Transferable[] = [],
  ): Promise<Results[K]> {
    const id = this.#nextId++;
    return new Promise<Results[K]>((resolve, reject) => {
      this.#pending.set(id, {
        resolve: resolve as (result: Results[Kind]) => void,
        reject,
      });
      this.#worker.postMessage({ ...body, id }, transfer);
    });
  }
}
