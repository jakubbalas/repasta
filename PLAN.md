# Repasta project plan

Build a keyboard-first clipboard workspace for macOS and Linux. The primary
flow is capture, find, preview, and reuse. Start with text and make capture
state, data lifetime, and platform limitations visible.

## Feature map

| Workflow | Repasta design | Milestone |
| --- | --- | --- |
| Recover recent copies | Bounded timeline, exact duplicate refresh, and a pause control | Desktop alpha |
| Find a clip | Keyboard palette with search-as-you-type and a safe text preview | Desktop alpha |
| Reuse a clip | Put the selection on the system clipboard. Offer direct insertion where the platform and permissions allow it. | Desktop alpha, then platform polish |
| Control collection | Manual capture mode, visible recording state, clear and delete actions, and exclusions when source identity is reliable | Desktop alpha |
| Choose where history lives | A storage setting: this session only (default), or a folder the user picks, encrypted or readable per folder. Later, a remote library adapter. | Local library |
| Keep useful clips | Named collections, editable text snippets, labels, and explicit retention | Local library |
| Find items by context | Filters by time, content type, collection, and source when available | Local library |
| Work with richer content | Multiple clipboard representations, paste as plain text, and bounded previews for images and file references | Rich clipboard |
| Reuse items in sequence | Ordered paste queue and explicit advance | Rich clipboard |
| Search across images or devices | Evaluate offline OCR and opt-in end-to-end encrypted sync independently | Later exploration |

## Milestones

1. **Foundation.** Dependency-free Rust history engine and stdin-driven CLI.
   Bound UTF-8 text to 64 KiB per clip, 512 clips, and 8 MiB of retained
   payload. Keep recent entries first and refresh exact duplicates. Start with
   an allocation-free scan over a bounded history for substring matching.
2. **Desktop alpha.** Add native capture and clipboard restore, then wire both
   into a keyboard-first palette. Keep history session-only. Show capture
   state and capability limits. Verify cross-app copy and restore, self-write
   suppression, pause and resume, clipboard changes during reads, and denied
   permissions.
3. **Local library.** Add the storage setting first (see "History storage"),
   then named collections, expiry, per-item deletion, and labels. Verify
   atomic writes, crash recovery, migrations, and behavior when key storage is
   locked or absent.
4. **Rich clipboard.** Preserve safe alternate formats, add the paste queue,
   and measure the impact of media on capture latency and memory. Publish a
   platform capability matrix before describing desktop compatibility.

## Platform approach

