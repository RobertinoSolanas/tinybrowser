# tinybrowser

Ein kleiner Webbrowser in **Rust** — mit **eigenem HTML-, CSS- und
JavaScript-Parser/-Engine**. Kein `html5ever`, kein `serde_json`, kein
`regex`, keine `image`-Bibliothek: Tokenizer, Parser, DOM, Kaskade, Layout,
Rasterizer, PNG-Encoder, Bitmap-Font und sogar der HTTP-Testserver sind
selbstgeschrieben.

Die einzigen Fremd-Abhängigkeiten sind **TLS** (`rustls`) und die
**Root-Zertifikate** (`webpki-roots`) — X.509/TLS 1.3 selbst zu bauen wäre die
falsche Art von "tiny".

### Baustand

| Baustein | Status | Tests |
|---|---|---|
| `tb-core` (Kernel, DEFLATE/gzip, CRC32) | ✅ fertig | 30 |
| `tb-net` (URL, HTTP/1.1, TLS, Charset, Cookies, Cache) | ✅ fertig | 46 + 11 E2E |
| `tb-dom` (Arena-DOM, Serializer) | ✅ fertig | 11 |
| `tb-html` (Tokenizer, TreeBuilder, Entities) | ✅ fertig (taz.de: 2893 Elemente in 3 ms) | 34 |
| `tb-css` (Parser, Selektoren, Werte, `@media`, `calc/var`) | ✅ fertig (260 KB taz-CSS in 5 ms) | 34 |
| `tb-style` (UA-Sheet, Matching, Kaskade, `ComputedStyle`) | ✅ fertig (taz.de: 2893 Elemente × 3193 Regeln in 51 ms) | 41 |
| `tb-gfx` (Framebuffer, 5×7-Bitmap-Font, PNG-Encoder) | ✅ fertig (PNG von PIL unabhängig verifiziert) | 33 |
| `tb-layout`, `tb-js`, `tb-dom-bindings`, `tb-browser`, `tiny` | 🚧 als nächstes | — |

Der detaillierte Fortschritt steht in [`todolist.md`](todolist.md) (jede Aufgabe
wird beim Fertigstellen abgehakt).

**Was jetzt schon läuft** (Phase 0–4, ohne CLI):

```console
$ cargo test --workspace 2>&1 | grep 'test result' | tail -3
test result: ok. 20 passed   (tb-style lib)
test result: ok. 17 passed   (tb-style/tests/cascade.rs)
test result: ok.  4 passed   (tb-style/tests/fixtures_taz.rs)
```

**Ziel-Ausgabe** (Phase 9, wenn `tb-browser` + CLI fertig sind — die Zahlen
stammen aus den aktuellen Fixture-Läufen):

```console
$ cargo run --release -- --screenshot /tmp/taz.png https://taz.de
→ https://taz.de/  (200 OK, text/html; charset=utf-8, 422271 bytes, 118 ms)
title:    taz.de | taz.de
elements: 2893   nodes: 8032   links: 262   images: 168   scripts: 31
pipeline: parse 3 ms · style 51 ms · layout … · render …
```

---

## 1. Schnellstart

```bash
cargo build --release              # alles bauen
cargo test                         # alle Unit- + Integrationstests
cargo run  -- https://taz.de       # echte Seite laden
cargo run  -- --screenshot /tmp/taz.png https://taz.de
cargo junit                        # Tests + JUnit-XML (target/junit.xml)
```

### CLI

```
tiny [OPTIONS] [URL]

  --dom              DOM-Baum ausgeben (Elemente, ids, Klassen, hrefs)
  --text             sichtbaren Text ausgeben (script/style werden übersprungen)
  --links            alle Links, absolut aufgelöst
  --css              geladene Stylesheets + Regelanzahl
  --layout           Box-Baum mit Maßen
  --js               JavaScript ausführen und Console-Ausgabe zeigen
  --console          Console-Ausgaben der Seite zeigen
  --screenshot FILE  PNG-Screenshot (eigener Rasterizer + PNG-Encoder)
  --width N          Viewport-Breite (Default 1280)
  --height N         Viewport-Höhe (Default 1024)
  --no-js            Skripte aus
  --no-css           CSS aus
  --timeout MS       Netzwerk-Timeout
  --repl             interaktive JavaScript-Konsole gegen die geladene Seite
  --serve DIR        lokaler Testserver auf 127.0.0.1 (für Demos)
```

