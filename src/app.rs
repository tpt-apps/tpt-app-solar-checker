//! Free/Pro WASM edition UI (tpt-appfront-dom).
//!
//! Rendering architecture (mirrors the microgrid-sizer/fea-lite split):
//! - the **input panel is mounted once** (`tpt_appfront_dom::mount`) —
//!   typing never loses focus; field values are read straight from the DOM
//!   when Calculate is pressed;
//! - the **results sections re-render fine-grained** via
//!   `tpt_appfront_dom::render` driven by outcome signals;
//! - the **charts are raw web-sys `<canvas>` elements** (the DOM backend
//!   has no canvas node kind) appended into placeholder containers and
//!   redrawn from effects that watch the outcome signals.
//!
//! Two DOM-backend quirks the markup works around (same lessons fea-lite
//! learned): a bare text node can't carry a `class`, so every styled piece
//! of text is wrapped in its own container; and the reconciler force-syncs
//! an input's DOM value to the UITree's static default on any re-render,
//! which is why all inputs live in the mounted-once shell and are only ever
//! written to directly.

use std::rc::Rc;

use tpt_appfront_core::{create_effect, Signal, UITree};
use tpt_solar_engine::{
    estimate, Household, PaybackReport, Site, SolarSystem, Tariff, MONTH_NAMES, MONTHS_PER_YEAR,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::chart;

#[cfg(feature = "pro")]
use tpt_solar_engine::{analyze_financing, compare_scenarios, LoanTerms};
#[cfg(feature = "pro")]
use tpt_solar_engine::{pdf, Scenario};

/// One estimate's current content.
#[derive(Clone)]
enum EstimateOutcome {
    Idle,
    Invalid(Vec<String>),
    Solved(Rc<PaybackReport>),
}

/// The scenario-comparison section's current content (Pro).
#[cfg(feature = "pro")]
#[derive(Clone)]
enum ScenarioListOutcome {
    Idle,
    Invalid(Vec<String>),
    Solved(Rc<Vec<tpt_solar_engine::ScenarioOutcome>>),
}

/// The financing section's current content (Pro): the terms as entered plus
/// the results.
#[cfg(feature = "pro")]
#[derive(Clone)]
enum LoanOutcome {
    Idle,
    Invalid(Vec<String>),
    Solved(Rc<LoanTerms>, Rc<tpt_solar_engine::LoanResult>),
}

#[derive(Debug, Clone)]
enum Msg {
    /// Run the single-property payback estimate.
    Calculate,
    /// Fill the location inputs from a built-in city preset.
    CityPreset(usize),
    /// Open a TPT Solutions URL — new tab in the browser, or via the
    /// desktop host's system-browser IPC inside the Pro shell. See
    /// [`visit_site`].
    VisitSite(String),
    /// (pro) compare the sidebar scenarios side by side.
    #[cfg(feature = "pro")]
    CompareScenarios,
    /// (pro) run the loan-vs-cash financing analysis.
    #[cfg(feature = "pro")]
    RunFinancing,
    /// (pro) download the PDF quote-negotiation report.
    #[cfg(feature = "pro")]
    ExportPdf,
}

const BASE_CSS: &str = r#"
.sc-app{--sc-bg:#fbf8f3;--sc-surface:#ffffff;--sc-panel:#f6f1e7;--sc-fg:#241c12;--sc-muted:#6f6555;--sc-border:#e7dfd0;--sc-accent:#b26a00;--sc-accent-2:#8a5200;--sc-accent-soft:#f7ead2;--sc-accent-fg:#ffffff;--sc-err:#b3261e;--sc-err-bg:#fbeeec;--sc-ok:#2e7d5b;--sc-grid:rgba(60,50,30,.05);--sc-shadow:0 1px 2px rgba(40,30,10,.05),0 1px 3px rgba(40,30,10,.07);max-width:1360px;margin:0 auto;background:var(--sc-bg);color:var(--sc-fg);font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif;font-size:14.5px;line-height:1.55;padding:0 4px 32px}
@media(prefers-color-scheme:dark){.sc-app{--sc-bg:#12100b;--sc-surface:#1a1712;--sc-panel:#211c14;--sc-fg:#efe9dd;--sc-muted:#a89e8c;--sc-border:#2f2921;--sc-accent:#f0a232;--sc-accent-2:#ffc061;--sc-accent-soft:#2c2210;--sc-accent-fg:#1c1503;--sc-err:#f2b8b5;--sc-err-bg:#2c1a1c;--sc-ok:#5cbf95;--sc-grid:rgba(255,255,255,.05);--sc-shadow:0 1px 2px rgba(0,0,0,.3)}}
.sc-app *{box-sizing:border-box}
.sc-app h2{font-size:1.36rem;margin:0;font-weight:700;letter-spacing:-.01em}
.sc-app h3{font-size:1rem;margin:0 0 6px;font-weight:700}

.sc-header{display:flex;align-items:flex-start;gap:14px;padding:22px 2px 18px;border-bottom:1px solid var(--sc-border);margin-bottom:22px}
.sc-brand{flex:0 0 auto;width:42px;height:42px;border-radius:11px;background:linear-gradient(135deg,var(--sc-accent),var(--sc-accent-2));display:flex;align-items:center;justify-content:center;color:var(--sc-accent-fg);font-weight:800;font-size:.78rem;letter-spacing:.01em;box-shadow:var(--sc-shadow)}
.sc-title-group{flex:1;min-width:0}
.sc-title-row{display:flex;align-items:center;gap:10px;flex-wrap:wrap}
.sc-chip{display:inline-flex;align-items:center;padding:3px 10px;border-radius:999px;font-size:.66rem;font-weight:800;letter-spacing:.05em;text-transform:uppercase;background:var(--sc-accent-soft);color:var(--sc-accent)}
.sc-sub{color:var(--sc-muted);margin:5px 0 0;font-size:.88rem}

.sc-shell{display:grid;grid-template-columns:minmax(320px,380px) 1fr;gap:22px;align-items:start}
@media(max-width:900px){.sc-shell{grid-template-columns:1fr}}
.sc-sidebar{display:flex;flex-direction:column;gap:16px;position:sticky;top:16px;max-height:calc(100vh - 32px);overflow-y:auto;padding:2px 4px 12px 2px;counter-reset:sc-step}
@media(max-width:900px){.sc-sidebar{position:static;max-height:none;overflow:visible}}
.sc-main{display:flex;flex-direction:column;gap:16px;min-width:0}

.sc-card{background:var(--sc-surface);border:1px solid var(--sc-border);border-radius:14px;padding:16px 18px;box-shadow:var(--sc-shadow)}
.sc-sidebar .sc-card h3{display:flex;align-items:center;gap:9px}
.sc-sidebar .sc-card h3::before{counter-increment:sc-step;content:counter(sc-step);flex:0 0 auto;width:20px;height:20px;border-radius:50%;background:var(--sc-accent);color:var(--sc-accent-fg);font-size:.68rem;display:flex;align-items:center;justify-content:center;font-weight:800}
.sc-hint{color:var(--sc-muted);font-size:.8rem;margin:8px 0 0}
.sc-row{display:flex;gap:12px;flex-wrap:wrap;margin-top:10px}
.sc-field{display:flex;flex-direction:column;gap:5px}
.sc-label{font-size:.72rem;color:var(--sc-muted);font-weight:600;text-transform:uppercase;letter-spacing:.03em}
.sc-input{width:110px;padding:7px 9px;border:1px solid var(--sc-border);border-radius:8px;background:var(--sc-bg);color:var(--sc-fg);font:inherit;transition:border-color .12s,box-shadow .12s}
select.sc-input{width:100%}
.sc-input:focus{outline:none;border-color:var(--sc-accent);box-shadow:0 0 0 3px var(--sc-accent-soft)}
.sc-actions{display:flex;gap:10px;flex-wrap:wrap;margin-top:14px}
.sc-btn{padding:8px 16px;border-radius:9px;border:1px solid var(--sc-border);background:var(--sc-surface);color:var(--sc-fg);font:inherit;font-weight:600;cursor:pointer;transition:filter .12s}
.sc-btn:hover{filter:brightness(.97)}
.sc-btn.sc-primary{background:var(--sc-accent);border-color:var(--sc-accent);color:var(--sc-accent-fg)}
.sc-btn.sc-primary:hover{filter:brightness(1.06)}

.sc-chartwrap{position:relative;border-radius:12px;overflow:hidden;border:1px solid var(--sc-border)}
.sc-chartwrap canvas{width:100%;height:auto;display:block;background-color:var(--sc-panel);background-image:linear-gradient(var(--sc-grid) 1px,transparent 1px),linear-gradient(90deg,var(--sc-grid) 1px,transparent 1px);background-size:24px 24px}
.sc-legend{display:flex;gap:14px;flex-wrap:wrap;margin-top:10px}
.sc-legend>div{display:inline-flex;align-items:center;gap:6px}
.sc-legend-label{font-size:.8rem;color:var(--sc-muted)}
.sc-swatch-solar::before{content:"";width:18px;height:12px;background:rgba(224,151,42,.45);border-bottom:2px solid #e0972a}
.sc-swatch-load::before{content:"";width:18px;height:0;border-top:3px solid #7a6f5f;border-radius:2px}
.sc-swatch-cash::before{content:"";width:18px;height:0;border-top:3px solid #2e9e6b;border-radius:2px}
.sc-swatch-loan::before{content:"";width:18px;height:0;border-top:3px dashed #5b7fa6;border-radius:2px}

.sc-results-view{margin:4px 0 0}
.sc-summary{margin:0 0 14px;padding:14px 16px;background:var(--sc-accent-soft);border-radius:10px}
.sc-headline{font-weight:700;font-size:1.05rem;margin:0 0 4px;color:var(--sc-fg)}
.sc-detail{color:var(--sc-muted);font-size:.83rem;margin:0}
.sc-stats{display:flex;gap:8px;flex-wrap:wrap;margin:0 0 14px}
.sc-stat{background:var(--sc-panel);border:1px solid var(--sc-border);border-radius:10px;padding:8px 12px;min-width:110px}
.sc-stat-value{display:block;font-size:1.02rem;font-variant-numeric:tabular-nums}
.sc-stat-label{font-size:.7rem;color:var(--sc-muted);text-transform:uppercase;letter-spacing:.03em;font-weight:600}
.sc-issues:empty{display:none}
.sc-issues{margin:0 0 12px;padding:10px 12px;background:var(--sc-err-bg);border:1px solid var(--sc-err);border-radius:10px}
.sc-issue{color:var(--sc-err);font-size:.88rem;margin:2px 0}
.sc-table{display:flex;flex-direction:column;font-size:.85rem;margin-top:4px}
.sc-thead,.sc-trow{display:grid;grid-template-columns:1.2fr repeat(4,1fr);gap:4px;padding:7px 8px;border-bottom:1px solid var(--sc-border)}
.sc-thead-wide,.sc-trow-wide{display:grid;grid-template-columns:1.6fr repeat(5,1fr);gap:4px;padding:7px 8px;border-bottom:1px solid var(--sc-border)}
.sc-thead,.sc-thead-wide{color:var(--sc-muted);font-size:.7rem;font-weight:700;text-transform:uppercase;letter-spacing:.03em;border-bottom:2px solid var(--sc-border)}
.sc-trow:hover,.sc-trow-wide:hover{background:var(--sc-panel)}
.sc-cell{text-align:left}
.sc-cellnum{text-align:right;font-variant-numeric:tabular-nums}
.sc-best{color:var(--sc-ok);font-weight:700}

.sc-upsell{border-radius:14px;padding:16px 18px;background:linear-gradient(135deg,var(--sc-accent-soft),transparent);border:1px dashed var(--sc-accent);color:var(--sc-fg);font-size:.88rem}
.sc-upsell h3{margin:0 0 6px}
.sc-upsell a{color:var(--sc-accent);font-weight:700;text-decoration:none}
.sc-upsell a:hover{text-decoration:underline}

.sc-footer{display:flex;flex-wrap:wrap;align-items:center;justify-content:space-between;gap:10px;margin-top:26px;padding-top:16px;border-top:1px solid var(--sc-border);color:var(--sc-muted);font-size:.82rem}
.sc-footer-brand{font-weight:700;color:var(--sc-fg)}
.sc-footer-links{display:flex;flex-wrap:wrap;gap:4px 16px}
.sc-footer-link{padding:0;border:none;background:none;color:var(--sc-accent);font:inherit;font-weight:600;cursor:pointer}
.sc-footer-link:hover{text-decoration:underline}
"#;

/// Built-in city presets: (label, lat, lon, UTC offset, altitude). The
/// custom entry leaves the fields untouched.
const CITIES: [(&str, &str, &str, &str, &str); 6] = [
    ("Wellington", "-41.2866", "174.7756", "12", "30"),
    ("Auckland", "-36.8485", "174.7633", "12", "20"),
    ("Hamilton", "-37.7870", "175.2793", "12", "40"),
    ("Christchurch", "-43.5321", "172.6362", "12", "37"),
    ("Dunedin", "-45.8788", "170.5028", "12", "10"),
    ("Custom (keep values)", "", "", "", ""),
];

/// Default field values — a 5 kWp north-facing array in Wellington quoted
/// at NZ$11,500, against a typical 7,000 kWh/yr household on a 30c/kWh
/// plan, so the first Calculate works with no edits.
mod defaults {
    pub const LAT: &str = "-41.2866";
    pub const LON: &str = "174.7756";
    pub const TZ: &str = "12";
    pub const ALT: &str = "30";
    pub const SIZE: &str = "5";
    pub const TILT: &str = "30";
    pub const AZIMUTH: &str = "0";
    pub const QUOTE: &str = "11500";
    pub const ANNUAL_KWH: &str = "7000";
    pub const IMPORT: &str = "30";
    pub const FEED_IN: &str = "8";
    pub const ESCALATION: &str = "3";
    pub const DISCOUNT: &str = "7";
    #[cfg(feature = "pro")]
    pub const SCENARIO_SIZES: [&str; 4] = ["3", "5", "6.5", "8"];
    #[cfg(feature = "pro")]
    pub const SCENARIO_QUOTES: [&str; 4] = ["7500", "11500", "14500", "17500"];
    #[cfg(feature = "pro")]
    pub const LOAN_DEPOSIT: &str = "0";
    #[cfg(feature = "pro")]
    pub const LOAN_RATE: &str = "10.9";
    #[cfg(feature = "pro")]
    pub const LOAN_TERM: &str = "7";
}

/// Everything the engine needs, read from the form.
struct FormInputs {
    site: Site,
    system: SolarSystem,
    tariff: Tariff,
    household: Household,
    discount_rate: f64,
}

/// Mounts the whole app into `container` (the hub's container div, or the
/// standalone `#tpt-appfront-root` marker).
pub fn mount_app(container: &web_sys::Element) -> Result<(), JsValue> {
    let document = web_sys::window()
        .expect("no window")
        .document()
        .expect("no document");

    upsert_style(&document, "solar-checker-styles", BASE_CSS);
    // `.sc-app` on the mount root itself: every color/font custom property
    // lives here, so the whole theme depends on this class being present on
    // an ancestor of the tree the CSS above targets.
    let existing_class = container.get_attribute("class").unwrap_or_default();
    if !existing_class.split_whitespace().any(|c| c == "sc-app") {
        let combined = if existing_class.is_empty() {
            "sc-app".to_string()
        } else {
            format!("{existing_class} sc-app")
        };
        let _ = container.set_attribute("class", &combined);
    }

    let estimate_outcome: Signal<EstimateOutcome> = Signal::new(EstimateOutcome::Idle);
    #[cfg(feature = "pro")]
    let scenarios: Signal<ScenarioListOutcome> = Signal::new(ScenarioListOutcome::Idle);
    #[cfg(feature = "pro")]
    let loan: Signal<LoanOutcome> = Signal::new(LoanOutcome::Idle);

    let dispatch: Rc<dyn Fn(Msg)> = {
        let estimate_outcome = estimate_outcome.clone();
        #[cfg(feature = "pro")]
        let scenarios = scenarios.clone();
        #[cfg(feature = "pro")]
        let loan = loan.clone();
        let container = container.clone();
        Rc::new(move |msg| match msg {
            Msg::Calculate => run_calculate(&container, &estimate_outcome),
            Msg::CityPreset(index) => apply_city_preset(&container, index),
            Msg::VisitSite(url) => visit_site(&url),
            #[cfg(feature = "pro")]
            Msg::CompareScenarios => run_compare(&container, &scenarios),
            #[cfg(feature = "pro")]
            Msg::RunFinancing => run_financing(&container, &estimate_outcome, &loan),
            #[cfg(feature = "pro")]
            Msg::ExportPdf => export_pdf(&container, &estimate_outcome, &scenarios, &loan),
        })
    };

    // 1. Static shell — mounted exactly once.
    let shell = shell_tree();
    let root = tpt_appfront_dom::mount(container, &shell, dispatch.clone())?;
    std::mem::forget(root);

    // 2. Reactive results section into its placeholder.
    let results_el = container
        .query_selector(".sc-results")?
        .ok_or_else(|| JsValue::from_str("results placeholder missing"))?;
    let estimate_for_render = estimate_outcome.clone();
    let handle = tpt_appfront_dom::render(
        &results_el,
        Rc::new(move || results_view(estimate_for_render.get())),
        dispatch.clone(),
    )?;
    std::mem::forget(handle);

    // 3. Monthly generation chart (raw canvas; no UITree node kind).
    let chart_holder = container
        .query_selector(".sc-chart")?
        .ok_or_else(|| JsValue::from_str("chart placeholder missing"))?;
    let canvas: web_sys::HtmlCanvasElement = document
        .create_element("canvas")?
        .dyn_into()
        .map_err(|_| JsValue::from_str("canvas cast failed"))?;
    canvas.set_width(960);
    canvas.set_height(380);
    chart_holder.append_child(&canvas)?;

    // 4. Redraw effect: outcome changes repaint the chart.
    {
        let canvas = canvas.clone();
        let container = container.clone();
        let estimate_for_draw = estimate_outcome.clone();
        let handle = create_effect(move || {
            if let EstimateOutcome::Solved(report) = estimate_for_draw.get() {
                let fg = read_css(&container, "--sc-fg", "#241c12");
                let muted = read_css(&container, "--sc-muted", "#6f6555");
                let border = read_css(&container, "--sc-border", "#e7dfd0");
                let _ = chart::draw(&canvas, &monthly_chart(&report), &fg, &muted, &border);
            }
        });
        std::mem::forget(handle);
    }

    // 5. (pro) Scenario section + cashflow chart + financing section.
    #[cfg(feature = "pro")]
    {
        let scenario_el = container
            .query_selector(".sc-scenario-results")?
            .ok_or_else(|| JsValue::from_str("scenario placeholder missing"))?;
        let scenarios_for_render = scenarios.clone();
        let handle = tpt_appfront_dom::render(
            &scenario_el,
            Rc::new(move || scenario_results_view(scenarios_for_render.get())),
            dispatch.clone(),
        )?;
        std::mem::forget(handle);

        let cashflow_holder = container
            .query_selector(".sc-cashflow-chart")?
            .ok_or_else(|| JsValue::from_str("cashflow chart placeholder missing"))?;
        let cashflow_canvas: web_sys::HtmlCanvasElement = document
            .create_element("canvas")?
            .dyn_into()
            .map_err(|_| JsValue::from_str("canvas cast failed"))?;
        cashflow_canvas.set_width(960);
        cashflow_canvas.set_height(380);
        cashflow_holder.append_child(&cashflow_canvas)?;

        let loan_el = container
            .query_selector(".sc-loan-results")?
            .ok_or_else(|| JsValue::from_str("loan placeholder missing"))?;
        let loan_for_render = loan.clone();
        let handle = tpt_appfront_dom::render(
            &loan_el,
            Rc::new(move || loan_results_view(loan_for_render.get())),
            dispatch.clone(),
        )?;
        std::mem::forget(handle);

        // The cumulative-cash chart needs both the cash-purchase stream and
        // the financed one, so the effect watches both signals and redraws
        // when either changes.
        let container_for_draw = container.clone();
        let estimate_for_draw = estimate_outcome.clone();
        let loan_for_draw = loan.clone();
        let handle = create_effect(move || {
            if let (EstimateOutcome::Solved(report), LoanOutcome::Solved(_, loan)) =
                (estimate_for_draw.get(), loan_for_draw.get())
            {
                let fg = read_css(&container_for_draw, "--sc-fg", "#241c12");
                let muted = read_css(&container_for_draw, "--sc-muted", "#6f6555");
                let border = read_css(&container_for_draw, "--sc-border", "#e7dfd0");
                let _ = chart::draw(
                    &cashflow_canvas,
                    &cashflow_chart(&report, &loan),
                    &fg,
                    &muted,
                    &border,
                );
            }
        });
        std::mem::forget(handle);
    }

    Ok(())
}

/// The mounted-once shell: header, input cards, chart card, results
/// placeholders, upsell.
fn shell_tree() -> UITree<Msg> {
    UITree::container(|c| {
        c.container(|header| {
            header.text("SC").class("sc-brand");
            header.container(|group| {
                group.container(|row| {
                    row.heading(2, "Solar Checker");
                    #[cfg(feature = "pro")]
                    row.text("Pro").class("sc-chip");
                })
                .class("sc-title-row");
                group.text(
                    "Independent solar payback calculator — a second opinion on any quote. \
                     TPT Solutions doesn't sell or install solar, so the numbers have no \
                     axe to grind. All figures in NZ$.",
                )
                .class("sc-sub");
            })
            .class("sc-title-group");
        })
        .class("sc-header");

        c.container(|shell| {
            shell.container(|sidebar| {
                sidebar.container(|card| {
                    card.heading(3, "Location");
                    card.container(|row| {
                        row.container(|field| {
                            label(field, "City preset");
                            field.select(
                                CITIES
                                    .iter()
                                    .enumerate()
                                    .map(|(i, (name, ..))| (i.to_string(), name.to_string()))
                                    .collect::<Vec<_>>(),
                                "0",
                            )
                            .class("sc-input sc-city")
                            .on_input(|value| {
                                Msg::CityPreset(value.parse::<usize>().unwrap_or(0))
                            });
                        })
                        .class("sc-field");
                    })
                    .class("sc-row");
                    card.container(|row| {
                        row.container(|field| {
                            label(field, "Latitude (\u{b0})");
                            field.input(defaults::LAT).class("sc-input sc-lat");
                        })
                        .class("sc-field");
                        row.container(|field| {
                            label(field, "Longitude (\u{b0})");
                            field.input(defaults::LON).class("sc-input sc-lon");
                        })
                        .class("sc-field");
                    })
                    .class("sc-row");
                    card.container(|row| {
                        row.container(|field| {
                            label(field, "UTC offset (h)");
                            field.input(defaults::TZ).class("sc-input sc-tz");
                        })
                        .class("sc-field");
                        row.container(|field| {
                            label(field, "Altitude (m)");
                            field.input(defaults::ALT).class("sc-input sc-alt");
                        })
                        .class("sc-field");
                    })
                    .class("sc-row");
                })
                .class("sc-card");

                sidebar.container(|card| {
                    card.heading(3, "Proposed system");
                    card.container(|row| {
                        row.container(|field| {
                            label(field, "Size (kWp)");
                            field.input(defaults::SIZE).class("sc-input sc-size");
                        })
                        .class("sc-field");
                        row.container(|field| {
                            label(field, "Tilt (\u{b0})");
                            field.input(defaults::TILT).class("sc-input sc-tilt");
                        })
                        .class("sc-field");
                        row.container(|field| {
                            label(field, "Azimuth (\u{b0})");
                            field.input(defaults::AZIMUTH).class("sc-input sc-azimuth");
                        })
                        .class("sc-field");
                    })
                    .class("sc-row");
                    card.container(|row| {
                        row.container(|field| {
                            label(field, "Installed quote (NZ$)");
                            field.input(defaults::QUOTE).class("sc-input sc-quote");
                        })
                        .class("sc-field");
                    })
                    .class("sc-row");
                    card.text(
                        "Azimuth from north: 0 = north-facing (best in New Zealand), \
                         90 = east, 180 = south, 270 = west. The quote is the total \
                         installed price before any rebate.",
                    )
                    .class("sc-hint");
                })
                .class("sc-card");

                sidebar.container(|card| {
                    card.heading(3, "Electricity & tariff");
                    card.container(|row| {
                        row.container(|field| {
                            label(field, "Consumption (kWh/yr)");
                            field.input(defaults::ANNUAL_KWH).class("sc-input sc-annual");
                        })
                        .class("sc-field");
                        row.container(|field| {
                            label(field, "Import rate (c/kWh)");
                            field.input(defaults::IMPORT).class("sc-input sc-import");
                        })
                        .class("sc-field");
                    })
                    .class("sc-row");
                    card.container(|row| {
                        row.container(|field| {
                            label(field, "Feed-in rate (c/kWh)");
                            field.input(defaults::FEED_IN).class("sc-input sc-feedin");
                        })
                        .class("sc-field");
                        row.container(|field| {
                            label(field, "Price rise (%/yr)");
                            field.input(defaults::ESCALATION).class("sc-input sc-escalation");
                        })
                        .class("sc-field");
                    })
                    .class("sc-row");
                    card.container(|row| {
                        row.container(|field| {
                            label(field, "Discount rate (%)");
                            field.input(defaults::DISCOUNT).class("sc-input sc-discount");
                        })
                        .class("sc-field");
                    })
                    .class("sc-row");
                    card.text(
                        "On a typical day the load peaks morning and evening, so much of \
                         the midday solar is exported rather than self-consumed — the \
                         split matters because the feed-in rate is far below the import \
                         rate.",
                    )
                    .class("sc-hint");
                })
                .class("sc-card");

                sidebar.container(|card| {
                    card.heading(3, "Estimate");
                    card.container(|actions| {
                        actions
                            .button("Calculate payback")
                            .class("sc-btn sc-primary")
                            .on_click(Msg::Calculate);
                    })
                    .class("sc-actions");
                })
                .class("sc-card");

                #[cfg(feature = "pro")]
                sidebar.container(|card| {
                    card.heading(3, "Scenario comparison (Pro)");
                    card.text("Compare quotes side by side: different sizes, different \
                               installers, same roof and tariff.")
                        .class("sc-hint");
                    for (i, size) in defaults::SCENARIO_SIZES.iter().enumerate() {
                        card.container(|row| {
                            row.container(|field| {
                                label(field, "Name");
                                field
                                    .input(&scenario_default_name(i))
                                    .class(format!("sc-input sc-scenario-name-{i}"));
                            })
                            .class("sc-field");
                            row.container(|field| {
                                label(field, "Size (kWp)");
                                field
                                    .input(*size)
                                    .class(format!("sc-input sc-scenario-size-{i}"));
                            })
                            .class("sc-field");
                            row.container(|field| {
                                label(field, "Quote (NZ$)");
                                field
                                    .input(defaults::SCENARIO_QUOTES[i])
                                    .class(format!("sc-input sc-scenario-quote-{i}"));
                            })
                            .class("sc-field");
                        })
                        .class("sc-row");
                    }
                    card.container(|actions| {
                        actions
                            .button("Compare scenarios")
                            .class("sc-btn sc-primary")
                            .on_click(Msg::CompareScenarios);
                    })
                    .class("sc-actions");
                })
                .class("sc-card");

                #[cfg(feature = "pro")]
                sidebar.container(|card| {
                    card.heading(3, "Financing (Pro)");
                    card.text("Loan vs cash: what the payments do to the break-even.")
                        .class("sc-hint");
                    card.container(|row| {
                        row.container(|field| {
                            label(field, "Deposit (NZ$)");
                            field
                                .input(defaults::LOAN_DEPOSIT)
                                .class("sc-input sc-loan-deposit");
                        })
                        .class("sc-field");
                        row.container(|field| {
                            label(field, "Interest (%/yr)");
                            field
                                .input(defaults::LOAN_RATE)
                                .class("sc-input sc-loan-rate");
                        })
                        .class("sc-field");
                        row.container(|field| {
                            label(field, "Term (years)");
                            field
                                .input(defaults::LOAN_TERM)
                                .class("sc-input sc-loan-term");
                        })
                        .class("sc-field");
                    })
                    .class("sc-row");
                    card.container(|actions| {
                        actions
                            .button("Calculate financing")
                            .class("sc-btn sc-primary")
                            .on_click(Msg::RunFinancing);
                    })
                    .class("sc-actions");
                })
                .class("sc-card");
            })
            .class("sc-sidebar");

            shell.container(|main| {
                main.container(|card| {
                    card.heading(3, "Generation vs consumption");
                    card.text("Monthly solar production against the household's monthly \
                               consumption — the winter gap is why payback is rarely as \
                               fast as the brochure says.")
                        .class("sc-hint");
                    // The canvas is appended into .sc-chart by mount_app.
                    card.container(|holder| {
                        holder.container(|_| {}).class("sc-chart");
                        holder.container(|legend| {
                            legend.container(|item| {
                                styled_text(item, "Solar generated", "sc-legend-label");
                            })
                            .class("sc-swatch-solar");
                            legend.container(|item| {
                                styled_text(item, "Household consumption", "sc-legend-label");
                            })
                            .class("sc-swatch-load");
                        })
                        .class("sc-legend");
                    })
                    .class("sc-chartwrap");
                })
                .class("sc-card");

                main.container(|card| {
                    card.heading(3, "Results");
                    card.text(
                        "Clear-sky model with typical New Zealand cloud and temperature by \
                         month, 2% soiling, 14% balance-of-system losses, and 0.5%/yr \
                         degradation. Actual output varies with weather, shading, and \
                         installation quality.",
                    )
                    .class("sc-hint");
                    // The reactive results section mounts into this placeholder.
                    card.container(|_| {}).class("sc-results");
                })
                .class("sc-card");

                #[cfg(feature = "pro")]
                main.container(|card| {
                    card.heading(3, "Scenario comparison (Pro)");
                    // The reactive scenario table mounts into this placeholder.
                    card.container(|_| {}).class("sc-scenario-results");
                })
                .class("sc-card");

                #[cfg(feature = "pro")]
                main.container(|card| {
                    card.heading(3, "Financing (Pro)");
                    card.text("Cumulative position with the money: cash purchase versus \
                               financed, over 25 years.")
                        .class("sc-hint");
                    // The cashflow canvas is appended here by mount_app.
                    card.container(|holder| {
                        holder.container(|_| {}).class("sc-cashflow-chart");
                        holder.container(|legend| {
                            legend.container(|item| {
                                styled_text(item, "Cash purchase", "sc-legend-label");
                            })
                            .class("sc-swatch-cash");
                            legend.container(|item| {
                                styled_text(item, "Financed", "sc-legend-label");
                            })
                            .class("sc-swatch-loan");
                        })
                        .class("sc-legend");
                    })
                    .class("sc-chartwrap");
                    // The reactive financing numbers mount into this placeholder.
                    card.container(|_| {}).class("sc-loan-results");
                })
                .class("sc-card");

                #[cfg(feature = "pro")]
                main.container(|card| {
                    card.heading(3, "Export (Pro)");
                    card.text(
                        "A one-page PDF with the assumptions next to the numbers — bring \
                         it to the quote negotiation.",
                    )
                    .class("sc-hint");
                    card.container(|actions| {
                        actions
                            .button("Export PDF report")
                            .class("sc-btn sc-primary")
                            .on_click(Msg::ExportPdf);
                    })
                    .class("sc-actions");
                })
                .class("sc-card");

                #[cfg(not(feature = "pro"))]
                main.container(|upsell| {
                    upsell.heading(3, "Comparing quotes, or financing it?");
                    upsell.text(
                        "The Pro edition ($9, desktop) compares different system sizes and \
                         quotes side by side, works out loan-versus-cash payback with a \
                         financing calculator, and exports a PDF report you can bring to \
                         a quote negotiation.",
                    );
                    upsell.link(
                        "https://tptsolutions.co.nz/tools/solar-checker",
                        "Get the Pro edition",
                    );
                })
                .class("sc-upsell");
            })
            .class("sc-main");
        })
        .class("sc-shell");

        // Present in both editions — the Pro desktop shell has no browser
        // chrome and no other route back to tptsolutions.co.nz once
        // installed, so without this the app is a dead end for discovering
        // the rest of the product line. See `visit_site`/`footer_link` for
        // why this can't just be a plain `<a href>`.
        c.container(|footer| {
            styled_text(footer, "TPT Solutions", "sc-footer-brand");
            footer.container(|links| {
                footer_link(links, "More tools", "https://tptsolutions.co.nz/tools");
                footer_link(links, "tptsolutions.co.nz", "https://tptsolutions.co.nz");
            })
            .class("sc-footer-links");
        })
        .class("sc-footer");
    })
}

/// `N kW` display name for scenario row `i`.
#[cfg(feature = "pro")]
fn scenario_default_name(i: usize) -> String {
    format!("Option {}", i + 1)
}

/// `.text()` produces a raw DOM text node, which can't carry a `class`
/// attribute (the reconciler no-ops classes for text nodes) — wrap the text
/// in a container so the styling actually reaches the page.
fn styled_text(
    c: &mut tpt_appfront_core::ContainerBuilder<Msg>,
    text: impl Into<String>,
    class: &str,
) {
    let text = text.into();
    c.container(move |c| {
        c.text(text);
    })
    .class(class);
}

/// Field label (see [`styled_text`]).
fn label(field: &mut tpt_appfront_core::ContainerBuilder<Msg>, text: impl Into<String>) {
    styled_text(field, text, "sc-label");
}

/// A footer "link" rendered as a plain `<button>` (not `<a>`) dispatching
/// [`Msg::VisitSite`]. Deliberately not `ContainerBuilder::link` — that
/// builder emits a real `href` alongside `on_click`, and the DOM
/// reconciler's click handler never calls `preventDefault`, so an anchor
/// with both would navigate the local `app://` webview *and* fire the
/// message. A button has no navigation behavior to suppress.
fn footer_link(c: &mut tpt_appfront_core::ContainerBuilder<Msg>, label_text: &str, url: &str) {
    c.button(label_text)
        .class("sc-footer-link")
        .on_click(Msg::VisitSite(url.to_string()));
}

/// Opens a TPT Solutions URL from wherever the app happens to be running.
///
/// The free edition's wasm runs in a browser tab, where a normal new-tab
/// open is correct. The Pro edition's *identical* wasm bundle is instead
/// served over the desktop shell's `app://localhost` custom protocol —
/// there, `window.open`/navigation would replace the app inside the native
/// window with no way back, so the open is routed through the desktop
/// host's allowlisted IPC bridge. Runtime protocol sniffing (rather than
/// `#[cfg(feature = "pro")]`) is what distinguishes the two, since it's the
/// hosting context that matters, not the build.
fn visit_site(url: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let in_desktop_shell = window
        .location()
        .protocol()
        .map(|p| p == "app:")
        .unwrap_or(false);

    if in_desktop_shell {
        if let Ok(appfront) = js_sys::Reflect::get(&window, &JsValue::from_str("__appfront")) {
            if let Ok(post) = js_sys::Reflect::get(&appfront, &JsValue::from_str("post")) {
                if let Ok(post_fn) = post.dyn_into::<js_sys::Function>() {
                    let params = js_sys::Object::new();
                    let _ = js_sys::Reflect::set(
                        &params,
                        &JsValue::from_str("url"),
                        &JsValue::from_str(url),
                    );
                    let _ = post_fn.call2(&appfront, &JsValue::from_str("open_external"), &params);
                }
            }
        }
    } else {
        let _ = window.open_with_url_and_target(url, "_blank");
    }
}

/// Downloads `bytes` as a file (used by the Pro PDF export).
#[cfg(feature = "pro")]
fn download_bytes(filename: &str, bytes: &[u8]) -> Result<(), JsValue> {
    let document = web_sys::window()
        .ok_or_else(|| JsValue::from_str("no window"))?
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;
    let array = js_sys::Array::new();
    array.push(&js_sys::Uint8Array::from(bytes));
    let blob = web_sys::Blob::new_with_u8_array_sequence(&array)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;
    let anchor: web_sys::HtmlAnchorElement = document
        .create_element("a")?
        .dyn_into()
        .map_err(|_| JsValue::from_str("anchor cast failed"))?;
    anchor.set_href(&url);
    anchor.set_download(filename);
    if let Some(body) = document.body() {
        let _ = body.append_child(&anchor);
    }
    anchor.click();
    let _ = anchor.remove();
    let _ = web_sys::Url::revoke_object_url(&url);
    Ok(())
}

// ---- Form reading ----

/// Parses one numeric input, recording a display-ready error on failure.
fn num(container: &web_sys::Element, selector: &str, name: &str, issues: &mut Vec<String>) -> f64 {
    let text = read_input(container, selector);
    match text.trim().replace(",", "").parse::<f64>() {
        Ok(value) if value.is_finite() => value,
        _ => {
            issues.push(format!("{name} must be a number (got \u{201c}{text}\u{201d})."));
            f64::NAN
        }
    }
}

/// Reads and validates the whole form; `Err` carries display-ready issues.
fn read_form(container: &web_sys::Element) -> Result<FormInputs, Vec<String>> {
    let mut issues = Vec::new();

    let site = Site {
        latitude_deg: num(container, ".sc-lat", "Latitude", &mut issues),
        longitude_deg: num(container, ".sc-lon", "Longitude", &mut issues),
        timezone_offset_hours: num(container, ".sc-tz", "UTC offset", &mut issues),
        altitude_m: num(container, ".sc-alt", "Altitude", &mut issues),
    };
    let system = SolarSystem {
        capacity_kw: num(container, ".sc-size", "System size", &mut issues),
        tilt_deg: num(container, ".sc-tilt", "Tilt", &mut issues),
        azimuth_deg: num(container, ".sc-azimuth", "Azimuth", &mut issues),
        installed_cost_nzd: num(container, ".sc-quote", "Installed quote", &mut issues),
    };
    let tariff = Tariff {
        import_nzd_per_kwh: num(container, ".sc-import", "Import rate", &mut issues) / 100.0,
        feed_in_nzd_per_kwh: num(container, ".sc-feedin", "Feed-in rate", &mut issues) / 100.0,
        annual_escalation: num(container, ".sc-escalation", "Price rise", &mut issues) / 100.0,
    };
    let household = Household {
        annual_kwh: num(container, ".sc-annual", "Consumption", &mut issues),
    };
    let discount_rate = num(container, ".sc-discount", "Discount rate", &mut issues) / 100.0;

    if !issues.is_empty() {
        return Err(issues);
    }
    issues.extend(site.validate());
    issues.extend(system.validate());
    issues.extend(tariff.validate());
    issues.extend(household.validate());
    if !(0.0..=0.25).contains(&discount_rate) {
        issues.push("Discount rate must be between 0% and 25%.".to_string());
    }
    if issues.is_empty() {
        Ok(FormInputs {
            site,
            system,
            tariff,
            household,
            discount_rate,
        })
    } else {
        Err(issues)
    }
}

/// Reads the financing inputs (Pro).
#[cfg(feature = "pro")]
fn read_loan(container: &web_sys::Element) -> Result<LoanTerms, Vec<String>> {
    let mut issues = Vec::new();
    let deposit = num(container, ".sc-loan-deposit", "Deposit", &mut issues);
    let rate = num(container, ".sc-loan-rate", "Interest rate", &mut issues) / 100.0;
    let term = num(container, ".sc-loan-term", "Loan term", &mut issues);
    if !issues.is_empty() {
        return Err(issues);
    }
    let terms = LoanTerms {
        deposit_nzd: deposit,
        annual_interest_rate: rate,
        term_years: term.round().clamp(1.0, 30.0) as u32,
    };
    issues.extend(terms.validate());
    let quote = read_input(container, ".sc-quote")
        .trim()
        .replace(",", "")
        .parse::<f64>()
        .unwrap_or(0.0);
    if deposit > quote && quote > 0.0 {
        issues.push("The deposit is larger than the installed quote.".to_string());
    }
    if issues.is_empty() {
        Ok(terms)
    } else {
        Err(issues)
    }
}

/// Reads the non-empty scenarios from the comparison rows (Pro). Rows with
/// neither a size nor a quote are treated as intentionally blank.
#[cfg(feature = "pro")]
fn read_scenarios(container: &web_sys::Element) -> Result<Vec<Scenario>, Vec<String>> {
    let mut issues = Vec::new();
    let mut scenarios = Vec::new();
    for i in 0..defaults::SCENARIO_SIZES.len() {
        let name = read_input(container, &format!(".sc-scenario-name-{i}"));
        let size_text = read_input(container, &format!(".sc-scenario-size-{i}"));
        let quote_text = read_input(container, &format!(".sc-scenario-quote-{i}"));
        if size_text.trim().is_empty() && quote_text.trim().is_empty() {
            continue;
        }
        let size = size_text.trim().parse::<f64>();
        let quote = quote_text.trim().replace(",", "").parse::<f64>();
        match (size, quote) {
            (Ok(size), Ok(quote)) if size > 0.0 && quote > 0.0 => {
                let name = if name.trim().is_empty() {
                    scenario_default_name(i)
                } else {
                    name
                };
                scenarios.push(Scenario {
                    name,
                    capacity_kw: size,
                    installed_cost_nzd: quote,
                });
            }
            _ => issues.push(format!(
                "Scenario row {} needs a size and a quote that are both positive.",
                i + 1
            )),
        }
    }
    if issues.is_empty() && scenarios.len() < 2 {
        issues.push("Fill in at least two scenarios to compare.".to_string());
    }
    if issues.is_empty() {
        Ok(scenarios)
    } else {
        Err(issues)
    }
}

// ---- Dispatch handlers ----

/// Runs the single-property estimate and publishes the outcome.
fn run_calculate(container: &web_sys::Element, outcome: &Signal<EstimateOutcome>) {
    let form = match read_form(container) {
        Ok(form) => form,
        Err(issues) => {
            outcome.set(EstimateOutcome::Invalid(issues));
            return;
        }
    };
    outcome.set(EstimateOutcome::Solved(Rc::new(estimate(
        &form.site,
        &form.system,
        &form.tariff,
        &form.household,
        form.discount_rate,
    ))));
}

/// Fills the location inputs from a city preset.
fn apply_city_preset(container: &web_sys::Element, index: usize) {
    let index = index % CITIES.len();
    let (_, lat, lon, tz, alt) = CITIES[index];
    if lat.is_empty() {
        return; // "custom" leaves the fields alone
    }
    write_field(container, ".sc-lat", lat);
    write_field(container, ".sc-lon", lon);
    write_field(container, ".sc-tz", tz);
    write_field(container, ".sc-alt", alt);
}

/// Runs the scenario comparison and publishes the outcome (Pro).
#[cfg(feature = "pro")]
fn run_compare(container: &web_sys::Element, outcome: &Signal<ScenarioListOutcome>) {
    let (site, tariff, household, discount_rate) = match read_form(container) {
        Ok(form) => (form.site, form.tariff, form.household, form.discount_rate),
        Err(issues) => {
            outcome.set(ScenarioListOutcome::Invalid(issues));
            return;
        }
    };
    let scenarios = match read_scenarios(container) {
        Ok(scenarios) => scenarios,
        Err(issues) => {
            outcome.set(ScenarioListOutcome::Invalid(issues));
            return;
        }
    };
    outcome.set(ScenarioListOutcome::Solved(Rc::new(compare_scenarios(
        &site,
        &tariff,
        &household,
        discount_rate,
        &scenarios,
    ))));
}

/// Runs the financing analysis against a fresh estimate and publishes the
/// outcome (Pro).
#[cfg(feature = "pro")]
fn run_financing(
    container: &web_sys::Element,
    estimate_outcome: &Signal<EstimateOutcome>,
    loan_outcome: &Signal<LoanOutcome>,
) {
    let form = match read_form(container) {
        Ok(form) => form,
        Err(issues) => {
            loan_outcome.set(LoanOutcome::Invalid(issues));
            return;
        }
    };
    let terms = match read_loan(container) {
        Ok(terms) => terms,
        Err(issues) => {
            loan_outcome.set(LoanOutcome::Invalid(issues));
            return;
        }
    };
    let report = estimate(
        &form.site,
        &form.system,
        &form.tariff,
        &form.household,
        form.discount_rate,
    );
    // Keep the main results in step with the financing run.
    estimate_outcome.set(EstimateOutcome::Solved(Rc::new(report.clone())));
    let result = analyze_financing(&report, form.system.installed_cost_nzd, &terms);
    loan_outcome.set(LoanOutcome::Solved(Rc::new(terms), Rc::new(result)));
}

/// Builds and downloads the PDF report from the current state (Pro). A
/// no-op until an estimate has been run — the report documents it.
#[cfg(feature = "pro")]
fn export_pdf(
    container: &web_sys::Element,
    estimate_outcome: &Signal<EstimateOutcome>,
    scenarios: &Signal<ScenarioListOutcome>,
    loan: &Signal<LoanOutcome>,
) {
    let EstimateOutcome::Solved(report) = estimate_outcome.get() else {
        return;
    };
    let form = match read_form(container) {
        Ok(form) => form,
        Err(_) => return,
    };
    let scenario_rows = match scenarios.get() {
        ScenarioListOutcome::Solved(rows) => Some(rows),
        _ => None,
    };
    let financing = match loan.get() {
        LoanOutcome::Solved(terms, result) => Some(((*terms).clone(), (*result).clone())),
        _ => None,
    };
    let rows = pdf::render_report(
        &form.site,
        &form.system,
        &form.tariff,
        &form.household,
        &report,
        form.discount_rate,
        scenario_rows.as_deref().map(Vec::as_slice),
        financing.as_ref().map(|(terms, result)| (terms, result)),
    );
    let _ = download_bytes("tpt-solar-report.pdf", &pdf::render_pdf("TPT Solar Checker", &rows));
}

// ---- Result views ----

/// Groups a dollar amount with thousands separators (`12500 -> "$12,500"`).
fn dollars(amount: f64) -> String {
    let rounded = amount.round() as i64;
    let negative = rounded < 0;
    let digits = rounded.unsigned_abs().to_string();
    let mut grouped = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if negative {
        format!("-${grouped}")
    } else {
        format!("${grouped}")
    }
}

/// Payback phrased for the headline (`None` = never within the horizon).
fn payback_text(payback: Option<f64>) -> String {
    match payback {
        Some(years) => format!("{years:.1} years"),
        None => "over 25 yrs".to_string(),
    }
}

/// The reactive results section (both editions).
fn results_view(outcome: EstimateOutcome) -> UITree<Msg> {
    UITree::container(|c| match outcome {
        EstimateOutcome::Idle => {
            styled_text(
                c,
                "Press \u{201c}Calculate payback\u{201d} to run the estimate.",
                "sc-detail",
            );
        }
        EstimateOutcome::Invalid(issues) => {
            c.container(|list| {
                for issue in issues {
                    styled_text(list, issue, "sc-issue");
                }
            })
            .class("sc-issues");
        }
        EstimateOutcome::Solved(report) => {
            let balance = &report.balance;
            let money = &report.financials;
            let never = money.payback_years.is_none();

            c.container(|summary| {
                styled_text(
                    summary,
                    if never {
                        "This system doesn't pay for itself within 25 years.".to_string()
                    } else {
                        format!(
                            "Payback in {:.1} years, then {:.0} years of essentially free power.",
                            money.payback_years.expect("checked not-None"),
                            25.0 - money.payback_years.expect("checked not-None")
                        )
                    },
                    "sc-headline",
                );
                styled_text(
                    summary,
                    format!(
                        "A {:.1} kWp system generates {:.0} kWh/yr here — {:.0}% is used \
                         on-site at the full import rate, {:.0} kWh is exported at the \
                         (lower) feed-in rate, and the remaining {:.0} kWh/yr still comes \
                         from the grid. Year-1 bill savings: {}.",
                        balance.system_size_kw,
                        balance.annual_generation_kwh,
                        balance.self_consumption_fraction() * 100.0,
                        balance.exported_kwh,
                        balance.grid_import_with_solar_kwh,
                        dollars(money.year1_savings_nzd),
                    ),
                    "sc-detail",
                );
            })
            .class("sc-summary");

            c.container(|stats| {
                stat(stats, &format!("{:.0}", balance.annual_generation_kwh), "Generation kWh/yr");
                stat(
                    stats,
                    &format!("{:.0}%", balance.self_consumption_fraction() * 100.0),
                    "Self-consumed",
                );
                stat(stats, &format!("{:.0}", balance.exported_kwh), "Exported kWh/yr");
                stat(stats, &dollars(money.year1_savings_nzd), "Year-1 savings");
                stat(stats, &payback_text(money.payback_years), "Payback");
                stat(
                    stats,
                    &dollars(money.lifetime_net_savings_nzd),
                    "25-yr net savings",
                );
                stat(
                    stats,
                    &match money.irr {
                        Some(irr) => format!("{:.1}%", irr * 100.0),
                        None => "\u{2014}".to_string(),
                    },
                    "IRR",
                );
                stat(stats, &dollars(money.npv_nzd), "NPV @ discount");
            })
            .class("sc-stats");

            c.container(|table| {
                table.container(|row| {
                    styled_text(row, "Month", "sc-cell");
                    styled_text(row, "Generated kWh", "sc-cellnum");
                    styled_text(row, "Consumed kWh", "sc-cellnum");
                    styled_text(row, "Self-used kWh", "sc-cellnum");
                    styled_text(row, "Exported kWh", "sc-cellnum");
                })
                .class("sc-thead");
                for m in 0..MONTHS_PER_YEAR {
                    table.container(|row| {
                        styled_text(row, MONTH_NAMES[m], "sc-cell");
                        styled_text(
                            row,
                            format!("{:.0}", balance.monthly_generation_kwh[m]),
                            "sc-cellnum",
                        );
                        styled_text(
                            row,
                            format!("{:.0}", balance.monthly_consumption_kwh[m]),
                            "sc-cellnum",
                        );
                        styled_text(
                            row,
                            format!("{:.0}", balance.monthly_self_consumed_kwh[m]),
                            "sc-cellnum",
                        );
                        styled_text(
                            row,
                            format!("{:.0}", balance.monthly_exported_kwh[m]),
                            "sc-cellnum",
                        );
                    })
                    .class("sc-trow");
                }
            })
            .class("sc-table");
        }
    })
}

/// One summary stat chip.
fn stat(c: &mut tpt_appfront_core::ContainerBuilder<Msg>, value: &str, label_text: &str) {
    c.container(|chip| {
        styled_text(chip, value, "sc-stat-value");
        styled_text(chip, label_text, "sc-stat-label");
    })
    .class("sc-stat");
}

/// Chart data: monthly generation (filled) against consumption (line).
fn monthly_chart(report: &PaybackReport) -> chart::Chart {
    let balance = &report.balance;
    chart::Chart {
        series: vec![
            chart::ChartSeries {
                color: "#e0972a",
                fill_color: Some("rgba(224,151,42,0.30)"),
                values: balance.monthly_generation_kwh.to_vec(),
                dashed: false,
            },
            chart::ChartSeries {
                color: "#7a6f5f",
                fill_color: None,
                values: balance.monthly_consumption_kwh.to_vec(),
                dashed: false,
            },
        ],
        x_labels: MONTH_NAMES.iter().map(|n| n[..3].to_string()).collect(),
        left_title: "kWh/month".to_string(),
    }
}

/// Chart data: cumulative cash, purchase versus financed (Pro).
#[cfg(feature = "pro")]
fn cashflow_chart(
    report: &PaybackReport,
    loan: &tpt_solar_engine::LoanResult,
) -> chart::Chart {
    let years: Vec<String> = (0..report.financials.cumulative_net_cash.len())
        .map(|y| y.to_string())
        .collect();
    chart::Chart {
        series: vec![
            chart::ChartSeries {
                color: "#2e9e6b",
                fill_color: None,
                values: report.financials.cumulative_net_cash.clone(),
                dashed: false,
            },
            chart::ChartSeries {
                color: "#5b7fa6",
                fill_color: None,
                values: loan.cumulative_net_cash.clone(),
                dashed: true,
            },
        ],
        x_labels: years,
        left_title: "Cumulative NZ$".to_string(),
    }
}

/// The reactive scenario-comparison section (Pro).
#[cfg(feature = "pro")]
fn scenario_results_view(outcome: ScenarioListOutcome) -> UITree<Msg> {
    UITree::container(|c| match outcome {
        ScenarioListOutcome::Idle => {
            styled_text(
                c,
                "Fill in at least two scenarios and press \u{201c}Compare scenarios\u{201d}.",
                "sc-detail",
            );
        }
        ScenarioListOutcome::Invalid(issues) => {
            c.container(|list| {
                for issue in issues {
                    styled_text(list, issue, "sc-issue");
                }
            })
            .class("sc-issues");
        }
        ScenarioListOutcome::Solved(outcomes) => {
            // The best payback (of those that pay back) gets highlighted.
            let best_payback = outcomes
                .iter()
                .filter_map(|o| o.report.financials.payback_years)
                .fold(f64::INFINITY, f64::min);

            c.container(|table| {
                table.container(|row| {
                    styled_text(row, "Scenario", "sc-cell");
                    styled_text(row, "Quote", "sc-cellnum");
                    styled_text(row, "Generation kWh/yr", "sc-cellnum");
                    styled_text(row, "Year-1 savings", "sc-cellnum");
                    styled_text(row, "Payback", "sc-cellnum");
                    styled_text(row, "25-yr net", "sc-cellnum");
                })
                .class("sc-thead-wide");
                for outcome in outcomes.iter() {
                    let money = &outcome.report.financials;
                    table.container(|row| {
                        styled_text(
                            row,
                            format!(
                                "{} ({:.1} kWp)",
                                outcome.scenario.name, outcome.scenario.capacity_kw
                            ),
                            "sc-cell",
                        );
                        styled_text(
                            row,
                            dollars(outcome.scenario.installed_cost_nzd),
                            "sc-cellnum",
                        );
                        styled_text(
                            row,
                            format!("{:.0}", outcome.report.balance.annual_generation_kwh),
                            "sc-cellnum",
                        );
                        styled_text(row, dollars(money.year1_savings_nzd), "sc-cellnum");
                        let payback = payback_text(money.payback_years);
                        if money.payback_years == Some(best_payback) {
                            styled_text(row, payback, "sc-cellnum sc-best");
                        } else {
                            styled_text(row, payback, "sc-cellnum");
                        }
                        styled_text(
                            row,
                            dollars(money.lifetime_net_savings_nzd),
                            "sc-cellnum",
                        );
                    })
                    .class("sc-trow-wide");
                }
            })
            .class("sc-table");
        }
    })
}

/// The reactive financing section (Pro).
#[cfg(feature = "pro")]
fn loan_results_view(outcome: LoanOutcome) -> UITree<Msg> {
    UITree::container(|c| match outcome {
        LoanOutcome::Idle => {
            styled_text(
                c,
                "Run an estimate, set the loan terms, and press \u{201c}Calculate \
                 financing\u{201d}.",
                "sc-detail",
            );
        }
        LoanOutcome::Invalid(issues) => {
            c.container(|list| {
                for issue in issues {
                    styled_text(list, issue, "sc-issue");
                }
            })
            .class("sc-issues");
        }
        LoanOutcome::Solved(terms, loan) => {
            // Out-of-pocket = the deposit = the financed position's start.
            let deposit = -loan.cumulative_net_cash.first().copied().unwrap_or(0.0);
            c.container(|summary| {
                styled_text(
                    summary,
                    format!(
                        "{} per month for {} years at {:.2}%.",
                        dollars(loan.monthly_payment_nzd),
                        terms.term_years,
                        terms.annual_interest_rate * 100.0
                    ),
                    "sc-headline",
                );
                styled_text(
                    summary,
                    format!(
                        "The loan adds {} in interest over the term. Break-even on the {} \
                         out-of-pocket: {}; over 25 years the financed position nets {}.",
                        dollars(loan.total_interest_nzd),
                        dollars(deposit),
                        payback_text(loan.payback_years),
                        dollars(loan.lifetime_net_savings_nzd),
                    ),
                    "sc-detail",
                );
            })
            .class("sc-summary");

            c.container(|stats| {
                stat(
                    stats,
                    &format!("{:.2}", loan.monthly_payment_nzd),
                    "Monthly payment",
                );
                stat(
                    stats,
                    &dollars(loan.annual_payment_nzd),
                    "Payments per year",
                );
                stat(stats, &dollars(loan.total_interest_nzd), "Total interest");
                stat(stats, &payback_text(loan.payback_years), "Break-even");
                stat(
                    stats,
                    &dollars(loan.lifetime_net_savings_nzd),
                    "25-yr net financed",
                );
            })
            .class("sc-stats");
        }
    })
}

// ---- DOM field helpers (the same proven implementations the other apps use). ----

fn read_input(container: &web_sys::Element, selector: &str) -> String {
    container
        .query_selector(selector)
        .ok()
        .flatten()
        .and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|input| input.value())
        .or_else(|| {
            container
                .query_selector(selector)
                .ok()
                .flatten()
                .and_then(|el| el.dyn_into::<web_sys::HtmlSelectElement>().ok())
                .map(|select| select.value())
        })
        .unwrap_or_default()
}

fn write_field(container: &web_sys::Element, selector: &str, value: &str) {
    if let Some(el) = container.query_selector(selector).ok().flatten() {
        if let Some(input) = el.dyn_ref::<web_sys::HtmlInputElement>() {
            input.set_value(value);
        } else if let Some(select) = el.dyn_ref::<web_sys::HtmlSelectElement>() {
            select.set_value(value);
        }
    }
}

fn read_css(container: &web_sys::Element, name: &str, fallback: &str) -> String {
    let value = web_sys::window()
        .and_then(|w| w.get_computed_style(container).ok())
        .flatten()
        .and_then(|style| style.get_property_value(name).ok())
        .unwrap_or_default();
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

fn upsert_style(document: &web_sys::Document, id: &str, css: &str) {
    if let Ok(Some(existing)) = document.query_selector(&format!("style#{id}")) {
        existing.set_text_content(Some(css));
        return;
    }
    let Some(head) = document.query_selector("head").ok().flatten() else {
        return;
    };
    if let Ok(style_el) = document.create_element("style") {
        let _ = style_el.set_attribute("id", id);
        style_el.set_text_content(Some(css));
        let _ = head.append_child(&style_el);
    }
}
