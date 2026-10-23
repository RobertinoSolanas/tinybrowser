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
| `tb-layout` (Box-Baum, Block/Inline/Flex, Intrinsic Sizing, Hit-Test) | ✅ fertig (taz.de: 3451 Boxen in **2,7 ms**) | 44 |
| `tb-js` (Lexer, Parser, Interpreter, Objekte, Promise/Async, RegExp, JSON) | ✅ fertig (taz.de: 47 KB echtes Inline-JS in **1,5 ms**, 12/12 Skripte sauber) | 112 |
| `tb-dom-bindings`, `tb-browser`, `tiny` | 🚧 als nächstes | — |

Der detaillierte Fortschritt steht in [`todolist.md`](todolist.md) (jede Aufgabe
wird beim Fertigstellen abgehakt).

**Was jetzt schon läuft** (Phase 0–7, ohne CLI):

```console
$ cargo test --workspace 2>&1 | grep 'test result' | awk '{s+=$4} END {print s" Tests, 0 Fehler"}'
400 Tests, 0 Fehler
```

Die JavaScript-Engine läuft schon in sich geschlossen — Konsolen-Ersatz:

```console
$ cargo run -q --release -p tb-js --example run -- \
    '[3,1,2].sort((a,b)=>a-b).map(x=>`#${x}`).join(' ') + ' ' + [1,2,3].reduce((a,b)=>a*b)'
=> "#1 #2 #3 6"
$ cargo run --release -p tb-js --example bench -- fixtures/js/taz-inline.js 200
      47 KB · lex   0.46 ms · parse   0.97 ms · tolerant   0.96 ms · 63 statements kept
strict parse: clean
```

(`fixtures/js/taz-inline.js` sind die 12 klassischen Inline-Skripte der echten
taz.de — GPT-Tag, Swiper-Init, Alpine-Reste — committed als Offline-Benchmark.)

**Ziel-Ausgabe** (Phase 9, wenn `tb-browser` + CLI fertig sind — die Zahlen
stammen aus den aktuellen Fixture-Läufen):

```console
$ cargo run --release -- --screenshot /tmp/taz.png https://taz.de
→ https://taz.de/  (200 OK, text/html; charset=utf-8, 422271 bytes, 118 ms)
title:    taz.de | taz.de
elements: 2893   nodes: 8032   links: 262   images: 168   scripts: 31
pipeline: parse 3 ms · style 51 ms · layout 3 ms · render …
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
| JavaScript | ✅ fertig: Lexer (UTF-8-sicher, Regex/Division, ASI), rekursiver Parser mit Pratt-artiger Präzedenzkette, Interpreter (Closures, `this`, Exceptions), Objekt-/Prototypsystem, Microtask-Queue + Timer-Queue, `async/await`, eigene Regex-Maschine (Backtracking), eigener JSON-Parser/-Schreiber |
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

Aktueller Stand — **400 Tests, 0 Fehler** (`cargo test --workspace`, inklusive
7 Doc-Tests):

```console
test result: ok. 30 passed   tb-core       (Geometrie, Farbe, DEFLATE/gzip ± python-Kreuztest)
test result: ok. 46 passed   tb-net        (URL-Vektoren, HTTP, Chunked, Charset, Cookies, Cache)
test result: ok. 11 passed   tb-net/e2e    (echter HTTP gegen tb-testkit, inkl. 304/ETag)
test result: ok. 11 passed   tb-dom        (Arena, Traversal, Serializer)
test result: ok. 34 passed   tb-html       (Tokenizer, TreeBuilder, Fuzz ohne Panics)
test result: ok. 33 passed   tb-css        (Werte, Selektoren, @media, Nesting, @scsssope-Müll)
test result: ok. 41 passed   tb-style      (Matcher, Kaskade, var/calc, UA-Sheet, taz-Fixtures)
test result: ok. 32 passed   tb-gfx        (Framebuffer, Font, PNG round-trip, CRC/Adler-Vektoren)
test result: ok. 43 passed   tb-layout     (Box-Geometrie, Umbruch, Flex, Position, Hit-Test)
test result: ok.  7 passed   tb-layout/taz (echte Seite bei 1280/768/375 px)
test result: ok. 34 passed   tb-js/lexer   (Zahlen, Strings, Templates, Regex, ASI)
test result: ok. 11 passed   tb-js/parser  (Destructuring, Klassen, ASI, Toleranz)
test result: ok. 54 passed   tb-js/interp  (Ausdrücke, Closures, this, Klassen, Promise/async)
test result: ok.  6 passed   tb-js/robust  (4000 Zufallsskripte, Tiefen-Limit, Benchmark)
test result: ok.  5 passed   tb-js/taz     (die echten Inline-Skripte von taz.de + Benchmark)
test result: ok.  2 passed   tb-testkit
```

