// Messages between the page (session.ts) and the worker holding the Session
// (session.worker.ts).

// Generated from the Rust types by ts-rs when running `cargo test`, see .cargo/config.toml.
import type { About } from "./generated/About";
import type { Analysis } from "./generated/Analysis";
import type { AnalysisDiff } from "./generated/AnalysisDiff";
import type { Diagnostic } from "./generated/Diagnostic";
import type { ItemDetail } from "./generated/ItemDetail";
import type { Link } from "./generated/Link";
import type { LinkKind } from "./generated/LinkKind";
import type { Links } from "./generated/Links";
import type { ParseReport } from "./generated/ParseReport";
import type { RecipeDetail } from "./generated/RecipeDetail";
import type { SearchResults } from "./generated/SearchResults";
import type { WhyStep } from "./generated/WhyStep";

export type {
  About,
  Analysis,
  AnalysisDiff,
  Diagnostic,
  ItemDetail,
  Link,
  LinkKind,
  Links,
  ParseReport,
  RecipeDetail,
  SearchResults,
  WhyStep,
};

// Everything the page knows about the current Session.
export interface SessionInfo {
  rootDirectory: string;
  fileCount: number;
  itemCount: number;
  recipeCount: number;
  report: ParseReport;
}

export type RequestBody =
  | { kind: "open"; bytes: ArrayBuffer; shapesToml: string }
  | { kind: "reingest"; shapesToml: string }
  | { kind: "analyze"; rulesToml: string }
  | {
      kind: "search";
      query: string;
      items: boolean;
      recipes: boolean;
      limit: number;
    }
  | { kind: "item"; id: string; limit: number }
  | { kind: "recipe"; id: string; limit: number }
  | { kind: "file"; path: string };

export type Kind = RequestBody["kind"];

// What each kind of request answers with.
export interface Results {
  open: SessionInfo;
  reingest: SessionInfo;
  analyze: Analysis;
  search: SearchResults;
  item: ItemDetail;
  recipe: RecipeDetail;
  file: string;
}

// `seq` pairs a response with its request. (Not `id`: that would clash with an item's.)
export type Request = RequestBody & { seq: number };

export type Response =
  | { seq: number; ok: true; result: Results[Kind] }
  | { seq: number; ok: false; error: string };
