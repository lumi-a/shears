import { createSignal, Show } from "solid-js";
import type { SessionClient } from "./session";

// What the popover shows. Either a file's text, or why it couldn't be read.
interface Shown {
  path: string;
  text?: string;
  error?: string;
}

// One popover for showing a file of the export, e.g. a recipe's JSON, with a
// button to copy it. `show` opens it with the file at `path`.
export function createFilePopover(session: SessionClient) {
  let popover!: HTMLDivElement;
  const [shown, setShown] = createSignal<Shown>();
  const [copied, setCopied] = createSignal(false);

  const show = (path: string) => {
    setShown({ path });
    setCopied(false);
    popover.showPopover();
    session.fileText(path).then(
      (text) => {
        if (shown()?.path === path) setShown({ path, text });
      },
      (caught: unknown) => {
        if (shown()?.path === path) {
          setShown({
            path,
            error: caught instanceof Error ? caught.message : String(caught),
          });
        }
      },
    );
  };

  const copy = (text: string) =>
    navigator.clipboard.writeText(text).then(
      () => setCopied(true),
      () => setCopied(false),
    );

  const view = (
    <div
      class="file-popover"
      popover
      ref={(element) => (popover = element)}
      aria-label="File"
    >
      <Show when={shown()}>
        {(current) => (
          <>
            <div class="file-popover-header">
              <span class="file">{current().path}</span>
              <Show when={current().text}>
                {(text) => (
                  <button type="button" onClick={() => copy(text())}>
                    {copied() ? "Copied" : "Copy"}
                  </button>
                )}
              </Show>
              <button type="button" onClick={() => popover.hidePopover()}>
                Close
              </button>
            </div>
            <Show when={current().error}>
              {(message) => <p class="error">{message()}</p>}
            </Show>
            <Show
              when={current().text}
              fallback={
                <Show when={current().error === undefined}>
                  <p class="hint">Loading...</p>
                </Show>
              }
            >
              {(text) => <pre>{text()}</pre>}
            </Show>
          </>
        )}
      </Show>
    </div>
  );

  return { show, view };
}