Beispiele:

```bash
cargo run --release -- --links https://taz.de | head
cargo run --release -- --repl fixtures/taz.de.html   # document.title usw.
cargo run --release -- --screenshot /tmp/taz.png --height 4000 https://taz.de
```

---

## 2. Architektur (Domain Driven Design)

### 2.1 Bounded Contexts

| Context | Crates | Verantwortung (Ubiquitous Language) |
|---|---|---|
| **Shared Kernel** | `tb-core` | `Rect/Size/Sides`, `Color`, `TbError`+`Phase`, `EngineConfig`, `Problems`, DEFLATE/gzip/PNG-Bausteine |
| **retrieval** | `tb-net` | `Url`, `Request/Response`, `HttpClient`, `CookieJar`, `HttpCache`, `Charset` |
| **markup** | `tb-dom`, `tb-html` | `Dom` (Arena), `Node`, `Element`, `Token`, `TreeBuilder` |
| **presentation** | `tb-css`, `tb-style`, `tb-layout`, `tb-gfx` | `StyleSheet`, `Declaration`, `Selector`, `ComputedStyle`, `BoxTree`, `Framebuffer` |
| **scripting** | `tb-js`, `tb-dom-bindings` | `Value`, `Ast`, `Interpreter`, `EventLoop`, `window/document`-Bindungen |
| **browser** (Application Layer) | `tb-browser`, `tb-cli` | `Engine`, `Navigation` (Aggregate), `ResourceLoader`, `Page` |
| Test-Infrastruktur | `tb-testkit`, `tools/xtask` | In-Process-HTTP-Server, JUnit-XML-Reporter |

Abhängigkeiten fließen nur in eine Richtung (shared kernel ← Domänen ←
Anwendung); `tb-browser` ist der einzige Ort, an dem die Pipelines zusammenlaufen.

### 2.2 Pipeline

```
        ┌──────────────┐   ┌────────────┐   ┌──────────────┐
 URL ──▶│ tb-net       │──▶│ tb-html    │──▶│ tb-css       │
        │ HTTP/1.1+TLS │   │ Tokens→Dom │   │ Stylesheets  │
        └──────────────┘   └─────┬──────┘   └──────┬───────┘
                                 │                 │
                                 ▼                 ▼
        ┌──────────────┐   ┌────────────┐   ┌──────────────┐
 PNG ◀──│ tb-gfx       │◀──│ tb-layout  │◀──│ tb-style     │
        │ Raster+Font  │   │ BoxTree    │   │ Cascade      │
        └──────────────┘   └────────────┘   └──────────────┘
                                 ▲
                        ┌────────┴────────┐
                        │ tb-js + bindings│  <script>, addEventListener,
                        │ (EventLoop)     │  setTimeout … verändert das DOM
                        └─────────────────┘
```

Der Zustand einer Navigation ist ein **Aggregate** (`Navigation`) mit expliziten
Phasen: `Idle → Fetching → Parsed → Styled → LaidOut → Scripted → Rendered`
(bzw. `Failed`). Alles, was eine Phase ungültig macht (Skript ändert DOM),
markiert die folgenden Phasen als dirty; `Page::render()` legt bei Bedarf
nach.

### 2.3 Fehlerbehandlung = eigener Domänenbegriff

`Problems` (in `tb-core`) sammelt *nicht-fatale* Diagnosen pro Phase
(CSS-Regel kaputt, JS `SyntaxError`, unbekanntes `@scsssope`). Eine echte
Browser-Seite ist nie "fehlerfrei"; der Browser bricht deshalb nicht ab, sondern
render weiter und die Tests prüfen, *welche* und *wie viele* Probleme auftreten.

---

## 3. Was selbst gebaut ist (und was nicht)

