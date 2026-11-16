// Messages between the page (session.ts) and the worker holding the Session
// (session.worker.ts).

// Mirrors shears_core::ingest::Diagnostic, as serialized by Session::report_json.
export interface Diagnostic {
  about: string;
  message: string;
}

// Mirrors shears_core::ingest::ParseReport, as serialized by Session::report_json.
export interface ParseReport {
  diagnostics: Diagnostic[];
  recipe_type_counts: Record<string, number>;
  registered_but_unreferenced: string[];
}

// Everything the page knows about the current Session.
export interface SessionInfo {
  rootDirectory: string;
  fileCount: number;
  itemCount: number;
  recipeCount: number;
  report: ParseReport;
}

// Mirrors shears_wasm::Analysis, as serialized by Session::analyze_json.
export interface Analysis {
  required_count: number;
  transitively_required_item_count: number;
  transitively_required_recipe_count: number;
  warnings: string[];
}

export type RequestBody =
  | { kind: "open"; bytes: ArrayBuffer; shapesToml: string }
  | { kind: "reingest"; shapesToml: string }
  | { kind: "analyze"; rulesToml: string };

export type Kind = RequestBody["kind"];

// What each kind of request answers with.
export interface Results {
  open: SessionInfo;
  reingest: SessionInfo;
  analyze: Analysis;
}

export type Request = RequestBody & { id: number };

export type Response =
  | { id: number; ok: true; result: Results[Kind] }
  | { id: number; ok: false; error: string };
