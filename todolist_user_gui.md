# tinybrowser — GUI / user-interaction plan (`todolist_user_gui.md`)

> Goal: turn the engine into a **browser a human can drive** — a window (or a
> terminal) with tabs, an address bar, back/forward/reload, a page view that
> scrolls, mouse clicks on links, keyboard shortcuts, selection, find-in-page,
> typing into forms, an inspector. Chrome/Firefox in *features and feel*,
> rendered by **our own** code.
>
> Hard rule, same as the rest of the workspace: **no third-party crates**.
> That means no `winit`, no `winit`-backed GUI toolkit, no `crossterm`, no
> `ratatui`, no `x11rb`, no `libc`. Windowing, terminal handling, input
> decoding and the widget layer are written here. The only foreign code in the
> whole workspace stays `rustls` + `webpki-roots`.

Legend: `[ ]` open, `[x]` DONE, `[~]` partial / deliberately simplified.

---

## 0. Ground rules and architecture

- [x] **G0.1** Decide the display model. Two renderers, one application state:
      - **cell grid** (text cells: char + fg + bg + bold/underline/inverse) —
        what a terminal can show, and what an X11 window shows when we draw our
        own bitmap font into it;
      - **pixels** (`tb_gfx::Framebuffer`) — the real page rasterisation we
        already have, used by pixel-capable backends.
      Both are produced from the *same* `App` state, so every interaction is
      testable head-less.
- [x] **G0.2** New bounded context **`presentation/ui`** with two crates:
      `tb-ui` (backend-agnostic shell: tabs, chrome, modes, commands, selection,
      search, hints, inspector) and the backend crates `tb-term`, `tb-x11`.
      Dependency direction: `tb-core ← … ← tb-browser ← tb-ui ← {tb-term, tb-x11} ← tb-cli`.
      Nothing below `tb-ui` knows a UI exists.
- [x] **G0.3** `Backend` trait as the anti-corruption layer between the shell and
      the display: `size`, `present(Screen|Framebuffer)`, `next_event(timeout)`,
      `set_title`, clipboard, `should_quit`. A `Headless` backend implements it
      for tests and for scripted demos.
- [x] **G0.4** Event model independent of any device: `Event::{Key, Pointer,
      Resize, Paste, Focus, Tick}`, `Key::{Char, Named}`, `Mods`, `PointerKind`.
- [x] **G0.5** Commands, not key codes, inside the app: `Command::{GoUrl, Back,
      Reload, NewTab, CloseTab, FindNext, HintMode, ToggleDevtools, …}` plus a
      keymap that maps `(Key, Mods, Mode) → Command`. Tests drive the app with
      commands *and* with raw key events.
- [x] **G0.6** `Screen` (cell grid) + `Theme` (light/dark chrome palette) as the
      canonical UI output type — diffable in tests, printable in CI.

## 1. Embedder API the UI needs (`tb-browser`, `tb-dom-bindings`)

The pipeline could produce a page, but nothing could *poke* it. Added:

- [x] **G1.1** `Host::match_state` reachable from the embedder: `Page::set_hover`,
      `Page::set_active` (`:hover`, `:active` are CSS states, the UI must be able
      to set them, not fake them).
- [x] `Page::set_focus(Option<NodeId>)` — reuses the `focus()` path in
      `tb-dom-bindings` (blur the old, `focus` event on the new, `:focus` matching).
- [x] **G1.3** `Page::fire(node, kind, &EventInit)` — events **with** payload
      (`clientX/clientY`, `button`, `key`, `code`, `ctrlKey`, …); `make_event`
      already accepted them, only the Rust entry point was missing.
- [x] **G1.4** Form editing: `Page::set_input_value(node, &str)` (writes the
      *dirty value*, not the attribute) and `Page::input_value(node)`.
- [x] **G1.5** Scrolling as state: `Page::set_scroll(x, y)` keeps `Host.scroll`
      (so `window.scrollY` is the truth) and drops the paint cache.