| Komponente | Implementierung |
|---||
| HTML-Tokenizer | ✅ fertig: Zustandsmaschine (Data, TagOpen, Attr*, BogusComment, RawText, CDATA …) |
| HTML-TreeBuilder | ✅ fertig: implizites `html/head/body/tbody`, `<p>`-Regeln, Void-Elemente, Foreign-Content (`svg`/`math`), Fehler-Erholung |
| DOM | ✅ fertig: Arena mit Generationen-Zählern (`NodeId{idx,gen}`), Serializer |
| CSS-Parser | ✅ fertig: Selektoren, Deklarationen, `@media`, `@supports`, `@font-face`, `@import`, Nesting, Fehler-Erholung |
| CSS-Werte | ✅ fertig: Farben (hex/rgb/hsl/148 Keywords), Lengths (px/em/rem/%/vw/vh/…), `var()`, `calc()`, `min()/max()/clamp()` |
| Kaskade | ✅ fertig: Spezifität, `!important`, Ursprung, Vererbung, UA-Stylesheet |
| Layout | 🚧 geplant: Block, Inline (Wortumbruch), Flexbox-light, absolut/fixed, `overflow` |
| Grafik | ✅ fertig: Framebuffer (Alfa-Compositing, Clipping, Striche, Rundungen), eigener **PNG-Encoder** (CRC32 + Adler32 + unser DEFLATE) **und** ein kleiner PNG-Leser für Tests |
| Font | ✅ fertig: hand gezeichneter 5×7-Bitmap-Font (ASCII 32–126 + äöüß € – — … “ ” •), Integer-Skalierung, fett/kursiv synthetisch |
| DEFLATE/gzip/zlib | ✅ fertig: Encoder **und** Decoder (RFC-1951/1950/1952, python-zlib-Kreuztest) |
| HTTP/1.1 | ✅ fertig: Request/Response, HeaderMap, Chunked, Redirects, Cookies, Cache, `data:`-URLs |
| URL | ✅ fertig: RFC-3986 + WHATWG-Relativreferenzen (`//host`, `?q`, `#f`) |
| JavaScript | 🚧 geplant: Lexer, Parser (Pratt), Interpreter, Objekte/Prototypen, Closures, `this`, Promise/Job-Queue, `setTimeout`-Task-Queue, `async/await` |
| DOM-API für JS | 🚧 geplant: `document.*`, `Element.*`, `classList`, `dataset`, Events, `querySelector(All)` |
| TLS | ❌ `rustls` + `webpki-roots` |
| Bilder (`<img>`) | ❌ keine Dekodierung; Platzhalter mit `alt`-Text und Rahmen |
| Grid-Layout | ❌ Fallback auf Block |

---

## 4. Teststrategie (automatisch, JUnit-kompatibel)

* **Unit-Tests** in jedem Crate (`#[cfg(test)]`, direkt neben dem Code).
* **Integrationstests** pro Crate in `tests/` (Parser-Vektoren, Toleranz
  kaputten Inputs, Fuzz-artige Zufallsmarkup-Schleifen: keine Panics).
* **E2E-Tests** mit `tb-testkit`: ein echter HTTP-Server im Prozess liefert
  `404`, Redirect-Ketten, Chunked, Gzip, Cookies, Cache/`304`, CSS, JS —
  der komplette Browser läuft dagegen, inklusive Screenshot-Prüfung.
* **Abnahmetest** `crates/tb-browser/tests/taz_de.rs`: taz.de (Snapshot **und**
  Live-Netz) — siehe Abschnitt 6.
* **JUnit**: `cargo junit` (xtask) erzeugt `target/junit.xml` mit einer
  `<testsuite>` pro Test-Target, inkl. stdout/stderr und Failure-Messages.

Aktueller Stand — **244 Tests, 0 Fehler** (`cargo test --workspace`):

```console
test result: ok. 30 passed   tb-core      (Geometrie, Farbe, DEFLATE/gzip ± python-Kreuztest)
test result: ok. 46 passed   tb-net       (URL-Vektoren, HTTP, Chunked, Charset, Cookies, Cache)
test result: ok. 11 passed   tb-net/e2e   (echter HTTP gegen tb-testkit, inkl. 304/ETag)
test result: ok. 11 passed   tb-dom       (Arena, Traversal, Serializer)
test result: ok. 34 passed   tb-html      (Tokenizer, TreeBuilder, Fuzz ohne Panics)
test result: ok. 34 passed   tb-css       (Werte, Selektoren, @media, Nesting, @scsssope-Müll)
test result: ok. 41 passed   tb-style     (Matcher, Kaskade, var/calc, UA-Sheet, taz-Fixtures)
test result: ok. 33 passed   tb-gfx       (Framebuffer, Font, PNG round-trip, CRC/Adler-Vektoren)
test result: ok.  2 passed   tb-testkit
```

