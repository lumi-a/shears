import { createSignal, type JSX } from "solid-js";

// Each pane keeps at least this fraction of the width.
const MIN_FRACTION = 0.1;
// How far one arrow key press moves a divider as a fraction of the width.
const KEY_STEP = 0.02;
const STORAGE_KEY = "shears.pane-dividers";

export interface PaneSpec {
  // Shown in the pane's header and used as its accessible name.
  title: string;
  body: JSX.Element;
}

type Dividers = [number, number];

// Where the two dividers sit as fractions of the width: 0 < first < second < 1.
function loadDividers(): Dividers {
  try {
    // Try and recall from local storage, sigh
    const stored: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "");
    if (
      Array.isArray(stored) &&
      stored.length === 2 &&
      stored.every((value) => typeof value === "number")
    ) {
      const [first, second] = stored as Dividers;
      if (
        first >= MIN_FRACTION &&
        second - first >= MIN_FRACTION &&
        1 - second >= MIN_FRACTION
      ) {
        return [first, second];
      }
    }
  } catch {
    // Nothing stored yet, or not valid, so use the default values below.
  }
  return [0.3, 0.6];
}

const clamp = (value: number, low: number, high: number) =>
  Math.min(Math.max(value, low), high);

// Three side-by-side panes.
// Drag a divider to resize (or focus it and use the arrow keys).
export function ThreePanes(props: { panes: [PaneSpec, PaneSpec, PaneSpec] }) {
  const [dividers, setDividers] = createSignal<Dividers>(loadDividers());
  let container!: HTMLDivElement;

  // Moves divider `index` to `fraction` while keeping every pane at least MIN_FRACTION wide.
  const moveDivider = (index: 0 | 1, fraction: number) => {
    const [first, second] = dividers();
    const next: Dividers =
      index === 0
        ? [clamp(fraction, MIN_FRACTION, second - MIN_FRACTION), second]
        : [first, clamp(fraction, first + MIN_FRACTION, 1 - MIN_FRACTION)];
    setDividers(next);
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
    } catch {
      // Storage unavailable (e.g. private mode): just don't remember it.
    }
  };

  const columns = () => {
    const [first, second] = dividers();
    // minmax(0, ...) lets a pane shrink below its content's width; it scrolls.
    return [first, second - first, 1 - second]
      .map((fraction) => `minmax(0, ${fraction}fr)`)
      .join(" var(--divider-width) ");
  };

  const Divider = (dividerProps: { index: 0 | 1 }) => {
    const onPointerMove = (event: PointerEvent) => {
      const rect = container.getBoundingClientRect();
      moveDivider(dividerProps.index, (event.clientX - rect.left) / rect.width);
    };
    return (
      <div
        class="pane-divider"
        role="separator"
        aria-orientation="vertical"
        aria-valuenow={Math.round(dividers()[dividerProps.index] * 100)}
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
        onKeyDown={(event) => {
          const step =
            event.key === "ArrowLeft"
              ? -KEY_STEP
              : event.key === "ArrowRight"
                ? KEY_STEP
                : 0;
          if (step === 0) return;
          event.preventDefault();
          moveDivider(
            dividerProps.index,
            dividers()[dividerProps.index] + step,
          );
        }}
      />
    );
  };

  return (
    <div
      class="panes"
      ref={(element) => (container = element)}
      style={{ "grid-template-columns": columns() }}
    >
      <Pane spec={props.panes[0]} />
      <Divider index={0} />
      <Pane spec={props.panes[1]} />
      <Divider index={1} />
      <Pane spec={props.panes[2]} />
    </div>
  );
}

function Pane(props: { spec: PaneSpec }) {
  return (
    <section class="pane" aria-label={props.spec.title}>
      <h2 class="pane-title">{props.spec.title}</h2>
      <div class="pane-body">{props.spec.body}</div>
    </section>
  );
}
