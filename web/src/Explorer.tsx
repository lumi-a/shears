import { createEffect, createSignal, For, Show, untrack } from "solid-js";
import {
  ItemDetailView,
  LinkText,
  RecipeDetailView,
  type Navigation,
  type Target,
} from "./Details";
import type { ItemDetail, Links, RecipeDetail, SessionClient } from "./session";
import { Split } from "./Split";

// How many search results to show at once.
const SHOWN_RESULTS = 200;
// How many recipes or slot members a detail view lists at once.
const SHOWN_PER_LIST = 50;

type Mode = "items" | "recipes";
const MODES: Mode[] = ["items", "recipes"];

// What a request to the Session came back with or why it failed.
type Outcome<T> = { value: T } | { error: string };

const messageOf = (caught: unknown) =>
  caught instanceof Error ? caught.message : String(caught);

// Runs `request` whenever the signals it reads change, keeping only the newest
// answer.
function createLatest<T>(request: () => Promise<T> | undefined) {
  const [outcome, setOutcome] = createSignal<Outcome<T>>();
  let latest = 0;
  createEffect(() => {
    const current = ++latest;
    const pending = request();
    if (pending === undefined) {
      setOutcome(undefined);
      return;
    }
    pending.then(
      (value) => {
        if (current === latest) setOutcome({ value });
      },
      (caught: unknown) => {
        if (current === latest) setOutcome({ error: messageOf(caught) });
      },
    );
  });
  return outcome;
}

const valueOf = <T,>(outcome: Outcome<T> | undefined) =>
  outcome !== undefined && "value" in outcome ? outcome.value : undefined;
const errorOf = <T,>(outcome: Outcome<T> | undefined) =>
  outcome !== undefined && "error" in outcome ? outcome.error : undefined;

// Searches items or recipes as you type, and shows the details of what's
// clicked, with a history to go back and forth in. The query is a selector
// like a rule's filter, or plain text as one id or /regex/.
export function Explorer(props: {
  session: SessionClient;
  // Changes whenever the graph or the analysis did.
  revision: unknown;
  // Something to open from outside, e.g. an id clicked in the Report pane.
  request: { target: Target } | undefined;
}) {
  const [query, setQuery] = createSignal("");
  const [mode, setMode] = createSignal<Mode>("items");
  const [history, setHistory] = createSignal<Target[]>([]);
  const [position, setPosition] = createSignal(-1);
  const target = () => history()[position()];

  const navigation: Navigation = {
    open: (next) => {
      const kept = history().slice(0, position() + 1);
      setHistory([...kept, next]);
      setPosition(kept.length);
    },
    search: (text, recipes) => {
      setQuery(text);
      setMode(recipes ? "recipes" : "items");
    },
  };

  createEffect(() => {
    const request = props.request;
    // Only the request is what this reacts to, not the history open() reads.
    if (request !== undefined) untrack(() => navigation.open(request.target));
  });

  const search = createLatest(() => {
    void props.revision;
    return props.session.search(query(), mode() === "recipes", SHOWN_RESULTS);
  });

  const detail = createLatest<{ item: ItemDetail } | { recipe: RecipeDetail }>(
    () => {
      void props.revision;
      const shown = target();
      if (shown === undefined) return undefined;
      return shown.kind === "item"
        ? props.session
            .item(shown.id, SHOWN_PER_LIST)
            .then((item) => ({ item }))
        : props.session
            .recipe(shown.id, SHOWN_PER_LIST)
            .then((recipe) => ({ recipe }));
    },
  );

  const results = (
    <div class="explorer-results">
      <Show when={errorOf(search())}>
        {(message) => <p class="error">{message()}</p>}
      </Show>
      <Show when={valueOf<Links>(search())}>
        {(found) => (
          <>
            <p class="hint">
              {found().total.toLocaleString()} {mode()}
              <Show when={found().total > found().shown.length}>
                , showing the first {found().shown.length}
              </Show>
            </p>
            <ul class="links">
              <For each={found().shown}>
                {(link) => (
                  <li>
                    <button
                      type="button"
                      class="link"
                      aria-current={
                        target()?.kind === link.kind && target()?.id === link.id
                          ? "true"
                          : undefined
                      }
                      onClick={() => navigation.open(link)}
                    >
                      <LinkText link={link} />
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </>
        )}
      </Show>
    </div>
  );

  const details = (
    <div class="explorer-detail">
      <div class="history">
        <button
          type="button"
          disabled={position() <= 0}
          onClick={() => setPosition(position() - 1)}
          aria-label="Back"
        >
          ←
        </button>
        <button
          type="button"
          disabled={position() >= history().length - 1}
          onClick={() => setPosition(position() + 1)}
          aria-label="Forward"
        >
          →
        </button>
      </div>
      <div class="detail-body">
        <Show
          when={target() !== undefined}
          fallback={<p class="hint">Click a result to see its details.</p>}
        >
          <Show when={errorOf(detail())}>
            {(message) => <p class="error">{message()}</p>}
          </Show>
          <Show when={valueOf(detail())}>
            {(shown) => {
              const current = shown();
              return "item" in current ? (
                <ItemDetailView detail={current.item} navigation={navigation} />
              ) : (
                <RecipeDetailView
                  detail={current.recipe}
                  navigation={navigation}
                />
              );
            }}
          </Show>
        </Show>
      </div>
    </div>
  );

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
      <Split
        direction="columns"
        storageKey="shears.split.explorer"
        initial={0.4}
        first={results}
        second={details}
      />
    </div>
  );
}
