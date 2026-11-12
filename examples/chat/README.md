# Chat

A shared web/desktop chat example with local, scripted replies. No API key or
HTTP server is required. Conversations can stream independently while you type,
search, or switch chats.

From the repository root:

```sh
# Web
target/debug/refract dev -p refract-chat --port 4175 --offline --open

# Desktop (in another terminal)
target/debug/refract dev -p refract-chat --platform desktop --offline
```

Build the CLI first if needed: `cargo build -p refract --features cli`.

## Try it

Send “Explain Rust state” to see a reply explaining the implementation. While
it streams, open another conversation and send another message. Both replies
continue independently. Stop preserves a partial reply; deleting a conversation
cancels only its own producer. Drafts survive switching conversations.

## Async work and channels

- `service.rs`: a committed `#[effect]` owns a typed `Runtime::channel` receiver
  and producer cleanup. Only the receiver touches UI-local graph state.
- `producer/native.rs`: generates and sends owned chunks on a worker thread.
  An interruptible wait paces delivery and wakes immediately on cancellation.
- `producer/browser.rs`: an abortable async task awaits browser delays and sends
  the same events. Dropping a pending delay clears its timeout. This is cooperative
  async work on the browser thread, not a Web Worker for CPU-heavy work.
- Both producers retain and retry messages when the inbox is full, yielding
  between retries. Closing the receiver ends delivery.
- `model.rs`: Stop clears the request immediately. Receivers check request IDs,
  so even completions queued before Stop cannot overwrite cancellation or leak
  into a subsequent reply.
- Effects live outside the visible conversation pane, so switching or filtering
  chats does not unmount a background stream. Completion, deletion, and app
  disposal release the channel and cancel the producer.

The source is injected internally for deterministic lifecycle tests. Production
adapters receive only a prompt and a message sender, never state or update handles.

## Checks

```sh
cargo test -p refract-chat --offline --lib
```

Tests cover cross-thread delivery, a full inbox, precise text-only updates,
concurrent conversation ownership, cancellation with queued results, deletion,
failure/retry, and disposal. With the web server running, open `/smoke.html` for
browser interaction checks.

## Viewport, title, and history

Select a conversation and watch the URL's `conversation` query parameter and the
browser/window title update. Browser Back and Forward restore the selected chat
without creating another entry. Query routing preserves the current path, other
query parameters, and deployment prefixes. Initial links to the three seeded
conversations work on reload; chats created during this session are not persisted,
so missing or deleted IDs are replaced with the current valid selection.

Resize below 760 logical pixels to collapse the sidebar. The menu button opens a
full-width conversation menu; selecting a conversation closes it again. The
breakpoint is a `Derived::distinct` value, so resizing within a size class does not
reevaluate the shell on every pixel. Sidebar preferences are scoped to the size
class. Streaming and drafts continue while the compact menu is visible.

`surface.rs` owns committed effects for incoming location changes and outgoing
titles. User selection pushes history; deletion and URL normalization replace
it. Host observations never call the selection command that pushes history.
The native launcher supplies `app://chat/` as its initial application URL.

## Focus feedback

Clicking the composer shows a quiet accent outline around its rounded container;
Tab navigation uses a stronger outline. The inner field delegates its indicator
with `.focus_ring(FocusRing::none())`, and the container uses
`.focus_within(FocusRing::new(ACCENT, 2.0, 2.0))`.

`FocusRing::new(color, width, offset)` configures the keyboard indicator in logical
pixels. Its corners follow `.radius(...)`; it occupies no layout space. Pointer
editing uses the same color at reduced opacity with at most a one-pixel outline
and no gap. Ordinary typing preserves that quieter treatment.

Native Tab and Shift+Tab reveal the focused control through nested scroll views.
This runs on navigation, so manually scrolling away does not snap back on the
next frame. If a focused control is removed, disabled, or excluded from tab
navigation, both hosts try the next surviving tab stop in the previous tree,
then search backward. With no remaining tab stop, focus clears. A disabled
control still receives its final `on_focus_change(false)` notification; removed
controls have no remaining handler to notify.

## Animation

The wide sidebar slides in and out as its layout width expands or collapses over
220ms. Compact windows use a short slide and fade for the sidebar panel.
Conversation selection fades in over 140ms and clears the previous highlight
immediately. Composer/send-control colors ease over 140ms. Switching
conversations fades the new pane in over 120ms; new messages fade in with a four
pixel upward entrance over 180ms. These are value-scoped presentation animations.
Typing and streamed tokens remain immediate, preserve editor/element identity,
and do not restart the message entrance. Initial mounting has no entrance motion.
