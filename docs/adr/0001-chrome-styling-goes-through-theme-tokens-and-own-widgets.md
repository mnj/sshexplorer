# Chrome styling goes through theme tokens and sshexplorer widgets

sshexplorer must not look like stock egui: it takes REDA's Paper look in light and dark, with an accent preset, two Densities, a UI scale and two chosen fonts, and stock egui shares one set of visuals across every widget. So every colour, space, radius, text style, shadow and duration lives in one `Tokens` value in a `theme` module, egui's `Style` is generated from it, and every control the operator sees is an sshexplorer widget in a `widgets` module built on those tokens. Code outside those two modules does not read `ui.visuals()` or `ui.spacing()`, construct a colour, pass a literal size, or call a stock egui widget that has an sshexplorer one; a source-scanning test enforces this. REDA's `theme` and `widgets` modules (its ADR-0015) are the starting code.

## Considered Options

- **Restyle egui's `Visuals` only:** far less code, but one set of visuals cannot give a primary button, a text field and a row their own recipes, so the stock look shows through.
- **egui's class-based widget themes (the `experimental` feature):** covers only a few widgets and egui documents it as open to breaking changes or removal.

## Consequences

- Tokens are named by job, never by value; the raw palette is private to `theme`. Colours follow light/dark and the Accent preset, space follows Density, text styles follow the font settings, and UI scale enlarges everything.
- Third-party widgets still get their look from the generated `Style`.
