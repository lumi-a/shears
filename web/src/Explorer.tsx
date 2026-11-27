import { createEffect, createSignal, For, Show, untrack } from "solid-js";
import {
  ItemDetailView,
  LinkText,
  RecipeDetailView,
  ShowAll,
  type Navigation,
  type Target,
} from "./Details";
import type {
  ItemDetail,
  LinkKind,
  RecipeDetail,
  SearchResults,
  SessionClient,
} from "./session";
import { Split } from "./Split";

// How many search results to show at once.
const SHOWN_RESULTS = 200;
// How many recipes or slot members a detail view lists at once.
const SHOWN_PER_LIST = 50;

// What a request to the Session came back with or why it failed.
type Outcome<T> = { value: T } | { error: string };

const KINDS: LinkKind[] = ["item", "recipe"];

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
// A list limit the page can raise which goes back to `initial` whenever `key`
// changes, e.g. on a new query.
function createLimit(initial: number, key: () => unknown) {
  const [raised, setRaised] = createSignal<{ key: unknown; to: number }>();
  const limit = () => {
    const current = raised();
    return current !== undefined && current.key === key()
      ? current.to
      : initial;
  };
  const raise = (to: number) => setRaised({ key: key(), to });
  return [limit, raise] as const;
}

const errorOf = <T,>(outcome: Outcome<T> | undefined) =>
  outcome !== undefined && "error" in outcome ? outcome.error : undefined;

// Searches items and recipes as you type, and shows the details of what's
// clicked, with a history to go back and forth in. The query is a selector
// like a rule's filter, an id /regex/, or plain text matched fuzzily.
export function Explorer(props: {
  session: SessionClient;
  // Changes whenever the graph or the analysis did.
  revision: unknown;
  // Something to open from outside, e.g. an id clicked in the Report pane.
  request: { target: Target } | undefined;
}) {
  const [query, setQuery] = createSignal("");
  // Which kinds the results list. Both by default.
  const [listed, setListed] = createSignal({ item: true, recipe: true });
  const [history, setHistory] = createSignal<Target[]>([]);
  const [position, setPosition] = createSignal(-1);
  const target = () => history()[position()];
  const [resultLimit, raiseResultLimit] = createLimit(
    SHOWN_RESULTS,
    () => `${listed().item} ${listed().recipe} ${query()}`,
  );
  const [detailLimit, raiseDetailLimit] = createLimit(SHOWN_PER_LIST, target);

  const navigation: Navigation = {
    open: (next) => {
      const kept = history().slice(0, position() + 1);
      setHistory([...kept, next]);
      setPosition(kept.length);
    },
    search: setQuery,
    showAll: raiseDetailLimit,
  };

  createEffect(() => {
    const request = props.request;
    // Only the request is what this reacts to, not the history open() reads.
    if (request !== undefined) untrack(() => navigation.open(request.target));
  });

  const search = createLatest(() => {
    void props.revision;
    return props.session.search(
      query(),
      listed().item,
      listed().recipe,
      resultLimit(),
    );
  });

  const detail = createLatest<{ item: ItemDetail } | { recipe: RecipeDetail }>(
    () => {
      void props.revision;
      const shown = target();
      if (shown === undefined) return undefined;
      return shown.kind === "item"
        ? props.session.item(shown.id, detailLimit()).then((item) => ({ item }))
        : props.session
            .recipe(shown.id, detailLimit())
            .then((recipe) => ({ recipe }));
    },
  );

  // Apart, so each <Show> below only re-renders when it switches between item
  // and recipe, and otherwise passes the new detail on.
  const itemDetail = () => {
    const shown = valueOf(detail());
    return shown !== undefined && "item" in shown ? shown.item : undefined;
  };
  const recipeDetail = () => {
    const shown = valueOf(detail());
    return shown !== undefined && "recipe" in shown ? shown.recipe : undefined;
  };

  const results = (
    <div class="explorer-results">
      <Show when={errorOf(search())}>
        {(message) => <p class="error">{message()}</p>}
      </Show>
      <Show when={valueOf<SearchResults>(search())?.links}>
        {(found) => (
          <>
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
                      onClick={() =>
                        navigation.open({ kind: link.kind, id: link.id })
                      }
                    >
                      <LinkText link={link} />
                    </button>
                  </li>
                )}
              </For>
              <ShowAll
                hidden={found().total - found().shown.length}
                onClick={() => raiseResultLimit(found().total)}
              />
            </ul>
          </>
        )}
      </Show>
    </div>
  );

  const details = (
    <div class="explorer-detail">
      <div class="detail-body">
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
        <Show
          when={target() !== undefined}
          fallback={<p class="hint">Click a result to see its details.</p>}
        >
          <Show when={errorOf(detail())}>
            {(message) => <p class="error">{message()}</p>}
          </Show>
          <Show when={itemDetail()}>
            {(item) => (
              <ItemDetailView detail={item()} navigation={navigation} />
            )}
          </Show>
          <Show when={recipeDetail()}>
            {(recipe) => (
              <RecipeDetailView detail={recipe()} navigation={navigation} />
            )}
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
          placeholder={`iron ingot, /regex/, or { mod = "gtceu" }`}
          aria-label="Search"
          spellcheck={false}
          value={query()}
          onInput={(event) => setQuery(event.currentTarget.value)}
        />
        <div class="kinds" role="group" aria-label="List">
          <For each={KINDS}>
            {(kind) => {
              // null after an error, or if the query isn't for this kind.
              const count = () =>
                valueOf<SearchResults>(search())?.[`${kind}_count`] ?? null;
              return (
                <button
                  type="button"
                  class={kind}
                  aria-pressed={listed()[kind]}
                  disabled={count() === null}
                  title={
                    count() === null
                      ? `The query isn't a selector for ${kind}s`
                      : `List ${kind}s`
                  }
                  onClick={() =>
                    setListed({ ...listed(), [kind]: !listed()[kind] })
                  }
                >
                  {kind}s{" "}
                  <span class="count">{count()?.toLocaleString() ?? "–"}</span>
                </button>
              );
            }}
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
