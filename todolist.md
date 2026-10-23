# tinybrowser — Arbeitsplan / TODO

> Ziel: Ein kleiner Webbrowser in **Rust** mit **eigenem HTML-, CSS- und
> JavaScript-Parser/Engine** (keine fremden Parser-Bibliotheken), durchgängig
> automatisiert getestet (Unit-Tests + Integrationstests + JUnit-XML-Reports).
> **Abnahmetest:** `https://taz.de` wird geladen, geparst, gestylt, gelayoutet,
> gerendert und korrekt untersucht.

Legende: `[ ]` = offen, `[x]` = DONE, `[~]` = teilweise / später verfeinert.

---

## 0. Grundlagen & Architektur (Domain Driven Design)

- [x] **T0.1** Repo-Struktur: Cargo-Workspace, `crates/*`, `tools/xtask`, `fixtures/`
- [x] **T0.2** Bounded Contexts geschnitten (siehe README "Architektur"):
      `retrieval` (net), `markup` (html+dom), `presentation` (css+style+layout+gfx),
      `scripting` (js+dom-bindings), `browser` (Orchestrierung / Application Layer)
- [x] **T0.3** `tb-core`: gemeinsame Werttypen (`Rect`, `Size`, `Point`, `Color`),
      Fehler-Taxonomie (`TbError`), `EngineConfig`, Logging/Problems-Collector
- [x] **T0.4** Entscheidungsliste "was bauen wir selbst / was nicht"
      (TLS = rustls + webpki-roots, **alles andere selbst**: HTTP/1.1, URL, HTML,
      CSS, JS, Layout, Rasterizer, PNG-Encoder, Bitmap-Font, Test-Server)

## 1. Context `retrieval` — Netzwerk (`tb-net`)

- [x] **T1.1** `url.rs`: eigener URL-Parser (RFC-3986-Schema, waswg-artige Auth-/Path-
      Normalisierung), Prozent-Encoding, `Url::join` für relative Links
- [x] **T1.2** `http.rs`: Request/Response-Modelle, HeaderMap (case-insensitive),
      Chunked-Decoding, `Connection: keep-alive`-Vorläufer, Content-Length, Redirect-Kette
- [x] **T1.3** `transport.rs`: TCP + TLS (rustls) + Zeitlimits, `http://` und `https://`
- [x] **T1.4** `client.rs`: `HttpClient` mit Redirects, User-Agent, Akzeptieren von
      `identity` (kein gzip nötig – taz liefert unkomprimiert), Fehler-Reporting
- [x] **T1.5** `charset.rs`: UTF-8 / Latin-1 / Windows-1252 Decoder + Charset-Erkennung
      aus Header und `<meta charset>`
- [x] **T1.6** `cookies.rs`: Cookie-Jar (Domain/Path/Secure/Expiry)
- [x] **T1.7** `cache.rs`: Memory-Cache mit ETag/Last-Modified + max-age
- [x] **T1.8** `tb-testkit`: eingebauter HTTP-Testserver (std::net) für Browser-E2E-Tests
- [x] **T1.9** Tests: URL-Vector-Tests, Header-/Chunked-Tests, Redirect-, Charset-,
      Cookie- und Cache-Tests, Live-Test gegen lokalen Server

## 2. Context `markup` — HTML & DOM (`tb-dom`, `tb-html`)

- [x] **T2.1** `tb-dom`: DOM als Arena (`Vec<NodeData>` + `NodeId`) mit
      Parent/Child/Sibling-Zeigern, Generation-Zählern (keine dangling refs),
      `Dom`-Aggregatwurzel, Traversal (pre-order, `descendants`, `elements`)
- [x] **T2.2** Node-Typen: Document, Doctype, Element, Text, Comment, CDATA,
      ProcessingInstruction + `Attr`-Listen, `tag_name`/`local_name`/Namespace
- [x] **T2.3** `tb-html::tokenizer`: Zustandmaschine (Data, TagOpen, EndTagOpen,
      TagName, Attribute (single/double/unquoted), SelfClosingStartTag, BogusComment,
      MarkupDeclarationOpen, Comment, Doctype, RAWTEXT/RCDATA/SCRIPT/STYLE/TITLE)
- [x] **T2.4** Character-Entity-Decoding (benannte + numerisch, hex/dez), CRLF-Normalisierung,
      NUL-Handling, Ersatzzeichen
- [x] **T2.5** `tb-html::tree_builder`: Implizite `html/head/body`, Foster-Parenting
      (Text in `table`), Self-Closing-/Void-Elements, implizites `tbody`,
      Adoption-Agency-Light (Fehlertoleranz bei `</a>`, `</p>`, `</div>`),
      `</meta>`/`</link>`-Toleranz (taz.sendet kaputte End-Tags)
