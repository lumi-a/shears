// Ready-made exports to try shears on without uploading anything.
//
// The `?url` imports make each file its own static asset that's fetched only when
// its preset is picked.
// They point at the committed files directly rather than copies.

import monifactoryArchive from "../../fixtures/monifactory-0.13.8-normalmode.tar.zst?url";
import vanillaArchive from "../../fixtures/vanilla-1.20.1.tar.zst?url";
import monifactoryParsing from "../../crates/core/src/ingest/shapes.toml?url";

export interface Preset {
  name: string;
  archiveUrl: string;
  // undefined: an empty parsing.toml.
  parsingUrl: string | undefined;
}

export const PRESETS: Preset[] = [
  {
    name: "Monifactory 0.13.8",
    archiveUrl: monifactoryArchive,
    parsingUrl: monifactoryParsing,
  },
  {
    name: "Vanilla 1.20.1",
    archiveUrl: vanillaArchive,
    parsingUrl: undefined,
  },
];

async function fetchOk(url: string): Promise<Response> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(
      `fetching ${url}: ${response.status} ${response.statusText}`,
    );
  }
  return response;
}

export interface FetchedPreset {
  archive: Blob;
  parsingToml: string;
}

export async function fetchPreset(preset: Preset): Promise<FetchedPreset> {
  const [archive, parsingToml] = await Promise.all([
    fetchOk(preset.archiveUrl).then((response) => response.blob()),
    preset.parsingUrl === undefined
      ? ""
      : fetchOk(preset.parsingUrl).then((response) => response.text()),
  ]);
  return { archive, parsingToml };
}
