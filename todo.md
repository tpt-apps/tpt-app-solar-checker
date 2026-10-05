# TPT Solar Checker — Build Todo

> Hub listing: `tptsolutions.co.nz/tools/solar-checker` (registry slug
> `solar-checker`, category `utility`). Spec: `apps.md` §T2-18 in the
> tpt-electrician-nz-2 repo. Base wraps `tpt-energy`. Free version:
> single-property estimate. Paid offline edition ($9 USD, Gumroad):
> multi-scenario comparison, financing calculator, PDF export. UI on
> `tpt-appfront`. Explicitly vendor-neutral positioning — must not read as
> made by a solar installer.
>
> Hub contract: `wasm-bindgen --target web` glue exposes `mount(container)`
> (called by `WasmAppRunner` after `default()` init). Standalone `trunk serve`
> mounts into the `#tpt-appfront-root` marker div instead — never
> `document.body` — so the two hosts never double-mount.

## Phase 0 — Scaffold & pipeline proof
- [x] Scaffold restructured to hub contract: `#[wasm_bindgen] pub fn mount(el)` +
      `#[wasm_bindgen(start)]` no-op unless `#tpt-appfront-root` present
      (mirrors tpt-app-microgrid-sizer; workspace root + `engine/` + `desktop/`)
- [x] `index.html` carries the `#tpt-appfront-root` marker div
- [x] Build pipeline (`build.ps1` / `build.sh` / `justfile`) mirroring
      fea-lite/cutlist-optimizer: free wasm-bindgen glue → web repo
      `public/apps/solar-checker/` via `-WebRepo`/`TPT_WEB_REPO`; `-Pro` =
      separate trunk build + webview exe + staged zip, never touching the
      free destination
- [x] Hello-world build loads through the standalone dev page (`trunk serve`)
- [x] git init + first commit

## Phase 1 — Core engine (wrap tpt-energy)
- [x] `tpt-nrg-solar` modeling reused for a single-property estimate:
      hourly clear-sky PV on the 21st of each month with NZ cloud/temperature
      tables, hourly netting against a residential load shape
- [x] Adapter crate `engine/` (`tpt-solar-engine`): system size, quote,
      tariff/feed-in/escalation, discount rate in → energy balance, payback
      period, lifetime savings, NPV/IRR out as plain structs — no UI deps,
      host-unit-testable; NPV/IRR via `tpt-nrg-lcoe`
- [x] Validation against hand-calculated reference payback scenarios
      (`engine/tests/validation.rs`: flat/degrading savings streams, break-even
      interpolation, loan amortization closed over 60 months, energy
      conservation, orientation/season physics, Wellington yield bounds)
- [x] Upstream fix found by the validation tests: `tpt-nrg-solar` hour angle
      was unwrapped, mirroring NZ mornings into the west (east/west-facing
      arrays simulated nonsense). Fixed in tpt-energy `position.rs`; a
      symmetry test here guards the regression. Added an explicit 14%
      balance-of-system derate so estimates err against the quote.

## Phase 2 — Free tier
- [x] Single-property estimate: system size, quote, tariff in → payback
      period out (default Wellington 5 kWp @ $11,500: 8.8 yrs, 6,905 kWh/yr)
- [x] Free-tier upsell copy pointing at multi-scenario comparison, financing
      calculator, PDF export
- [x] Browser-verified: mount, calculate, results/chart/table render
      correctly, city presets fill fields, invalid input shows issues

## Phase 3 — Paid desktop edition (cargo feature `pro`)
- [x] `pro` gates: multi-scenario comparison (up to 4 size/quote rows),
      financing calculator (deposit/rate/term → payment, interest,
      break-even, cumulative cash chart), PDF export (hand-rolled PDF 1.4
      writer, assumptions next to the numbers)
- [x] Pro UI: scenario comparison table (best payback highlighted),
      financing stats + cash-vs-financed chart, "Export PDF report"
- [x] Desktop shell via `tpt-appfront-webview` (dist embedded in the exe);
      Windows exe build + launch smoke test
- [x] Pro features browser-verified from the pro trunk bundle: scenario
      table, financing numbers hand-checked ($9,500 @ 10.9%/7yr →
      $162.16/mo ✓), PDF download validated (`%PDF-1.4`, all sections)
- [ ] Gumroad product from apps.md listing copy ($9), exe upload, URL into
      registry — manual step, listing copy at `GUMROAD.md`;
      upload `release\TPT-Solar-Checker-Pro.zip` (or the bare exe)

## Phase 4 — Ship & measure
- [x] Registry entry added (`utility` category,
      `wasm: { entry: '/apps/solar-checker/solar-checker.js' }`, status
      `coming-soon`) per the "Adding a finished app to the hub" runbook
- [x] Free bundle shipped into the web repo (`public/apps/solar-checker/`)
- [ ] Flip registry `status` to `'live'` + set `price: '$9'` + `gumroadUrl`
      once the Gumroad product exists
- [ ] Verify hub card/detail/runner in dev (`npm run dev`), then prod after
      deploy
- [x] WASM size budget check: free 103.7 KB gz (256 KB raw), pro 117 KB gz
      (297 KB raw) — both well under the ~600 KB budget
