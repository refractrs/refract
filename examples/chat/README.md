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
