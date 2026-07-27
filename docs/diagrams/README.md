# Diagram sources for `OVER_VIEW.html`

`OVER_VIEW.html` carries its diagrams as **inlined SVG**, so the page opens
offline with no network, no CDN and no JavaScript. The cost of that is the
usual one: nobody hand-edits an SVG full of absolute coordinates. So the
Mermaid source lives here, and `regen.py` is the one-way path from source to
page.

```sh
python3 docs/diagrams/regen.py     # re-renders every diagram into OVER_VIEW.html
```

Edit a `.mmd`, run that, review the diff. **Don't hand-edit the `<svg>` blocks
in `OVER_VIEW.html`** — the next run overwrites them.

Sources map to the page in sorted filename order: `01-*.mmd` becomes the
element with `id="ovd0"`, `02-*.mmd` becomes `ovd1`, and so on. Adding a
diagram means adding both a `.mmd` here and an `<svg id="ovdN">` placeholder
in the page.

## Files

| File | Diagram |
|---|---|
| `01-system-map.mmd` | External APIs → backend → Mongo → CLI/TUI |
| `02-backend-modules.mmd` | Every backend file and what it imports |
| `03-boot-sequence.mmd` | `index.ts` top to bottom |
| `04-live-path.mmd` | Twitch push + YouTube poll converging on `live-state.ts` |
| `05-sync-path.mmd` | The shared `syncFrom*` skeleton and its two gates |
| `06-data-model.mmd` | The four collections and their relationships |
| `07-cli-internals.mmd` | `main.rs` → `routes.rs` → `config.rs` |
| `08-watch-loop.mmd` | `watch.rs`'s poll loop and `apply()` |
| `09-tui-loop.mmd` | The TUI's `select!` core |

`mermaid-config.json`, `puppeteer-config.json` and `svgo.config.mjs` are the
render settings. Only `regen.py` reads them.

## Regenerating produces a big diff even when nothing changed

`mermaid-cli` lays diagrams out by *measuring rendered text in a headless
Chromium*, and those measurements vary slightly between runs. Two consecutive
regens with no source edit produce identical elements, identical attribute
names and identical text — verified — but different coordinates, so
`OVER_VIEW.html` churns by thousands of lines regardless.

Practical consequences:

- **Only run `regen.py` when you actually changed a `.mmd`.** Running it "to
  check" costs you a meaningless 400KB diff.
- **Don't review the regenerated diff line by line.** Review the `.mmd` diff,
  which is small and readable, then open the page and look at it.
- Because of the churn, `OVER_VIEW.html` is best committed on its own rather
  than mixed into a commit with code changes.

## Two things that will bite you

**Write HTML entities, not literal characters.** Mermaid renders labels via
`innerHTML`, so `&lt;`, `&gt;` and `&amp;` survive correctly, while a literal
`<` gets parsed as a tag and silently vanishes — `Option<Command>` would
render as `Option`. This is also why the sources are `.mmd` files rather than
fenced blocks pasted back into the HTML: a browser decodes entities in a
`<pre>` *before* Mermaid ever sees them, so the browser path is strictly worse
at this than `mermaid-cli` is.

**Colours are theme tokens by the time they reach the page.** `regen.py`
rewrites the hexes `mermaid-cli` bakes into each SVG's `<style>` block into
`var(--ink-soft)` and friends, which resolve against `OVER_VIEW.html`'s
`:root`. That is what makes the diagrams follow the light/dark toggle. Node
fills set by `classDef` are deliberately left alone — a tinted fill with dark
text is its own self-contained ground and reads correctly on either theme.
If you add a `classDef`, follow that convention or the node will be
unreadable in one of the two themes.