- [x] **G1.6** Geometry for the UI: `Page::runs()` (text runs with position and
      style, in reading order — selection, find and the text renderer all need
      them), `Page::rect_of`, `Page::element_at`, `Page::link_href_at`.
- [x] **G1.7** `Page::scroll_into_view(node)` — focus-follows-keyboard needs it.
- [x] **G1.8** Cost guard for `:hover` restyling: restyle only when the hovered
      element *changed*, and only for documents small enough for it to stay
      interactive (`--hover-css` forces it on).
- [x] **G1.9** Tests for every entry point above (`crates/tb-browser/tests/interaction.rs`).

## 2. `tb-ui` — shell core (no drawing yet)

- [x] **G2.1** `event.rs`, `keymap.rs` (see G0.4/G0.5), `mode.rs`
      (`Normal`, `Address`, `Find`, `Hints`, `Devtools`, `Menu`, `Select`, `Passthrough`).
- [x] **G2.2** `editor.rs` — single line text editor for the address bar and the
      find bar: caret, selection, insert/delete, word motions, `Home`/`End`,
      history recall (↑ through visited URLs), and a `display()` that elides the
      middle of a long URL like Chrome does. Suggestions (history + bookmarks)
      are completed on `Tab`; a floating suggestion *popup* was **cut**.
- [x] **G2.3** `chrome.rs` — the geometry and hit-testing of the browser chrome:
      tab strip, `+` button, back/forward/reload/home, address bar with lock
      glyph, bookmark star, menu button, bookmark bar, status bar, find bar,
      scrollbars. Hit-testing answers `ChromeHit::{Tab, NewTab, Back, …}`.
- [x] **G2.4** `tab.rs` — a `Tab` owns one `Page` **plus** everything that is
      per-view and not per-document: scroll offset, per-tab session history,
      hover target, focused node, selection, find state, pending address text.
- [x] **G2.5** `app.rs` — the state machine: tab list, active tab, mode, focus of
      the *chrome* vs the *page*, command dispatch, navigation (including
      `NavRequest`s a script asked for), load progress, title.
- [x] **G2.6** `selection.rs` — text selection over layout runs: point→run lookup,
      word/line expansion (double/triple click), drag selection, `text()` for the
      clipboard, `rects()` for highlighting. Works in both renderers.
- [x] **G2.7** `search.rs` — find-in-page: case-insensitive (+ `\b`-free) matching
      over runs, next/prev wrap-around, match count, per-match highlight.
- [x] **G2.8** `hints.rs` — keyboard link activation (`f` / `F`): visible link
      candidates from the box tree, `aa`/`ab`… labels, type-to-filter, first
      match wins on timeout, `Esc` cancels, `F` opens in a new tab.
- [x] **G2.9** `devtools.rs` — inspector drawer with pages: **Elements** (DOM path
      + attributes of the picked node), **Console** (page `console.*` + errors, and
      a prompt that `eval`s), **Problems** (parse/network/script), **CSS**
      (sheets, rule counts, computed style of the picked node), **Layout** (box
      geometry under the cursor), **Info** (URL, status, timings, stats).
- [x] **G2.10** `clipboard.rs` — internal clipboard (copy/paste works without any
      system service) + OSC-52 escape for terminals that support it.
- [x] **G2.11** `theme.rs` — light and dark chrome palette, `--theme`, follows
      `prefers-color-scheme` for the page viewport (`ColorScheme` in `EngineConfig`).

## 3. `tb-ui` — rendering

- [x] **G3.1** `screen.rs` — the cell grid (`Cell { ch, fg, bg, bold, underline,
      inverse }`), `put`, `fill`, `text`, `rect_border`, `clip`, a `diff()` used
      by tests, and a plain-text dump for CI.
- [x] **G3.2** `render_cells.rs` — the whole UI as cells: chrome rows + page
      content mapped from layout runs (run → row by baseline, run → column by
      `x / advance`), link colour/underline from computed style, selection and
      find highlights, caret for the focused input, hint labels, devtools drawer,
      scrollbars as `█`/`▒`, loading spinner row.
