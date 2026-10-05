# Gumroad listing

Everything needed to create/update the Gumroad product once this app is
built. Source copy is `apps.md` §T2-18 in the `tpt-electrician-nz-2` repo — if
that changes, update here too (and vice versa).

## Step-by-step: publish this listing

1. Build the Pro edition (`./build.ps1 -Pro`, once the build pipeline exists
   — see `todo.md` and mirror `tpt-app-fea-lite`/`tpt-app-cutlist-optimizer`'s
   pattern) — produces the packaged exe/zip in `release/`.
2. Log in to Gumroad → **New product** → **Digital product**.
3. Set the product name, price, and upload the packaged file (all below).
4. Paste the short description, bullets, and full description (below) into
   their respective fields.
5. Set category/tags (below) and add a cover image (see note below).
6. Publish, then copy the resulting product URL (`https://gum.co/...`).
7. Paste that URL into the `solar-checker` entry's `gumroadUrl` field in
   `tpt-electrician-nz-2\src\lib\tpt-apps-registry.ts`, and set `price: '$9'`
   on the same entry.
8. Flip that entry's `status` to `'live'` once both the free demo and this
   Gumroad listing exist.
9. Commit and push the registry repo.

## File to upload

`release\TPT-Solar-Checker-Pro.zip` (exe + dist + README, ~700 KB), produced
by `.\build.ps1 -Pro`. The exe embeds its web bundle, so the bare
`target\release\tpt-solar-checker-pro.exe` also works as a single-file
upload. Build sizes: pro wasm 117 KB gz; exe ~1.2 MB.

## Product name

```
TPT Solar Checker — Independent Solar Payback Calculator
```

## Price

**$9 USD, one-time purchase** (no subscription).

## Short description / summary field

```
Not made by a solar installer — an independent second opinion before you sign a quote.
```

## Bullets / feature list

```
Vendor-neutral
Multi-scenario comparison
Financing calculator
PDF export
One-time purchase
```

## Full description (long-form field)

```
TPT Solar Checker is an explicitly vendor-neutral solar payback calculator —
the entire value proposition is that it isn't made by anyone trying to sell
you a solar system, so the numbers can be trusted as a second opinion before
signing a quote.

Who it's for: homeowners evaluating solar quotes who don't fully trust the
payback numbers their installer's own sales calculator gives them — a real,
specific trust gap, since every installer's calculator is obviously
incentivized to make the payback period look short.

Pro edition (this download):
- Multi-scenario comparison (free is a single-property estimate)
- Financing-cost calculator
- PDF export to bring to a quote-negotiation conversation

Try the free single-property estimate first, right in your browser, no
install: tptsolutions.co.nz/tools/solar-checker

Requirements: Windows 10 (20H2+) or Windows 11. Uses the WebView2 runtime,
preinstalled on both — nothing extra to download.
```

## Category / tags

- Category: **Software > Business & Money** (or Gumroad's closest equivalent)
- Tags: `solar-calculator`, `homeowner-tools`, `renewable-energy`,
  `independent-review`, `consumer-tools`, `windows`

## Cover image / thumbnail

Not created yet. A screenshot of the multi-scenario comparison chart is the
most convincing cover once built. No stock/AI art. Candidate: full-window
shot of the Pro edition with the scenario table + cumulative cash-flow chart
visible (see `todo.md` Phase 3 — the build renders it with the default
Wellington example).

## After publishing

1. Copy the resulting product URL (`https://gum.co/...`).
2. Paste it into the `gumroadUrl` field of the `solar-checker` entry in
   `tpt-electrician-nz-2\src\lib\tpt-apps-registry.ts`, and set `price: '$9'`.
3. Set `status` to `'live'`.
4. Commit and push the registry repo.