Die taz.de-Fixtures (HTML + drei echte Stylesheets) sind committed, laufen also
**offline** und deterministisch; der Live-Test gegen `https://taz.de` kommt mit
`tb-browser` (Phase 10).


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

**JavaScript (✅ fertig):** `var/let/const` (mit TDZ-Verzicht, dokumentiert),
Funktionen, Pfeilfunktionen, Closures, `this`/`call`/`apply`/`bind`, `arguments`,
Objekte (Literal, Shorthand, computed keys, `__proto__`), Arrays (~40 Methoden),
Strings (~35 Methoden), Zahlen (`toFixed`/`toString(radix)`), `try/catch/finally`,
`throw` (jeder Wert), `for`/`for-in`/`for-of`/`while`/`do`/`switch` (auch labels),
Destructuring (verschachtelt, Defaults, Rest), Spread/Rest, Klassen (`extends`,
`super` in Methoden und Konstruktoren, statische Mitglieder, Felder), Getter/Setter,
Template-Literals (auch tagged), `?.`, `??`, `||=`/`&&=`/`??=`, `**`, `new.target`,
`Promise` (then/catch/finally/all/race/allSettled, Thenable-Adoption),
`async`/`await`, `setTimeout`/`setInterval` (virtuelle Uhr), `queueMicrotask`,
`console.*`, `JSON` (parse/stringify inkl. Replacer/Reviver), `Math`, `Date`,
`Map`/`Set`, `RegExp` (eigene Backtracking-Maschine: Klassen, Anker, Gruppen,
Lookahead, Backreferences, `g`/`i`/`m`/`s`/`u`), `Error`-Hierarchie, `eval`,
`structuredClone`.

**JS-Grenzen (bewusst):** keine Generatoren-Fortsetzung (`yield` führt den Body
einmal aus), kein ES-Modul-`import`/`export` (Loader überspringt `type="module"`,
T8.4), Regex-Lookbehind wird abgelehnt statt falsch gematcht, `Symbol`/`BigInt`
nur light, keine Worker, `await` pumpt die Event-Loop synchron (Reihenfolge kann
in exotischen Fällen abweichen), Parser-Kapitel `MAX_NEST = 200`
(tief verschachteltes Minified-JS wird Syntaxfehler statt Stack-Overflow).

**Grafik (✅ fertig):** ARGB-Framebuffer mit Alpha-Compositing und Clipping pro
Aufruf, Rect-/Linien-/Rundungsfüllung, solide/gestrichelte/gepunktete Ränder,
Bitmap-Text (5×7, Integer-Skal 1–6, synthetisch fett & schräg, Unterstreichen/
Durchstreichen), `<img>`-Platzhalter mit Berg-und-Sonne-Glyph plus `alt`-Text,
PNG-Encoder (Signatur, `IHDR`, `tEXt`, `IDAT`, `IEND`) — das Ergebnis wird von
Pythons `zlib` **und** PIL sauber gelesen.

**Layout (✅ fertig):** Block (auto-Breite, `margin: auto`, `box-sizing`,
Prozent, `min/max`, light Margin-Collapse), Inline-Flow mit Wortumbruch nach
Font-Metriken, Brechen überlanger Wörter an UTF-8-Grenzen, `white-space`
(normal/nowrap/pre/pre-wrap), `text-align` inkl. `justify`, `inline-block` und
ersetzte Elemente im Textlauf, `float` (links/rechts, umflossener Text, `clear`),
Flexbox (row/column/\*-reverse, wrap, grow/shrink, `flex-basis: auto` →
Hauptachsendimension, alle `justify-content`-Werte, `align-items`/`align-self`
inkl. `stretch`, row-/column-gap), `position: absolute|fixed|relative|sticky`
(mit `left/right/top/bottom`-Stretching), Intrinsic Sizing
(`min-content`/`max-content`/`fit-content`), `overflow`-Achsen bei der
Scrollhöhe, Hit-Test (`element_at`, `elementFromPoint`, `link_at`).

**Grenzen (bewusst):** kein CSS-Grid (taz nutzt keins), keine Bilddekodierung —
`<img>` bekommt CSS-Maße, `width`/`height`-Attribute und Aspect-Ratio, sonst
300×150; keine Web Fonts (font-family → Bitmap-Font, 5×7), keine
richtungsabhängige Typografie (RTL), kein Tabellen-/`order`-Layout, keine echte
parallele Skriptausführung, TLS = Fremdcrate.

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

