import { For, Show, type JSX } from "solid-js";
import type { ItemDetail, Link, Links, RecipeDetail } from "./session";

// What the Explorer shows details of.
export type Target = Pick<Link, "kind" | "id">;

// How the details reach back into the Explorer.
export interface Navigation {
  // Shows `target`'s details.
  open: (target: Target) => void;
  // Searches for `query`.
  search: (query: string) => void;
  // Shows every list of the current details in full, up to `total` entries.
  showAll: (total: number) => void;
}

// The last entry of a list that's been cut off.
// Specifies how many more there are and a button to show them all.
export function ShowAll(props: { hidden: number; onClick: () => void }) {
  return (
    <Show when={props.hidden > 0}>
      <li class="more">
        and {props.hidden.toLocaleString()} more.{" "}
        <button type="button" class="link" onClick={() => props.onClick()}>
          Show all
        </button>
      </li>
    </Show>
  );
}

// An item or recipe id, also coloured by which of the two it is.
// This is struck through if the latest analysis found it not needed.
export function LinkText(props: { link: Link }) {
  return (
    <span
      class={`link-text ${props.link.kind}`}
      classList={{ dead: props.link.live === false }}
      title={
        props.link.kind +
        (props.link.live === null
          ? ""
          : props.link.live
            ? ", needed"
            : ", not needed")
      }
    >
      {props.link.id}
    </span>
  );
}

const percent = (chance: number) => `${Math.round(chance * 1000) / 10}%`;

// Whether the latest analysis found it needed, in words.
function Liveness(props: { live: boolean | null; needed: string }) {
  return (
    <Show
      when={props.live !== null}
      fallback={<span class="hint">Not analyzed yet.</span>}
    >
      <span classList={{ dead: props.live === false }}>
        {props.live ? props.needed : `Not ${props.needed.toLowerCase()}`}
      </span>
    </Show>
  );
}

// A clickable id, opening that item's or recipe's details.
function LinkButton(props: { link: Link; navigation: Navigation }) {
  return (
    <button
      type="button"
      class="link"
      onClick={() =>
        props.navigation.open({ kind: props.link.kind, id: props.link.id })
      }
    >
      <LinkText link={props.link} />
    </button>
  );
}

// The links of a list the Session cut off and a button to fetch the rest.
function LinkItems(props: { links: Links; navigation: Navigation }) {
  return (
    <>
      <For each={props.links.shown}>
        {(link) => (
          <li>
            <LinkButton link={link} navigation={props.navigation} />
          </li>
        )}
      </For>
      <ShowAll
        hidden={props.links.total - props.links.shown.length}
        onClick={() => props.navigation.showAll(props.links.total)}
      />
    </>
  );
}

// A capped list of links, with how many there are in its heading.
function LinkList(props: {
  title: string;
  links: Links;
  navigation: Navigation;
}) {
  return (
    <>
      <h4>
        {props.title} ({props.links.total.toLocaleString()})
      </h4>
      <ul class="links">
        <LinkItems links={props.links} navigation={props.navigation} />
      </ul>
    </>
  );
}

export function ItemDetailView(props: {
  detail: ItemDetail;
  navigation: Navigation;
}) {
  return (
    <>
      <h3 class="detail-title item">
        <span class="detail-kind">{props.detail.fluid ? "fluid" : "item"}</span>
        {props.detail.id}
      </h3>
      <p>
        <Liveness live={props.detail.live} needed="Needed" />
        <Show when={props.detail.required}>
          {(cause) => <>, required directly by {cause()}</>}
        </Show>
      </p>

      <Show when={props.detail.why.length > 0}>
        <h4>Why it's needed</h4>
        <ol class="why">
          <For each={props.detail.why}>
            {(step): JSX.Element =>
              "via" in step ? (
                <li>
                  input of{" "}
                  <LinkButton
                    link={step.via.recipe}
                    navigation={props.navigation}
                  />
                  , which makes{" "}
                  <LinkButton
                    link={step.via.output}
                    navigation={props.navigation}
                  />
                </li>
              ) : (
                <li>required by {step.required}</li>
              )
            }
          </For>
        </ol>
      </Show>

      <Show when={props.detail.tags.length > 0}>
        <h4>Tags ({props.detail.tags.length})</h4>
        <ul class="links">
          <For each={props.detail.tags}>
            {(tag) => (
              <li>
                <button
                  type="button"
                  class="link"
                  title="Search for items with this tag"
                  onClick={() => props.navigation.search(`{ tag = "${tag}" }`)}
                >
                  {tag}
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>

      <LinkList
        title="Made by"
        links={props.detail.produced_by}
        navigation={props.navigation}
      />
      <LinkList
        title="Used in"
        links={props.detail.used_in}
        navigation={props.navigation}
      />
    </>
  );
}

export function RecipeDetailView(props: {
  detail: RecipeDetail;
  navigation: Navigation;
}) {
  const recipeType = () => props.detail.recipe_type;
  return (
    <>
      <h3 class="detail-title recipe">
        <span class="detail-kind">recipe</span>
        {props.detail.id}
      </h3>
      <p>
        <Liveness live={props.detail.live} needed="Useful" />
      </p>
      <p>
        <button
          type="button"
          class="link"
          title="Search for recipes of this type"
          onClick={() =>
            props.navigation.search(`{ type = "${recipeType()}" }`)
          }
        >
          {recipeType()}
        </button>
        <Show when={props.detail.eu_per_tick}>
          {(eu) => <>, {eu().toLocaleString()} EU/t</>}
        </Show>
        <Show when={props.detail.ticks}>
          {(ticks) => <>, {(ticks() / 20).toLocaleString()} s</>}
        </Show>
      </p>

      <h4>Inputs ({props.detail.inputs.length})</h4>
      <ul class="slots">
        <For each={props.detail.inputs}>
          {(slot) => (
            <li>
              {slot.count.toLocaleString()}×
              <Show when={slot.tag}>{(tag) => <> #{tag()}</>}</Show>
              <Show when={slot.chance === 0}>
                <span class="hint"> (catalyst, not consumed)</span>
              </Show>
              <Show when={slot.chance > 0 && slot.chance < 1}>
                <span class="hint">
                  {" "}
                  ({percent(slot.chance)} chance to be consumed)
                </span>
              </Show>
              <ul class="links">
                <LinkItems links={slot.members} navigation={props.navigation} />
              </ul>
            </li>
          )}
        </For>
      </ul>

      <h4>Outputs ({props.detail.outputs.length})</h4>
      <ul class="slots">
        <For each={props.detail.outputs}>
          {(output) => (
            <li>
              {output.count.toLocaleString()}×{" "}
              <LinkButton link={output.item} navigation={props.navigation} />
              <Show when={output.chance < 1}>
                <span class="hint"> ({percent(output.chance)} chance)</span>
              </Show>
              <Show when={output.edge !== null && output.edge !== "propagate"}>
                <span class="hint">
                  {" "}
                  {output.edge}, by {output.edge_cause}
                </span>
              </Show>
            </li>
          )}
        </For>
      </ul>
    </>
  );
}
