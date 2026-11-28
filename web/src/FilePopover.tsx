import { createSignal, Show } from "solid-js";
import type { SessionClient } from "./session";

// What the popover shows. Either a file's text, or why it couldn't be read.
interface Shown {
  path: string;
  text?: string;
  error?: string;
}

// One popover for showing a file of the export, e.g. a recipe's JSON, with a
// button to copy it. `show` opens it with the file at `path`, once that's read,
// so it doesn't flash empty first.
export function createFilePopover(session: SessionClient) {
  let popover!: HTMLDivElement;
  const [shown, setShown] = createSignal<Shown>();
  const [copied, setCopied] = createSignal(false);

  // The latest path asked for. A slower earlier read won't replace it.
  let latest: string | undefined;
  const show = (path: string) => {
    latest = path;
    const open = (current: Shown) => {
      if (latest !== path) return;
      setShown(current);
      setCopied(false);
      if (!popover.matches(":popover-open")) popover.showPopover();
    };
    session.fileText(path).then(
      (text) => open({ path, text }),
      (caught: unknown) =>
        open({
          path,
          error: caught instanceof Error ? caught.message : String(caught),
        }),
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
            <Show when={current().text}>{(text) => <pre>{text()}</pre>}</Show>
          </>
        )}
      </Show>
    </div>
  );

  return { show, view };
}