## 5.2 Wie das Layout funktioniert

**Box-Baum** (`box_tree`): DOM × `display`. `display: none` wirft den Teilbaum
weg, `inline-block`/`<img>`/`<input>` werden *atomare* Inline-Boxen, und
anonyme Block-Boxen entstehen nur dort, wo CSS sie verlangt — nur *gemischte*
Kinder (Inline neben Block). Ein Container, der ausschließlich Inline-Inhalte
hat, bricht selbst die Zeilen (`inline_flow`), damit die Line-Boxes am
echten Element hängen (`p.lines`) und nicht an einem Wrapper.

**Maße** (`block`): ein Durchlauf von oben nach unten, der als einzigen Zustand
den *Containing Block* (`Cb { rect, height_definite }`) mitnimmt. Alle
Ergebnisse stehen in Dokument-Koordinaten, damit Maler und Hit-Test keine
verschachtelten Koordinatensysteme nachführen müssen. Zwei Regeln sind dabei
diskutabel wert:

* `margin: auto` verteilt den *freien* Platz — `0 auto` zentriert, `auto 0`
klemmt links an.
* `height: 100 %` ist nur dann eine Höhe, wenn der Parent eine *definite* Höhe
  hat. Sonst heißt es `auto` — und für `<img>` bedeutet `auto`: Aspect-Ratio
  halten. Ohne diese Regel wären alle taz-Bilder viewport-hoch (1024 px), was
  die Seite auf 88 000 px gestreckt hat; mit ihr sind es ~6 600 px.

**Inline-Flow** (`inline`): vor dem Umbruch wird der Inline-Teilbaum zu Tokens
geflach — Wörter, kollabierende Leerzeichen, erzwungene Umbrüche (`<br>`),
atomare Boxen, Floats. Deshalb läuft `<b>`-Text *in der Zeile des Absatzes*
(statt in einer eigenen) und behält seine Schrift. Der Pen wandert greedy, ein
zu langes Wort wird an UTF-8-Grenzen zerteilt (nicht mitten in `ü`), Zeilen
bekommen Baseline aus dem höchsten Fragment, `text-align`/`justify` verteilen am
Schluss. Gemessen wird mit `tb_gfx::Font` — **dem** Rasterizer, der später
malt: Layout und Pixel können gar nicht auseinanderlaufen.

**Flexbox** (`flex`): `flex-basis: auto` heißt „Hauptachsendimension, sonst
Inhalt“, der Rest ist Vektorrechnung: Überhang wird nach `grow`/`shrink·basis`
verteilt, `wrap` schneidet Linien, `justify-content` setzt Startposition und
Lücken, `align-items`/`align-self` (inkl. `stretch`) wirkt auf der Querachse.

**Intrinsic Sizing** (`min_content`/`max_content`/`fit_width`): Floats und
Flex-Items bekommen `clamp(max-content, min-content, verfügbar)` statt — wie in
der ersten Version — in einer 0-px-Box gemessen zu werden. Das ist der
Unterschied zwischen „jede Überschrift bricht nach einem Wort“ und einer Seite,
die aussieht.

**Out-of-Flow** (zweiter Pass): `absolute`/`fixed` werden erst gesetzt, wenn der
normale Fluss steht — inklusive Stretching zwischen `left`/`right` und
`top`/`bottom`. `float` dagegen wirkt im Fluss: der `FloatState` wandert durch
dasselbe BFC mit, damit nachfolgende Absätze den Platz wirklich umfließen.

---

## 5.3 Wie die JavaScript-Engine läuft

**Lexer** (`lexer`): ein Durchlauf über Bytes, Output sind `Lexed { token, span,
nl, prev }`. Zwei Dinge machen mehr als „Regex in Stücke schneiden“: (1) `/`
ist nur dann ein Regex-Literal, wo kein Wert zu Ende sein kann — deshalb merkt
sich der Lexer das vorherige Lexem (`Lexeme::ValueEnd` …); (2) ASI — der Lexer
zählt nicht Zeilen, er markiert nur *„vor diesem Token war ein Zeilenvorschub“*
(`nl`), und der Parser setzt das Semikolon genau dort, wo die Spec es erlaubt.
Strings und Idents sind UTF-8-sicher (Lead-Byte → Sequenzlänge, nie ein
Mid-Byte-Zerhacken), Templates werden komplett kassiert (inkl. `${}`-Zählung)
und später erneut geparst.

