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

// Mirrors shears_wasm::Diff. Every list consists of ids and is sorted.
export interface AnalysisDiff {
  items_added: string[];
  items_removed: string[];
  recipes_added: string[];
  recipes_removed: string[];
}

// Mirrors shears_wasm::Analysis, as serialized by Session::analyze_json.
export interface Analysis {
  required_count: number;
  transitively_required_item_count: number;
  transitively_required_recipe_count: number;
  warnings: string[];
  // Against the previous successful analysis, null for the first one.
  diff: AnalysisDiff | null;
}

// A reference to an item or recipe. live: whether it's transitively required
// (an item) or propagates (a recipe), null before the first analysis.
export interface Link {
  id: string;
  live: boolean | null;
}

// Mirrors shears_wasm's SearchResult and details::Links: the first few of a
// list, and how long it is.
export interface Links {
  total: number;
  shown: Link[];
}

// Mirrors shears_wasm::details::WhyStep.
export type WhyStep =
  { via: { recipe: string; output: string } } | { required: string };

// Mirrors shears_wasm::details::ItemDetail.
export interface ItemDetail {
  id: string;
  fluid: boolean;
  tags: string[];
  required: string | null;
  live: boolean | null;
  why: WhyStep[];
  produced_by: Links;
  used_in: Links;
}

// Mirrors shears_wasm::details::RecipeDetail.
export interface RecipeDetail {
  id: string;
  recipe_type: string;
  eu_per_tick: number | null;
  ticks: number | null;
  live: boolean | null;
  inputs: {
    tag: string | null;
    count: number;
    chance: number;
    members: Links;
  }[];
  outputs: {
    item: Link;
    count: number;
    chance: number;
    edge: "propagate" | "inert" | "dropped" | null;
    edge_cause: string | null;
  }[];
}

export type RequestBody =
  | { kind: "open"; bytes: ArrayBuffer; shapesToml: string }
  | { kind: "reingest"; shapesToml: string }
  | { kind: "analyze"; rulesToml: string }
  | { kind: "search"; query: string; recipes: boolean; limit: number }
  | { kind: "item"; id: string; limit: number }
  | { kind: "recipe"; id: string; limit: number };

export type Kind = RequestBody["kind"];

// What each kind of request answers with.
export interface Results {
  open: SessionInfo;
  reingest: SessionInfo;
  analyze: Analysis;
  search: Links;
  item: ItemDetail;
  recipe: RecipeDetail;
}

export type Request = RequestBody & { id: number };

export type Response =
  | { id: number; ok: true; result: Results[Kind] }
  | { id: number; ok: false; error: string };