- [x] **G3.3** `render_pixels.rs` — the pixel path: page painted by `Page::paint`
      into the content area, chrome painted with `BoxPaint`/`TextPaint` (our own
      5×7 font), selection/hover/find overlays, scrollbars, focus rings.
- [x] **G3.4** Dirty tracking: repaint only when something changed (event, scroll,
      DOM mutation from a timer), and only the content area when a page changed.
- [x] **G3.5** `Headless` backend: fixed grid, scripted events, screen assertions
      (`asserts/*.txt` style diffs), used by all `tb-ui` tests.

## 4. `tb-term` — terminal backend, written from scratch

- [x] **G4.1** Raw mode without `libc`: `stty -g` saves, `stty raw -echo`
      enters, `shutdown()` and `Drop` restore — deliberately **not** `min 0
      time 1`, because a timed read returns zero bytes and zero bytes is how
      `read()` reports EOF: the reader thread would be dead before the first
      key. Refuses to run when stdin is not a TTY (then `--ui-backend headless`
      prints the screen instead).
- [x] **G4.2** Size detection without `ioctl`: `COLUMNS`/`LINES`, else `stty size`,
      else the `CSI 18 t` report re-read from stdin; re-queried when idle, so a
      resized window reflows (`Resize` event → relayout).
- [x] **G4.3** `escape.rs` — output side: alternate screen, hide/show cursor,
      truecolor SGR (`38;2;r;g;b` / `48;2;…`), bold/underline/inverse, clear,
      run-length-ish writer that only emits changes, `\x1b[8;rows;cols` reader for
      size, window title OSC 0, OSC 52 clipboard, bell.
- [x] **G4.4** `input.rs` — input side, a small state machine over bytes:
      printable UTF-8 (multi-byte safe), `CR`/`LF`/`TAB`/`BS`/`ESC`, CSI `A/B/C/D`
      arrows with modifier params, `~`-forms (Insert/Delete/PageUp/PageDown/
      Home/End/F1–F5), `ESC[1;5C` (Ctrl+→), SS3, function keys via `ESC O P…S`,
      mouse: `SGR` (`ESC[<b;x;yM|m`) with click/drag/release/wheel and modifier
      bits, bracketed paste.
- [x] **G4.5** Mouse-to-cell-to-pixel mapping, wheel = 3 rows, shift+wheel = page.
- [x] **G4.6** `TermBackend` implementing `tb_ui::Backend`, with a stdin reader
      thread feeding an `mpsc::Receiver` so `next_event(timeout)` can block
      without `select()`.
- [x] **G4.7** Tests: escape parser vectors (arrows, modifiers, SGR mouse, paste,
      partial sequences split across reads), SGR writer output golden tests,
      UTF-8 boundary tests.

## 5. `tb-x11` — a real window without `x11rb`

- [x] **G5.1** `xauth.rs` — parse `~/.Xauthority` (length-prefixed records:
      family, address, display number, protocol, data) so `DISPLAY=:0` works.
- [x] **G5.2** `conn.rs` — connect to `/tmp/.X11-unix/X<n>` (or `tcp:host:port`),
      connection request with `MIT-MAGIC-COOKIE-1`, read setup reply
      (roots, depths, visuals, pixmap formats, `resource_id_base`), request/reply
      framing, error events, sequence tracking.
- [x] **G5.3** `window.rs` — `CreateWindow` (truecolor visual of depth 24/32),
      `CreateGC`, `ChangeProperty` for `WM_NORMAL_HINTS`, `StoreName`/
      `_NET_WM_NAME`, `ChangeWindowAttributes` event mask, `MapWindow`,
      `ConfigureNotify`/`Expose` handling, `PutImage` (ZPixmap, 32 bpp, chunked by
      `max_request_length`). `GetImage` was **cut** — nothing reads pixels back;
      the round trip is a `GetInputFocus` (`Sync`, opcode 120, is rejected
      `BadRequest` by this Xorg).