**Parser** (`parser`): rekursiver Abstieg mit einer Präzedenzkette pro Ebene.
Zwei bewusste Entscheidungen:

* `parse_program` ist **streng** wie ein Browser (ein Syntaxfehler tötet das
  Skript); `parse_tolerant` ist der Fremd-Skript-Modus — er verwirft nur das
  kaputte Statement und meldet es in `Problems`. Ein exotisches Vendor-Bundle
  kann so nicht die Seite killen.
* **Rekursionstiefe ist ein Feature, kein Unfall.** Minifiziertes JS verschachtelt
  hart; jeder rekursive Einstieg (`parse_statement`, `parse_expression`,
  `parse_primary`, Binding-Patterns) hole sich einen Slot über `enter()`, ab
  `MAX_NEST = 200` gibt es einen Syntaxfehler. Ohne das: Stack-Overflow → der
  ganze Browser ist weg. Gleiches gilt für `JSON.parse` (≤ 200).

**Interpreter** (`runtime` + `exprs`/`stmts`): Baum-Walk über `Rc<Expr>`/`Rc<Stmt>`
mit einer Environment-Kette (Global/Function/Block), `Value` als Vier-Varianten-
Enum plus `Rc<RefCell<Obj>>` für Objekte. `this` ist kein Wert im Scope, sondern
hängt an der Function-Umgebung (Pfeilfunktionen erben die Umgebung, also `this`
automatisch mit). Besonderheiten, die echte Seiten brauchen:

* **Completion Value.** `last_value` merkt die letzte Auswertungs-Statement-Value;
  `eval("try { x } catch (e) { 42 }")` liefert 42. Funktionskörper bekommen beim
  Aufruf ein frisches und beim Zurückkehren das restaurierte `last_value`.
* **`let`-Schleifen-Bindungen.** `for (let i = …)` forkt pro Iteration eine
  Environment (`env::fork` + `copy_values` zurück), sonst liefern
  `fs.map(f => f())` in Callback-Schleifen überall den letzten Wert — der
  klassische Bug, den jeder Zählcode hat.
* **Schritt-Budget** statt Strg+C: `tick()` zählt Schritte (Default 30 Mio.) und
  Depth; ein Feindskript kann den Browser nicht festsetzen, es wirft eine
  `RangeError`.

**Objekte & Schlüssel** (`value`): `Obj { kind, props: Vec<(Key, Prop)>, proto }`.
`own_keys()` sortiert wie JS: Integer-Indizes aufsteigend, dann
Insertions-Reihenfolge (`Object.keys({b:1,a:2,7:3}) === "7,b,a"`) — davon hängen
`JSON.stringify`, `for-in` und Framework-Magie ab. `Key` ist `Name(Rc<str>)`
oder `Num(f64)`, damit `a[0]` und `a['0']` derselbe Slot sind.

**Event-Loop** (`jobs`): Promise-Reaktionen liegen in einer VecDeque
(Microtasks), Timer in einer Liste mit **virtueller Uhr** (deterministisch, kein
`sleep` — wichtig für Tests). `await` pumpt: erst Microtasks, dann den nächst-
fälligen Timer, bis die Promise settling ist (mit Pump-Limit). Damit funktioniert
das klassische `await new Promise(r => setTimeout(r, ms))` ohne Threads; die
Reihenfolge in exotischen Fällen kann von V8 abweichen (dokumentiert).

**Regex** (`regexp`) und **JSON** (`json`) sind eigene Smalltalks: ein
Backtracking-VM mit kompilierten Instruktionen (`Char`/`Class`/`Look`/`Split`/
`Save`/`BackRef`) bzw. ein Hand-Parser mit Tiefen-Limit — kein `regex`-Crate,
kein `serde_json`.

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
  tb-layout/               Box-Baum, Block/Inline/Flex, Intrinsic Sizing, Hit-Test
  tb-gfx/                  Framebuffer, Bitmap-Font, PNG-Encoder
  tb-js/                   JS-Engine: Lexer, Parser, Interpreter, EventLoop
  tb-dom-bindings/         window/document/Element/Events für JS
  tb-browser/              Engine, Navigation, ResourceLoader, Page
  tb-cli/                  Binary `tiny`
  tb-testkit/              HTTP-Testserver für E2E-Tests
tools/xtask/               cargo junit
fixtures/                  taz.de.html, die drei echten Stylesheets und
                           js/taz-inline.js (Offline-Tests + JS-Benchmark)
```

## 8. Lizenz

MIT or Apache-2.0.
