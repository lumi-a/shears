import { createEffect, createSignal, For, Show } from "solid-js";
import type { Link, Links, SessionClient } from "./session";

// How many search results to show at once.
const SHOWN_RESULTS = 200;

type Mode = "items" | "recipes";
const MODES: Mode[] = ["items", "recipes"];

// One search's outcome.
type Outcome = { links: Links } | { error: string };

// An item or recipe id. This is struck through if the latest analysis found it not needed.
export function LinkText(props: { link: Link }) {
  return (
    <span
      class="link-text"
      classList={{ dead: props.link.live === false }}
      title={
        props.link.live === null
          ? undefined
          : props.link.live
            ? "needed"
            : "not needed"
      }
    >
      {props.link.id}
    </span>
  );
}

// Searches items or recipes as you type. The query is a selector like a
// rule's filter, or plain text as one id or /regex/.
export function Explorer(props: {
  session: SessionClient;
  // Changes whenever the graph or the analysis did.
  revision: unknown;
}) {
  const [query, setQuery] = createSignal("");
  const [mode, setMode] = createSignal<Mode>("items");
  const [outcome, setOutcome] = createSignal<Outcome>();

  // Every search gets a number so a slow answer to an old query can't
  // replace a newer one.
  let latest = 0;
  createEffect(() => {
    const current = ++latest;
    const text = query();
    const recipes = mode() === "recipes";
    void props.revision;
    props.session.search(text, recipes, SHOWN_RESULTS).then(
      (links) => {
        if (current === latest) setOutcome({ links });
      },
      (caught: unknown) => {
        if (current === latest) {
          setOutcome({
            error: caught instanceof Error ? caught.message : String(caught),
          });
        }
      },
    );
  });

  const links = () => {
    const current = outcome();
    return current !== undefined && "links" in current
      ? current.links
      : undefined;
  };
  const error = () => {
    const current = outcome();
    return current !== undefined && "error" in current
      ? current.error
      : undefined;
  };

  return (
    <div class="explorer">
      <div class="explorer-search">
        <input
          type="search"
          placeholder={`id, /regex/, or { mod = "gtceu" }`}
          aria-label="Search"
          spellcheck={false}
          value={query()}
          onInput={(event) => setQuery(event.currentTarget.value)}
        />
        <div class="modes" role="radiogroup" aria-label="Search for">
          <For each={MODES}>
            {(name) => (
              <button
                type="button"
                role="radio"
                aria-checked={mode() === name}
                onClick={() => setMode(name)}
              >
                {name}
              </button>
            )}
          </For>
        </div>
      </div>
      <Show when={error()}>
        {(message) => <p class="error">{message()}</p>}
      </Show>
      <Show when={links()}>
        {(found) => (
          <div class="explorer-results">
            <p class="hint">
              {found().total.toLocaleString()} {mode()}
              <Show when={found().total > found().shown.length}>
                , showing the first {found().shown.length}
              </Show>
            </p>
            <ul>
              <For each={found().shown}>
                {(link) => (
                  <li>
                    <LinkText link={link} />
                  </li>
                )}
              </For>
            </ul>
          </div>
        )}
      </Show>
    </div>
  );
}