Dazu drei Doc-Tests. Die taz.de-Fixtures (HTML + drei echte Stylesheets) sind
committed, laufen also **offline** und deterministisch; der Live-Test gegen
`https://taz.de` kommt mit `tb-browser` (Phase 10).


---

## 5. Was die Engine heute kann (Support-Matrix)

**HTML:** Vollständige Tokenizer-Zustände inkl. RawText (`script`, `style`,
`title`, `textarea`), Character-Entities (benannt + numerisch),
Fehler-Erholung für `</meta>`/`</link>` (taz sendet das!), `<<>>`, `<div/>`,
nicht geschlossene Kommentare, `svg`/`math`-Namespaces.

**CSS (✅ fertig):** Selektoren (Typ, Id, Klasse, Attribute mit allen
Operatoren, Pseudo-Klassen inkl. `:not/:is/:where/:has/:nth-child`,
`::before/after`), Kombinatoren, `@media`-Auswertung
(Breite/Höhe/orientation/prefers-color-scheme/reduced-motion), Custom
Properties, `calc()`, `min()/max()/clamp()`, Shorthands (`margin`, `padding`,
`border*`, `background`, `font`, `flex`, `inset`, `gap`).

**Kaskade (✅ fertig):** UA-Stylesheet in unserem eigenen CSS-Dialekt,
Rechts-nach-links-Matching mit Index über das rechte Selektor-Ende, Origins
UA < Autor < `style=""` (mit `!important`-Umkehr), Vererbung nach CSS-Regeln,
`var()` mit Fallback, `calc()` → lineare Länge (`pct% + px`), `::before`/
`::after`-`content`.

**JavaScript (🚧 geplant):** `var/let/const`, Funktionen, Pfeilfunktionen, Closures,
`this`/`call`/`apply`/`bind`, Objekte, Arrays, Strings, Zahlen, `try/catch/
finally`, `throw`, `for`/`for-in`/`for-of`/`while`/`switch`, `class` (Basis),
Getter/Setter, Template-Literals, Spread/Rest, Destructuring (Basis),
`Promise`+Microtask-Queue, `async/await`, `setTimeout`, `console.*`, `JSON`,
`Math`, `Date`, `RegExp` (Basis).

**Grafik (✅ fertig):** ARGB-Framebuffer mit Alpha-Compositing und Clipping pro
Aufruf, Rect-/Linien-/Rundungsfüllung, solide/gestrichelte/gepunktete Ränder,
Bitmap-Text (5×7, Integer-Skal 1–6, synthetisch fett & schräg, Unterstreichen/
Durchstreichen), `<img>`-Platzhalter mit Berg-und-Sonne-Glyph plus `alt`-Text,
PNG-Encoder (Signatur, `IHDR`, `tEXt`, `IDAT`, `IEND`) — das Ergebnis wird von
Pythons `zlib` **und** PIL sauber gelesen.

**Layout (🚧 geplant):** Block, Inline mit Wortumbruch, `inline-block`, Flexbox (row/column,
wrap, grow/shrink/basis, `justify-content`, `align-items`, `gap`),
`position: absolute|fixed|relative`, `overflow`, `z-index`-Reihenfolge.

**Grenzen (bewusst):** kein CSS-Grid, keine Bilddekodierung, keine Web Fonts
(font-family → Bitmap-Font), keine richtungsabhängige Typografie, `float` nur
ansatzweise, keine echte parallele Skriptausführung, TLS = Fremdcrate.

---

## 5.1 Wie die Kaskade funktioniert

Reihenfolge pro Element, von billig nach teuer:

1. **Flachklopfen** — `@media` einmal gegen den Viewport prüfen, verschachtelte
   Regeln herausziehen → flache Liste `(Selektor, Deklarationen, Ursprung,
   Quellenzahl)`.