- [x] **T2.6** Frame/Scopes: `in_table`-Scopes, `p`-Button-Regel, Formatting-Element-Basics
- [x] **T2.7** Serializer: `outer_html`/`inner_html` (für `innerHTML`, Debugging, Tests)
- [x] **T2.8** Tests: Tokenizer-Vector-Tests, Tree-Shaping-Tests (taz-typische Muster:
      `<meta></meta>`, verschachtelte `<a>`, `<table>`, `<script>` mit `<` und `-->`),
      Roundtrip-Tests, Fuzz-artige Random-Input-Tests (keine Panics)

## 3. Context `presentation` — CSS (`tb-css`)

- [x] **T3.1** `tokenizer.rs`: Idents, Zahlen/Dimensionen/Prozente, Strings, Hash,
      Function, URL, `Cdo`/`Cdc`, Kommentare, Escape-Sequenzen
- [x] **T3.2** `ast.rs`: `StyleSheet`, `QualifiedRule`, `Declaration`, `Value`,
      `Selector`-AST (Typ, Id, Klasse, Attribut-Operateure, Pseudo-Klasse/-Element,
      `+`, `>`, `~`, `,`), `@media`, `@font-face`, `@keyframes`, `@supports`, `@import`
- [x] **T3.3** Parser: QRule- und Block-Parsing mit Fehler-Erholung
      (ungenießbare Regeln überspringen statt Abbruch), verschachtelte Regeln
