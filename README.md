# TPT Solar Checker

An explicitly **vendor-neutral** solar payback calculator for New Zealand
homeowners — the entire value proposition is that it isn't made by anyone
trying to sell a solar system, so the numbers can be trusted as a second
opinion before signing a quote.

- **Free edition** (in browser at
  [tptsolutions.co.nz/tools/solar-checker](https://tptsolutions.co.nz/tools/solar-checker)):
  single-property estimate — system size, quote, tariff in; payback period,
  lifetime savings out.
- **Pro edition** ($9 USD, Windows desktop via Gumroad): multi-scenario
  comparison (quotes side by side), financing calculator (loan vs cash),
  PDF report export for the quote negotiation.

Spec: `apps.md` §T2-18 in the `tpt-electrician-nz-2` repo. Listing copy:
`GUMROAD.md`. Build status: `todo.md`.

## Layout

```
src/         wasm UI (tpt-appfront-dom) — lib.rs carries the hub contract
engine/      tpt-solar-engine: pure payback model over tpt-energy, host-testable
desktop/     tpt-solar-desktop: WebView2 shell for the Pro exe
dist/hub/    free wasm bundle for the website (build output)
release/     staged Pro exe package + zip (build output)
```

The engine speaks kW / kWh / NZD and converts at the boundary into the
MW / MWp `tpt-energy` uses. Physics: `tpt-nrg-solar` (Ineichen clear-sky,
NOCT thermal derating, inverter clipping) sampled hourly on the 21st of each
month with typical NZ cloud/temperature by month, then an explicit 14%
balance-of-system derate so the estimate errs against the quote. Money:
self-consumed vs exported split from an hourly load shape (export earns the
much lower feed-in rate), price escalation, 0.5%/yr degradation, NPV/IRR
via `tpt-nrg-lcoe`.

> Upstream note: `tpt-nrg-solar` had an hour-angle wrap bug (NZ mornings
> placed the sun in the west, so east/west-facing arrays simulated
> nonsense). Fixed in `tpt-energy` `position.rs` (solar hour wrapped to
> [0, 24)); `engine/tests/validation.rs` guards the regression with an
> east/west symmetry test.

## Development

```pwsh
just dev        # trunk serve — standalone dev page (mounts into #tpt-appfront-root)
just test       # engine tests (free + pro) + wasm typecheck
just check      # typecheck free, pro, and desktop
just build      # free wasm bundle -> dist\hub (gzip size report)
just pro        # pro trunk bundle + WebView2 exe + release\ zip
```

Deploy the website bundle (copies the two free files into the hub repo):

```pwsh
$env:TPT_WEB_REPO = "D:\Programming\2 WIP\TPT Electrician\tpt-electrician-nz-2"
.\build.ps1
```

Two-edition invariant: the website always gets the FREE bundle.
`build.ps1 -Pro` builds a separate trunk bundle for the exe and never
writes the web-repo destination; `dist\hub` must never be built with
`--features pro`. Also don't run `trunk serve` while `build.ps1 -Pro`
runs — serve rebuilds its own (free) bundle into the same `dist\`.

## Hub contract

The website's `WasmAppRunner` dynamic-imports the wasm-bindgen `--target
web` glue, awaits its default export, then calls `mount(container)`.
Standalone (`trunk serve`) mounts into the `#tpt-appfront-root` marker div
that only `index.html` provides, so the two hosts never double-mount.

## Validation

`cargo test -p tpt-solar-engine` runs hand-calculated reference scenarios
(annual savings, break-even interpolation, loan amortization closed over 60
months, energy conservation, orientation/season physics, Wellington yield
bounds). The money math is separated from the physics precisely so it can
be hand-checked; see `engine/tests/validation.rs`.