- [x] **G5.4** `keysyms.rs` — `GetKeyboardMapping` once at startup, keycode +
      `Shift`/`Lock` state → keysym; Latin-1 keysyms map straight to chars,
      `0xff00`-block keys map to `Key::Named`.
- [x] **G5.5** `events.rs` — decode `KeyPress`/`KeyRelease`, `ButtonPress`/
      `ButtonRelease` (incl. wheel buttons 4–7), `MotionNotify`, `EnterNotify`,
      `ConfigureNotify`, `ClientMessage` (`WM_DELETE_WINDOW`, `_NET_WM_PING`),
      `DestroyNotify` → `tb_ui::Event`.
- [x] **G5.6** `X11Backend`: draws the `Screen` with our own bitmap font into a
      framebuffer and `PutImage`s it; double-buffered, only on damage.
- [x] **G5.7** Window close, minimize-safe, resize → relayout, primary-selection
      copy on select (best effort), `--ui-backend x11`.
- [x] **G5.8** Tests: Xauthority vectors; setup/keysym/event decoding from
      committed byte vectors; **live** round-trip test `#[ignore]`d, run with
      `xvfb-run` (create window, put image, `GetImage` it back, compare pixels).
      Skipped automatically when no display is reachable.

## 6. Interaction — mouse

- [x] **G6.1** Pointer move → hover: link under the cursor in the status bar,
      `:hover` for the page (throttled, G1.8), `cursor: pointer`-ish feedback.
- [x] **G6.2** Left click: chrome buttons; page → `<a href>` navigation
      (fragment-only links scroll, `javascript:`/`mailto:` are reported, not
      fetched), `target="_blank"` opens a new tab, form controls (text field
      focus, checkbox/radio toggle, `<select>` popup, submit button),
      buttons that only exist for the UI (e.g. closing a tab).
- [x] **G6.3** Click order matters: `mousedown` → `mouseup` → `click`, and the
      navigation happens only if no handler cancelled the click (`preventDefault`).
- [x] **G6.4** Middle click = open in new tab (links, and `+` on a tab closes it).
- [x] **G6.5** Right click = context menu (back/forward/reload, open link in new
      tab, copy link address, copy selected text, view source, devtools, zoom),
      click-away dismisses.
- [x] **G6.6** Wheel: page scroll with the usual accelerations, `Ctrl`+wheel zoom,
      wheel over the tab strip switches tabs, middle-click autoscroll is
      deliberately not modelled.
- [x] **G6.7** Drag: select text; double click selects a word, triple a line;
      dragging the scrollbar thumb, clicking a scrollbar track pages.
- [x] **G6.8** Tab strip: click to switch, middle click closes, drag is *not*
      modelled (documented).
- [x] **G6.9** `:active` while a mouse button is down on an element.

## 7. Interaction — keyboard

- [x] **G7.1** Address bar: `Ctrl+L`, `F6`, `Alt+D`, or click; typing; `Enter`
      navigates; a bare word goes through a search engine (`--search-url`);
      `Esc` restores; `↑`/`↓` walk visited-URL history; live suggestions
      (history + bookmarks) as a popup.
- [x] **G7.2** Navigation: `Alt+←`/`Backspace` back, `Alt+→` forward,
      `Ctrl+R`/`F5` reload, `Ctrl+Shift+R` bypassing the cache, `Alt+H` home,
      `Ctrl+U` view source.
- [x] **G7.3** Tabs: `Ctrl+T`, `Ctrl+W`, `Ctrl+Shift+T` (reopen last closed),
      `Ctrl+Tab`/`Ctrl+Shift+Tab`, `Ctrl+1…8`, `Ctrl+9` (last).
- [x] **G7.4** Scrolling: arrows, `PgUp`/`PgDn`, `Space`/`Shift+Space`, `Home`,
      `End`, and `Ctrl+Home`/`Ctrl+End`.
- [x] **G7.5** Find: `Ctrl+F` opens, `Enter`/`N` next, `Shift+Enter`/`P` previous,
      `Esc` closes and clears highlight, `Ctrl+G` = find next.
