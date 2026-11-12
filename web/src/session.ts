// The page's handle on the Session living in session.worker.ts.
//
// Every call is a message to the worker and answered in order.

import type { RequestBody, Response, SessionInfo } from "./session-protocol";

export type { Diagnostic, ParseReport, SessionInfo } from "./session-protocol";

interface Pending {
  resolve: (info: SessionInfo) => void;
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
        pending?.resolve(response.info);
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

  #send(body: RequestBody, transfer: Transferable[] = []) {
    const id = this.#nextId++;
    return new Promise<SessionInfo>((resolve, reject) => {
      this.#pending.set(id, { resolve, reject });
      this.#worker.postMessage({ ...body, id }, transfer);
    });
  }
}