- **macOS.** Prototype AppKit general pasteboard capture with polling gated by
  `NSPasteboard.changeCount`. Measure polling latency and idle wakeups.
  Treat clipboard restore, shortcuts, and direct insertion as separate
  capabilities. [AppKit change count](https://developer.apple.com/documentation/appkit/nspasteboard/changecount)
- **Linux Wayland.** Probe the compositor for a supported data-control
  protocol. Provide a focused manual capture workflow when background capture
  is unavailable. Verify keyboard shortcut support separately and document
  behavior by compositor. [Wayland protocol definition](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/ext-data-control/ext-data-control-v1.xml)
- **Linux X11.** Watch ownership of the `CLIPBOARD` selection and implement
  bounded, time-limited transfers. Treat source attribution as best effort.
  [X.Org selection documentation](https://www.x.org/releases/X11R7.6/doc/libX11/specs/libX11/libX11.html)

Keep capture and clipboard ownership logic in native adapters. Keep policy,
history, and search in Rust. The first palette uses Iced with the tiny-skia
software renderer. Evaluate it on both platforms for latency, focus,
accessibility, and packaging. The Linux preview uses bounded reads from
clipboard helpers with a timeout for manual capture while native capture
adapters are pending.

## History storage

The user picks where history lives in the panel's settings view:

- **This session only** (default). History stays in memory and ends when
  Repasta quits.
- **A folder.** Repasta keeps one history file in the chosen folder and
  rewrites it atomically after each change (temporary file, sync, rename).
  Each folder is either encrypted or readable:
  - *Encrypted* files hold AES-256-GCM sealed history. The key is created on
    first use and kept in the macOS Keychain. Linux reports encryption as
    unavailable until a Secret Service adapter exists.
  - *Readable* files use a simple length-prefixed text format that opens in
    any editor.

Loading treats the file as hostile input: it is bounded by the same clip,
count, and byte limits as live history and rejected if malformed. Choosing a
folder that already holds history merges it with the session. Switching a
folder between encrypted and readable rewrites it in the new form and removes
the old file. Switching back to session-only keeps the folder's file, and Clear
deletes the file of the active folder. Two Repasta instances must not share a
folder; there is no cross-process locking yet.

Storage sits behind a small `Store` interface (load, save, erase) in the
platform-neutral library. The folder store and its encryption are one
implementation. Native key storage and folder pickers stay in desktop adapters.

### Remote library adapter (exploration)

A later `Store` implementation could keep history in a remote library service
through its API, for example the user's own Memberberry workspace. Open
questions before building it:

- Sync model: whole-history snapshots work for a folder, but a remote library
  needs per-clip create and delete events, conflict handling, and offline
  queues.
- Encryption: whether clips are sealed before upload so the service never sees
  plaintext.
- Credentials: API tokens live in OS-backed key storage, never in settings
  files or logs.
- Network policy: uploads run off the capture and search path, require explicit
  opt-in, and show their state in the panel.

## Data handling and threat model

- Require explicit user activation for background collection. Honor sensitive
  markers and app exclusions where the platform provides dependable signals.
  Clipboard sources may omit these signals, so provide a quick pause control
  and a manual-only mode.
- Treat every representation as hostile input. Bound bytes and transfer time,
  escape display text, and isolate rich-format parsing. Treat URLs and paths
  as data unless the user explicitly chooses an action.
- Keep network activity out of the capture and search path. Keep payloads and
  search queries out of diagnostics. Define the scope of clear and delete
  across live state, indexes, and any on-disk copies before persistence ships.
- Persistent history is opt-in per folder. Encryption is on by default and
  uses authenticated encryption with a key in OS-backed key storage. Readable
  files are an explicit per-folder choice, and the settings view warns that
  copied secrets are then stored as readable text. If the key store is
  unavailable or a saved file cannot be read, keep the file untouched and use
  session-only history until the user resolves it.
- Account for exposure through a running user session, swap, and crash dumps.
  OS isolation and session lock behavior are part of the platform review.

## Initial performance targets

Use synthetic data and release builds. Record hardware, OS, compositor, and
corpus with each result. Targets are provisional until the first desktop
prototype is measured.

- Search p95 under 10 ms with 512 clips, including a full 8 MiB corpus and
  no-match queries.
- Visible palette p95 under 100 ms from a shortcut in a warm session.
- Capture to searchable p95 under 300 ms for text, including polling delay.
- Idle background CPU under 0.5% of one core over five minutes. Report idle
  RSS and wakeups alongside CPU.
- During rapid copies or stalled transfers, keep memory and work queues
  bounded and record dropped or rejected captures without including clip data.

The first search baseline was **0.38 ms p95** on an Apple M5 Pro running
macOS 27.2 with optimized Rust code. It measured 500 no-match searches over
512 distinct synthetic 16 KiB clips (8 MiB total), excluding UI rendering.

Saving a full history to a folder took **11.6 ms** readable and **13.8 ms**
encrypted (mean of 10 saves, including sync and rename); loading it took
3.2 ms and 3.8 ms. Measured on the same Apple M5 Pro, macOS 27.2, release
build, APFS internal SSD, with 128 synthetic 64 KiB clips (8 MiB). Saves run on
the UI thread after each change, so move them off it if larger histories or
slower disks push this toward a frame budget.