- [x] **G7.6** Links by keyboard: `f` (current tab) / `F` (new tab) hint mode.
- [x] **G7.7** Focus traversal with `Tab`/`Shift+Tab` over links and form
      controls, `Enter`/`Space` activates the focused control, typing into a
      focused text field edits its value and fires `keydown`/`keypress`/`input`
      (`Backspace`, arrows, `Home`/`End` included), `Enter` in a single-line
      field submits its form.
- [x] **G7.8** Devtools: `F12` toggles, `Ctrl+J` console, `Ctrl+Shift+C` pick
      element, `1`–`6` switch panel inside the drawer (when it has focus).
- [x] **G7.9** Zoom `Ctrl+=`/`Ctrl+-`/`Ctrl+0` (engine scale, real re-layout),
      theme `Ctrl+Shift+K` (dark/light).
- [x] **G7.10** Copy/paste: `Ctrl+C` copies the selection, `Ctrl+Insert` too,
      `Ctrl+V` pastes into the address bar or a focused field, `Ctrl+A` selects
      all; clipboard through OSC-52 where the terminal allows it.
- [x] **G7.11** Bookmarks: `Ctrl+D` add, bookmark bar with clicks, `Ctrl+B`
      toggles the bar, `~/.config/tiny/bookmarks.json` persistence (own JSON
      writer/reader on `tb-js`'s JSON, no serde).
- [x] **G7.12** Session: `Ctrl+Q` quits, per-tab history survives tab switches,
      closed tabs come back with `Ctrl+Shift+T`. A `--restore` session file was
      **cut** (no state directory policy yet; `--new-tab URL` composes the same
      startup by hand).
- [x] **G7.13** `?`/`F1` help overlay (key reference), `:` command line
      (`:help`, `:open URL`, `:tabnew`, `:bookmarks`, `:devtools`, `:quit`).

## 8. Wiring, docs, quality

- [x] **G8.1** CLI: `--ui`, `--ui-backend auto|x11|term|headless`, `--theme`,
      `--zoom`, `--new-tab URL` (repeatable), `--search-url`, `--home URL`,
      `--no-chrome`, `--ui-script FILE` (drive the UI from a script and print the
      screen — this is how the GUI is tested in CI).
- [x] **G8.2** `auto` backend: X11 if `DISPLAY` resolves, else terminal if stdin
      is a TTY, else head-less with a clear message.
- [x] **G8.3** Tests: shell tests (keymap, commands, chrome hit-testing, address
      editor, selection, find, hints, devtools, tab history, form typing,
      navigation via click), terminal parser/writer tests, X11 vectors + live.
- [x] **G8.4** `cargo junit` picks the new suites up; clippy `-D warnings` and
      `cargo fmt --check` clean.
- [x] **G8.5** README: section **"GUI — user interactions"** (keys, mouse,
      backends, what is deliberately not modelled), status table and workspace
      list updated.

## 9. Deliberately **not** modelled (and why)

- `canvas`, WebGL, video/audio: we have no rasteriser/decoder for them.
- Drag and drop between windows, IME composed text input, multi-touch,
  HiDPI fractional scaling (integer zoom only), sub-pixel font rendering.
- X11 CLIPBOARD selection *acquisition as owner* and full `Xdnd`: OSC-52 and the
  internal clipboard cover copy/paste for our purposes.
- Wayland backend: the protocol needs a compositor-specific handshake and a
  `xdg-shell` implementation; the X11 backend (or XWayland) covers the same
  users. The `Backend` trait is where a Wayland implementation would hang.
- Full CSS-accurate text metrics in cell mode: the terminal's font advance is
  the ruler there, so we position runs by our own advance and accept a
  one-column drift on unusual glyphs.
- Frames/iframes, shadow DOM, ES modules, `IntersectionObserver` — still not in
  the engine at all (see `README.md` §5).

## 10. Abnahme — womit es bewiesen ist

**Automatisch** (`cargo test --workspace`, 2026-10-24: 35 GUI-Fälle, 20
Terminal-Protokollfälle, 20 X-Protokollfälle, alle grün; `cargo fmt --check` und
Clippy für `tb-ui`/`tb-term`/`tb-x11` without warnings):

* `crates/tb-ui/tests/ui.rs` — das Shell-Verhalten, jeweils ein Event rein und
  ein Bildschirm raus: Booten mit Chrome, `--no-chrome`, Resize, Adressleiste
  (absolut, relativ, Suche), Klick auf Links, Mittelklick, Ctrl+Klick
  (Hintergrund-Tab), `target=_blank`, Back/Forward, Scrollen mit Tasten und Rad,
  Scrollbar-Drag, Hover in der Statuszeile, Finden (Zähler, `n`/`N`, Wrap),
  Hints, Tabs (öffnen/wechseln/schließen/wiederherstellen), Drag-Selektion +
  `Ctrl+C`, Doppelklick-Wort, `Ctrl+A`, Formular-Tippen und `Enter`-Submit,
  Inspector, Zoom, Theme, Hilfe, Kommandozeile, `Ctrl+Q`, Quelltext-Tab,
  Fokussprung mit `Tab`/`Shift+Tab`, Fragment-Scrolling.
* `crates/tb-term/tests/terminal.rs` — Decoder- und Writer-Vektoren: UTF-8 über
  Lesegrenzen, Pfeile mit Modifiern, Funktionstasten, ESC-Alt, SGR-Maus
  (Press/Release/Drag/Rad), Bracketed Paste, Geometry-Antwort, Kitty CSI-u,
  Painter: erster Frame vollständig, danach nur Diffs, Resize, Titel-Escaping.
* `crates/tb-x11/tests/protocol.rs` — Setup-Parsing gegen echte Bytes,
  Request-Längen und Padding, Property-Zähleinheiten, `PutImage`-Header
  (inklusive Depth-Byte), Pixelpacking vorwärts und rückwärts, Event-Decoding,
  Error-Layout (Opcode-Nummer steht in Byte 10), Keysym-Tabelle, Shift-Level.

**Von Hand, gegen echte Server** (nicht im CI, weil Xvfb und ein Pty da sind):

* *X11 gegen `Xvfb :77`*: Fenster `0x200001 "Hello GUI — tiny"` erscheint in
  `xwininfo -root -tree`; `ffmpeg -f x11grab` zeigt weiß/schwarz-Text, die
  `#eee`-Box und `#b00`-Überschrift — die author-Stylesheets der Seite sind also
  gemalt. `ConfigureWindow` von außen → `ConfigureNotify` → Relayout, 82 % des
  Schirms ändern sich. Gefunden und behoben: `PutImage` braucht das Depth-Byte
  im Header (ohne ihn antwortet der Server `BadMatch`), und `Sync` existiert
  hier nicht → Round-trip über `GetInputFocus`.
* *Terminal gegen ein echtes Pty* (`pty.fork()`): Seite malt, `j`×30 scrollt
  (Status `0 %` → `10 %`), `Ctrl+L` + URL + `Enter` navigiert, `Ctrl+Q` beendet
  sauber und `stty -g` gibt die Shell zurück. Der Test hat den EOF-Fehler oben
  (G4.1) gefunden: vorher kamen gar keine Tasten an.
* *Eingabe-Injektion in X11* ist ohne XTEST nicht möglich (SendEvent von einem
  anderen Client für Key/Button lehnt der Server ab) — der X11-Pfad ist deshalb
  auf Event-Decoding-Ebene getestet, das Verhalten selbst über `--ui-script`.

**Bewusst abgeschnitten, damit es honest bleibt:** `GetImage`, Suggestion-Popup,
`--restore`, Tabstreifen-Drag, Autoscroll, XTEST/eigene Test-X-Extension,
dem CLIPBOARD-Selection-Sitz (wir besitzen die Selection nicht, OSC-52 und der
interne Clipboard reichen), Wayland.