- [x] **T3.4** Wert-Typen: Farben (#hex, rgb/rgba, hsl, benannte 148), Length
      (px/em/rem/%/vw/vh/vmin/vmax), Zeit, Zahlen, Keyword-Listen, `var()`,
      Shorthand-Handling: margin/padding/inset/border/background/font*/flex*/transition
- [x] **T3.5** Media-Query-Evaluation: `screen/print/all`, `min-width`, `max-width`,
      `prefers-color-scheme` (Viewport kommt aus `EngineConfig`)
- [x] **T3.6** Custom Properties (`--x`, `var(--x, fallback)`) + `calc()` ( +-*/ und Klammern)
- [x] **T3.7** Tests: Selector-AST-VEktoren, Farb-Parsing, Shorthands, @media-Filter,
      `calc()`, Custom Properties, Toleranz kaputter CSS (taz nutzt `@scsssope`!)

## 4. Context `presentation` — Style (`tb-style`)

- [x] **T4.1** UA-Stylesheet (html5-ish: display-Typen, head/none, block, ul/ol/li,
      a:link, h1..h6, media: none, form: none-Basics)
- [x] **T4.2** Selector-Matching gegen DOM-Arena (Anker-Matching von rechts),
      `:hover`/`:focus` als State-Slots, `:root`, `:not()`, `:is()`, `:nth-child()`,
      `:first/last-child`, `:empty`, `:checked`, `:has()` (light)
- [x] **T4.3** Spezifität (a,b,c) + Import-Order + `!important` + Inline-Style +
      User-Agent-Reihenfolge → Cascade
- [x] **T4.4** Vererbte vs. nicht-vererbte Properties, Initialwerte, `inherit`/
      `initial`/`unset`/`revert`
- [x] **T4.5** `ComputedStyle`: aufgelöste Lengths (px), Font-Ketten, Farben,
      Display, Box-Modell (margin/border/padding), Flex-Eigenschaften, Positionierung,
      Zeilenhöhe, Text-Properties, `z-index`, `display:none`-Subtree-Pruning
- [x] **T4.6** Test: Cascade-Prioritäten, Vererbung, Vererbung über `!important`,
      `!important` im UA-Sheet, Media-Wechsel, Berechnung von em/rem in verschachtelten Fonts

## 5. Context `presentation` — Grafik & Font (`tb-gfx`)

- [x] **T5.1** `framebuffer.rs`: ARGB8-Framebuffer, Fill, Rect, Linien, Compositing (alpha)
- [x] **T5.2** eigener Bitmap-Font: 5x7-Glyphen (ASCII 32–126) als `.font`-Textquelle,
      Integer-Skalierung, `measure(text, size)`, `advance`
- [x] **T5.3** PNG-Encoder selbstgebaut: Chunk-Layout, CRC32, Adler-32,
      DEFLATE (stored + fixed Huffman) → gültige PNG-Dateien ohne Dependencies
- [x] **T5.4** `Image`-Dekodierung: `<img>`-Platzhalter mit alt-Text + Rahmen
      (Deko-Bilder stören die Akzeptanz nicht; dokumentiert)
- [x] **T5.5** Tests: CRC/Adler bekannter Vektoren, PNG-Signatur + Roundtrip-Check
      mit Python (`zlib`), Font-Metriken, Glyph-Vollständigkeit

## 6. Context `presentation` — Layout (`tb-layout`)

- [x] **T6.1** Box-Tree aus DOM + ComputedStyle (block/inline/inline-block/none/text)
      — `display:none`-Teilbäume fallen weg, gemischte Inhalte bekommen anonyme
      Block-Wrapper, nur rein-Inline-Container broken selbst die Zeilen (`inline_flow`)
- [x] **T6.2** Block-Layout: containing block (`Cb` + *definite*-Height-Regel),
      auto-Breiten, `margin: 0 auto`, L/TR-Breite, `box-sizing`, Padding/Border,
      `min/max-width/height`, leichte Margin- Collapse benachbarter Blöcke
- [x] **T6.3** Inline-Layout: gephasteter Inline-Flow (Wörter, kollabierende
      Leerzeichen, `<br>`, verschachtelte Inline-Elemente laufen in einer Zeile),
      Brechen überlanger Wörter an UTF-8-Grenzen, Line-Boxes mit Baseline,
      `text-align` (left/center/right/justify), `white-space`
      (normal/nowrap/pre/pre-wrap), `text-transform`
- [~] **T6.4** Flexbox-Light: row/column/\*-reverse, wrap, grow/shrink,
      `flex-basis: auto` → Hauptachsendimension, justify-content (alle Werte),
      align-items/align-self inkl. stretch, row-/column-gap
      **Bekannte Lücken:** kein echtes `order`, keine `basis %`-Edge-Cases,
      verschachtelte prozentuale Flex-Breiten auf Bulma-Seiten weichen ab
- [x] **T6.5** absolute/fixed/sticky-Positionierung (zweiter Pass inkl.
      left/right/top/bottom-Stretching), `overflow`-Achsen beachten die Scrollhöhe,
      relative Verschiebung
- [x] **T6.6** Dokument-Größe / Scroll extents + Hit-Test (`element_at`, `hit_test`,
      `link_at`) — Hit-Test liefert Text-Run und die verlinkte Elementkette
- [x] **T6.7** Intrinsic Sizing: `min-content`/`max-content`/`fit-content` für
      Floats und Flex-Items (ersetzt das alte „breite = 0“-Messen)
- [x] **T6.8** Tests: 36 Layout-Fälle (auto width, 50 %, box-sizing, zentrierte Box,
      Umbruch, nowrap, pre, align, inline-block, float, flex row/column/wrap/gap,
      justify/align, absolute/fixed/relative, none-Subtree, Hit-Test, NaN-Freiheit)
      + 7 taz-Fixtur-Tests (1280/768/375 px, endliche Geometrie, Link-Treffer,
      Reflow, Laufzeit)
- [ ] **T6.9** Nach Arbeitstag: CSS Grid (taz nutzt keins), echte Bildmetriken
      (Aspect-Ratio aus WebP-Header), `line-height`-Exponenten, Tabellen-Layout

## 7. Context `scripting` — JavaScript (`tb-js`)

- [ ] **T7.1** Lexer: Numbers, Strings (', ", `), Template-Literals mit `${}`,
      Idents (Unicode), Keyword-Liste, Operatoren, Regex-Literals, Kommentare, ASI-Vorarbeit
- [ ] **T7.2** Parser (rekursiver Absturz-Parser, Pratt für Ausdrücke):
      var/let/const, Funktionen (pfeil, method, generator-basics), Klassen (Methoden,
      `extends`, `super`), if/else, for/for-in/for-of/while/do, switch, try/catch/finally,
      throw, return/break/continue, Destructuring (light), Spread/Rest, Member/Call/New,
      Ternär, Comma, logische Operatoren, `typeof`, `void`, `delete`, `in`, `instanceof`
- [ ] **T7.3** Interpreter: Environment-Chain, Closures, `this`-Bindung,
      Werttypen (Undefined/Null/Boolean/Number/String/Object/Function/Error/Symbol/BigInt-light)
- [ ] **T7.4** Objekt-System: Property-Maps, Getter/Setter, Prototype-Ketten,
      `Object/Function/Array/String/Number/Boolean/Math/JSON/Date/Error/Promise`(Teile)
- [ ] **T7.5** Promise/Async: Job-Queue (microtask) + Task-Queue (`setTimeout`),
      `async`/`await` über Fortsetzung, `queueMicrotask`
- [ ] **T7.6** `console` (log/warn/error/info/debug/table-light) → fängt taz-Logging
- [ ] **T7.7** Robustheit: Syntaxfehler in Fremdscripten (cash.min.js, Alpine, Google GPT)
      werden protokolliert und übersprungen, kein Panic, Laufzeitfehler → `Error`-Objekt
- [ ] **T7.8** Tests: Ausdrucksauswertung,losures, this, Arrays, JSON, String-Methoden,
      try/catch, Promise-Reihenfolge, async/await, Regex-light, Regex-Methoden,
      Syntaxfehler-Toleranz, 1000-Iterationen-Benchmark

## 8. Context `scripting` — DOM-Bindings (`tb-dom-bindings`)

- [ ] **T8.1** `window`/`globalThis`, `self`, `navigator`, `location` (href/protocol/host/
      pathname/search/hash/reload/assign), `history` (pushState/light), `screen`,
      `document` (documentElement, head, body, title, charset, readyState, referrer, URL,
      hidden, visibilityState)
- [ ] **T8.2** Node/Element-API: `getElementById`, `getElementsByTagName`,
      `getElementsByClassName`, `querySelector`, `querySelectorAll`, `createElement`,
      `createTextNode`, `appendChild`, `insertBefore`, `removeChild`, `remove`, `replaceWith`,
      `parentNode/childNodes/children/firstElementChild/nextElementSibling`,
      `textContent`, `innerHTML`, `outerHTML`, `className`, `id`, `classList`
      (add/remove/toggle/contains), `getAttribute/setAttribute/removeAttribute/hasAttribute`,
      `dataset`, `style` (inline), `matches`, `closest`, `getBoundingClientRect`
- [ ] **T8.3** Event-System: `addEventListener`/`removeEventListener`/`dispatchEvent`,
      Event-Objekt (type, target, currentTarget, preventDefault, stopPropagation, bubbles),
      Bubbling/Capture, `onclick`-Attribute, DOMContentLoaded + load-Events
- [ ] **T8.4** Script-Integration: `<script>` in Dokumentreihenfolge ausführen
      (inline + `src` über `tb-net`), `type=module`/`application/ld+json` erkennen und
      nicht als klassisches JS ausführen
- [ ] **T8.5** Tests: Skript lädt DOM, `querySelectorAll`-Ergebnisse, `innerHTML`-Roundtrip,
      Event-Dispatch mit Bubbling, `classList`, DOMContentLoaded-Reihenfolge

## 9. Application Layer — Browser-Orchestrierung (`tb-browser`, `tb-cli`)

- [ ] **T9.1** `Navigation` (Aggregate): Zustand `Idle → Fetching → Parsed → Styled →
      LaidOut → Scripted → Rendered / Failed`, History-Stack, `back()/forward()`
- [ ] **T9.2** `ResourceLoader`: Subresources (CSS via `<link>`, `<style>`, Inline-`style=`),
      Skripte parallel geladen (eigener Thread-Pool), URL-Auflösung gegen Base-URL,
      Cache-Benutzung, Limits (Anzahl/Größe)
- [ ] **T9.3** `Page`: vereint Document + Stylesheets + ComputedStyles + BoxTree +
      Problems + Console-Log; `page.text()`, `page.links()`, `page.titles()`,
      `page.dump_dom()`, `page.render()`, `page.screenshot(png)`
- [ ] **T9.4** `Engine`-Fassade: `fetch`, `load(url)`, `load_html(base,url)`, `eval(js)`
- [ ] **T9.5** CLI `tiny`: `--dom --text --links --css --layout --js --console
      --screenshot out.png --width --height --no-js --timeout --verbose --repl`
- [ ] **T9.6** E2E-Tests gegen lokalen `tb-testkit`-Server: vollständiger Pageload,
      CSS-Anwendung, JS-Ausführung, Screenshot nicht leer, Navigation via Klick/History
- [ ] **T9.7** JUnit: `cargo junit` (xtask) wandelt `cargo test`-Ausgabe in
      `target/junit.xml` (Suite pro Test-Target, stdout/err, Fehlermeldungen, Skip)

## 10. Abnahme: taz.de

- [ ] **T10.1** Fixture: `fixtures/taz.de.html` (Snapshot) + Offline-Test `taz_snapshot_loads`
- [ ] **T10.2** Live-Test `taz_de_loads_correctly`: HTTP 200, `text/html`, utf-8,
      `Document::title == "taz.de | taz.de"`, > 300 Elemente, > 150 Links (absolut),
      CSS-Regeln > 0, Layout-Höhe > Viewport, Screenshot enthält Text-Pixel
- [ ] **T10.3** `cargo run -- https://taz.de` liefert Artikel-Text + Linkliste
- [ ] **T10.4** Doku im README aktualisieren (Ergebnisse, Grenzen, Benchmarks)

## 11. Feinschliff

- [ ] **T11.1** `cargo clippy` sauber (Warning-free on our crates), `cargo fmt`
- [ ] **T11.2** CI-Workflow (`.github/workflows/ci.yml`) mit `cargo test` + JUnit-Artefakt
- [ ] **T11.3** README final: Architektur, Bounded Contexts, Glossar (DDD),
      Build/Run/Test-Anleitung, Support-Matrix, bekannte Grenzen
