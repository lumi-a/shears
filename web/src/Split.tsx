import { createSignal, type JSX } from "solid-js";

// Each side keeps at least this fraction of the split's size.
const MIN_FRACTION = 0.1;
// How far one arrow key press moves the divider, as a fraction of the size.
const KEY_STEP = 0.02;

const clamp = (value: number, low: number, high: number) =>
  Math.min(Math.max(value, low), high);

// Where the divider sits as a fraction of the size, MIN_FRACTION from either end.
function loadFraction(storageKey: string, initial: number): number {
  try {
    // Try and recall from local storage, sigh
    const stored: unknown = JSON.parse(localStorage.getItem(storageKey) ?? "");
    if (
      typeof stored === "number" &&
      stored >= MIN_FRACTION &&
      stored <= 1 - MIN_FRACTION
    ) {
      return stored;
    }
  } catch {
    // Nothing stored yet, or not valid, so use `initial`.
  }
  return initial;
}

// Two parts with a draggable divider between them. Either vertical or horizontal.
//
// Drag the divider to resize, or focus it and use the arrow keys.
export function Split(props: {
  direction: "columns" | "rows";
  // Where the divider position is remembered across visits.
  storageKey: string;
  // The first part's share of the size until the user moves the divider.
  initial: number;
  first: JSX.Element;
  second: JSX.Element;
}) {
  const [fraction, setFraction] = createSignal(
    // Only read once: these props aren't expected to change.
    // eslint-disable-next-line solid/reactivity
    loadFraction(props.storageKey, props.initial),
  );
  let container!: HTMLDivElement;
  const isColumns = () => props.direction === "columns";

  const moveDivider = (to: number) => {
    const next = clamp(to, MIN_FRACTION, 1 - MIN_FRACTION);
    setFraction(next);
    try {
      localStorage.setItem(props.storageKey, JSON.stringify(next));
    } catch {
      // Storage unavailable (e.g. private mode), so just don't remember it, it's fine.
    }
  };

  // minmax(0, ...) lets a part shrink below its content's size.
  const tracks = () =>
    `minmax(0, ${fraction()}fr) var(--divider-width) minmax(0, ${1 - fraction()}fr)`;

  const onPointerMove = (event: PointerEvent) => {
    const rect = container.getBoundingClientRect();
    moveDivider(
      isColumns()
        ? (event.clientX - rect.left) / rect.width
        : (event.clientY - rect.top) / rect.height,
    );
  };

  const onKeyDown = (event: KeyboardEvent) => {
    const [back, forward] = isColumns()
      ? ["ArrowLeft", "ArrowRight"]
      : ["ArrowUp", "ArrowDown"];
    const step =
      event.key === back ? -KEY_STEP : event.key === forward ? KEY_STEP : 0;
    if (step === 0) return;
    event.preventDefault();
    moveDivider(fraction() + step);
  };

  return (
    <div
      class={`split ${props.direction}`}
      ref={(element) => (container = element)}
      style={{
        [isColumns() ? "grid-template-columns" : "grid-template-rows"]:
          tracks(),
      }}
    >
      {props.first}
      <div
        class="split-divider"
        role="separator"
        // A vertical line separates columns, a horizontal one rows.
        aria-orientation={isColumns() ? "vertical" : "horizontal"}
        aria-valuenow={Math.round(fraction() * 100)}
        aria-valuemin={0}
        aria-valuemax={100}
        tabindex="0"
        onPointerDown={(event) => {
          event.currentTarget.setPointerCapture(event.pointerId);
          event.currentTarget.addEventListener("pointermove", onPointerMove);
        }}
        onLostPointerCapture={(event) => {
          event.currentTarget.removeEventListener("pointermove", onPointerMove);
        }}
        onKeyDown={onKeyDown}
      />
      {props.second}
    </div>
  );
}

// A titled, independently scrolling part of the layout.
export function Pane(props: { title: string; children: JSX.Element }) {
  return (
    <section class="pane" aria-label={props.title}>
      <h2 class="pane-title">{props.title}</h2>
      <div class="pane-body">{props.children}</div>
    </section>
  );
}