2. **Index** — jede Regel liegt in genau einem Eimer, benannt nach dem *rechten*
   Endstück ihres Selektors (`#id` → Eimer `id:x`, sonst Klasse, sonst Typ,
   sonst global). Ein Element sieht ~4 Eimer statt 3193 Regeln.
3. **Match** — rechts nach links über die `parts`-Liste, ohne Allokation;
   `:not()`/`:is()`-Argumente liegen gecacht (thread-local).
4. **Gewinnen** — pro Property bleibt ein Kandidat: Vergleich über
   `(Rang[Ursprung+!important], Spezifität, Quellenreihenfolge)`.
   Rang: `UA normal 1 < Autor normal 2 < style="" 3 < Autor !import 4 <
   style="" !import 5 < UA !import 6`.
5. **Anwenden** — Shorthand-Expansion und Auflösung relativer Längen nach
   `L::{Auto, Px, Linear{pct, fixed}}`. `Linear` heißt: „zur Layout-Zeit
   `pct/100 · basis + fixed`", weil das Prozent-Basis (Containing Block) erst im
   Layout bekannt ist.

Der `L`-Typ ist der Grund, warum `calc(100% - 32px)`, `50vw` und `2rem` schon im
Cascade fertig durchgerechnet werden können, ohne das Layout zu kennen — und
warum `font-size: 1.5em` die echte Parent-Schriftgröße benutzt statt zu raten.

---

## 6. Abnahmetest: taz.de

Der Test läuft in zwei Varianten:

* `taz_snapshot_loads_correctly` — `fixtures/taz.de.html` (committed, offline,
  deterministisch).
* `taz_de_loads_correctly` — echter HTTP-Request gegen `https://taz.de`
  (fallen gelassen auf den Snapshot, wenn gerade kein Netz verfügbar ist; dann
  mit Warnung, damit der Grund sichtbar bleibt).

Prüfungen (Auszug):

* HTTP 200, `Content-Type: text/html`, Charset `utf-8`
* `document.title == "taz.de | taz.de"`, `<html lang="de">`
* > 2000 Elemente, > 150 Links (`href` absolut aufgelöst), > 100 Bilder
* Stylesheets werden geladen und angewendet (`ComputedStyle` von `body`, `h1`, `a`)
* Layout: Dokumenthöhe > Viewport, erste Boxes mit positiver Breite
* Screenshot: PNG-Signatur korrekt, nicht-weiße Pixel vorhanden
* Keine Panics; nur *bekannte* Problem-Kategorien (z. B. Fremd-Skripte, die
  unsere Engine bewusst ablehnt)

---

## 7. Workspace

```
Cargo.toml                 Workspace, nur rustls + webpki-roots als Dependencies
todolist.md                Arbeitsplan (Abhaken = DONE)
crates/
  tb-core/                 Shared Kernel: Geometrie, Farbe, Fehler, Config,
                           Problems, DEFLATE/gzip/zlib, CRC32/Adler32
  tb-net/                  URL, HTTP/1.1, TLS-Transport, Charset, Cookies, Cache
  tb-dom/                  DOM-Arena, Traversal, Serializer
  tb-html/                 HTML-Tokenizer + TreeBuilder + Entities
  tb-css/                  CSS-Parser (Selektoren, Werte, @media), calc/var
  tb-style/                UA-Sheet, Matching, Kaskade, ComputedStyle
  tb-layout/               BoxTree: Block/Inline/Flex/absolut, Hit-Test
  tb-gfx/                  Framebuffer, Bitmap-Font, PNG-Encoder
  tb-js/                   JS-Engine: Lexer, Parser, Interpreter, EventLoop
  tb-dom-bindings/         window/document/Element/Events für JS
  tb-browser/              Engine, Navigation, ResourceLoader, Page
  tb-cli/                  Binary `tiny`
  tb-testkit/              HTTP-Testserver für E2E-Tests
tools/xtask/               cargo junit
fixtures/                  taz.de.html + taz-Stylesheets (Offline-Tests)
```

## 8. Lizenz

MIT or Apache-2.0.
