//! Pro edition: the PDF quote-negotiation report.
//!
//! Two halves:
//! - a minimal single-purpose PDF 1.4 writer (Helvetica, Letter portrait,
//!   paginated styled lines plus ruled tables) — no external crates, just
//!   a text-stream emitter with a correct xref table;
//! - [`render_report`], which lays out one estimate's inputs, headline
//!   numbers, monthly generation, scenario comparison, and financing
//!   comparison into those rows.
//!
//! The output is deliberately written for the use case of being handed to
//! a solar installer in a quote negotiation: it reads as an independent
//! second opinion, shows the assumptions next to the numbers, and says so.

/// One line in the report body.
pub enum Row {
    /// Document title (18 pt bold).
    Title(String),
    /// Section heading (13 pt bold).
    Head(String),
    /// Body text (10 pt regular).
    Body(String),
    /// Muted note (9 pt regular, gray).
    Note(String),
    /// Vertical gap (points).
    Gap(f64),
    /// A table: header row + data rows, column widths across the text
    /// width, light rules between rows.
    Table {
        /// Header labels, one per column.
        columns: Vec<String>,
        /// Data rows; each inner vec must match `columns` in length.
        rows: Vec<Vec<String>>,
        /// Relative column widths (must sum to ~1.0; default equal).
        weights: Option<Vec<f64>>,
    },
}

/// Renders the rows into a PDF document (bytes).
pub fn render_pdf(_title: &str, rows: &[Row]) -> Vec<u8> {
    const PAGE_W: f64 = 612.0;
    const PAGE_H: f64 = 792.0;
    const MARGIN: f64 = 56.0;
    const TEXT_W: f64 = PAGE_W - 2.0 * MARGIN;
    const LINE: f64 = 15.0;

    // ---- layout: break rows into pages of positioned lines ----
    enum Op {
        Text {
            x: f64,
            y: f64,
            size: f64,
            bold: bool,
            gray: bool,
            text: String,
        },
        Rule {
            y: f64,
        },
    }
    let mut pages: Vec<Vec<Op>> = vec![Vec::new()];
    let mut y = PAGE_H - MARGIN;

    let push_page = |pages: &mut Vec<Vec<Op>>, y: &mut f64| {
        pages.push(Vec::new());
        *y = PAGE_H - MARGIN;
    };

    for row in rows {
        let (size, bold, gray, text) = match row {
            Row::Title(t) => (18.0, true, false, t),
            Row::Head(t) => (13.0, true, false, t),
            Row::Body(t) => (10.0, false, false, t),
            Row::Note(t) => (9.0, false, true, t),
            Row::Gap(h) => {
                y -= h;
                continue;
            }
            Row::Table {
                columns,
                rows: data,
                weights,
            } => {
                let n = columns.len().max(1);
                let widths: Vec<f64> = match weights {
                    Some(w) if w.len() == n => w.iter().map(|&x| x * TEXT_W).collect(),
                    _ => vec![TEXT_W / n as f64; n],
                };
                // Header.
                if y < MARGIN + 3.0 * LINE {
                    push_page(&mut pages, &mut y);
                }
                {
                    let page = pages.last_mut().expect("at least one page");
                    let mut x = MARGIN;
                    for (i, col) in columns.iter().enumerate() {
                        page.push(Op::Text {
                            x,
                            y,
                            size: 10.0,
                            bold: true,
                            gray: false,
                            text: escape(col),
                        });
                        x += widths[i];
                    }
                }
                y -= LINE;
                // Rule under the header:
                pages.last_mut().expect("page").push(Op::Rule { y: y + 4.0 });
                for data_row in data {
                    if y < MARGIN + LINE {
                        push_page(&mut pages, &mut y);
                        // repeat header on continuation pages
                        let page = pages.last_mut().expect("page");
                        let mut x = MARGIN;
                        for (i, col) in columns.iter().enumerate() {
                            page.push(Op::Text {
                                x,
                                y,
                                size: 10.0,
                                bold: true,
                                gray: false,
                                text: escape(col),
                            });
                            x += widths[i];
                        }
                        y -= LINE;
                        pages.last_mut().expect("page").push(Op::Rule { y: y + 4.0 });
                    }
                    let page = pages.last_mut().expect("page");
                    let mut x = MARGIN;
                    for (i, cell) in data_row.iter().enumerate() {
                        page.push(Op::Text {
                            x,
                            y,
                            size: 10.0,
                            bold: false,
                            gray: false,
                            text: escape(cell),
                        });
                        x += widths.get(i).copied().unwrap_or(0.0);
                    }
                    pages.last_mut().expect("page").push(Op::Rule { y: y - 4.0 });
                    y -= LINE;
                }
                pages.last_mut()
                    .expect("page")
                    .push(Op::Rule { y: y + 4.0 + 4.0 });
                y -= 4.0;
                continue;
            }
        };
        if y < MARGIN + LINE {
            push_page(&mut pages, &mut y);
        }
        pages.last_mut().expect("page").push(Op::Text {
            x: MARGIN,
            y,
            size,
            bold,
            gray,
            text: escape(text),
        });
        y -= size + 6.0;
    }

    // ---- emit PDF objects ----
    let n_pages = pages.len();
    // Object numbering: 1 catalog, 2 pages, then per page [page, contents],
    // then fonts: F1 regular, F2 bold.
    let first_page_obj = 3usize;
    let font_regular_obj = first_page_obj + 2 * n_pages;
    let font_bold_obj = font_regular_obj + 1;

    let mut objects: Vec<(usize, Vec<u8>)> = Vec::new();
    objects.push((1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()));
    let kids: Vec<String> = (0..n_pages)
        .map(|i| format!("{} 0 R", first_page_obj + 2 * i))
        .collect();
    objects.push((
        2,
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            n_pages
        )
        .into_bytes(),
    ));

    for (page_index, ops) in pages.iter().enumerate() {
        let page_obj = first_page_obj + 2 * page_index;
        let content_obj = page_obj + 1;
        objects.push((
            page_obj,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] \
                 /Contents {content_obj} 0 R /Resources << /Font \
                 << /F1 {font_regular_obj} 0 R /F2 {font_bold_obj} 0 R >> >> >>"
            )
            .into_bytes(),
        ));

        let mut stream = String::new();
        for op in ops {
            match op {
                Op::Text {
                    x,
                    y,
                    size,
                    bold,
                    gray,
                    text,
                } => {
                    let font = if *bold { "F2" } else { "F1" };
                    let gray_op = if *gray { " 0.45 g" } else { " 0 g" };
                    stream.push_str(&format!(
                        "BT{gray_op} /{font} {size} Tf 1 0 0 1 {x} {y} Tm ({text}) Tj ET\n"
                    ));
                }
                Op::Rule { y } => {
                    stream.push_str(&format!(
                        "0.8 G 0.5 w {MARGIN} {y} m {} {y} l S\n",
                        MARGIN + TEXT_W
                    ));
                }
            }
        }
        objects.push((
            content_obj,
            format!("<< /Length {} >>\nstream\n{}\nendstream", stream.len(), stream)
                .into_bytes(),
        ));
    }

    objects.push((
        font_regular_obj,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ));
    objects.push((
        font_bold_obj,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ));

    // ---- serialize with a correct xref ----
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let mut offsets = vec![0usize; objects.len() + 1];
    for (num, body) in &objects {
        offsets[*num] = out.len();
        out.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for num in 1..=objects.len() {
        out.extend_from_slice(format!("{:010} 00000 n \n", offsets[num]).as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref_at
        )
        .as_bytes(),
    );
    out
}

/// Escapes a string for a PDF literal (parentheses and backslashes), and
/// transliterates the characters WinAnsi can't carry to safe ASCII.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            '\u{b7}' | '\u{2022}' => out.push('\u{b7}'), // middle dot exists in WinAnsi
            '\u{d7}' => out.push('x'),                   // multiplication sign -> x
            '\u{2248}' => out.push('~'),                 // almost-equal
            '\u{2013}' | '\u{2014}' => out.push('-'),    // en/em dash
            '\u{2018}' | '\u{2019}' => out.push('\''),   // curly single quotes
            '\u{201c}' | '\u{201d}' => out.push('"'),    // curly double quotes
            c if (c as u32) < 128 => out.push(c),
            c if (c as u32) < 256 => out.push(c), // WinAnsi range
            _ => out.push('?'),
        }
    }
    out
}

/// Formats a dollar amount with thousands separators (`12500 -> "12,500"`).
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

/// Formats a payback period for the report (`None` reads as "over 25 yrs").
fn payback_text(payback: Option<f64>) -> String {
    match payback {
        Some(years) => format!("{years:.1} yrs"),
        None => "over 25 yrs".to_string(),
    }
}

/// Lays out one estimate (plus optional scenario comparison and financing)
/// into report rows.
pub fn render_report(
    site: &crate::Site,
    system: &crate::SolarSystem,
    tariff: &crate::Tariff,
    household: &crate::Household,
    report: &crate::PaybackReport,
    discount_rate: f64,
    scenarios: Option<&[crate::ScenarioOutcome]>,
    loan: Option<(&crate::LoanTerms, &crate::LoanResult)>,
) -> Vec<Row> {
    use crate::MONTH_NAMES;

    let balance = &report.balance;
    let financials = &report.financials;

    let mut rows = vec![
        Row::Title("TPT Solar Checker — Payback Report".to_string()),
        Row::Note(
            "Independent second opinion — prepared with TPT Solar Checker Pro. \
             TPT Solutions does not sell or install solar systems."
                .to_string(),
        ),
        Row::Gap(6.0),
        Row::Head("Assumptions".to_string()),
        Row::Table {
            columns: vec![
                "Input".to_string(),
                "Value".to_string(),
                "Input".to_string(),
                "Value".to_string(),
            ],
            rows: vec![
                vec![
                    "Location".to_string(),
                    format!(
                        "{:.2}°, {:.2}° (UTC{:+})",
                        site.latitude_deg, site.longitude_deg, site.timezone_offset_hours
                    ),
                    "System size".to_string(),
                    format!("{:.1} kWp", system.capacity_kw),
                ],
                vec![
                    "Roof".to_string(),
                    format!(
                        "{:.0}° tilt, {:.0}° azimuth ({}-facing)",
                        system.tilt_deg,
                        system.azimuth_deg,
                        if system.azimuth_deg < 90.0 || system.azimuth_deg > 270.0 {
                            "north"
                        } else if (90.0..=270.0).contains(&system.azimuth_deg) {
                            "south"
                        } else {
                            "east/west"
                        }
                    ),
                    "Installed quote".to_string(),
                    dollars(system.installed_cost_nzd),
                ],
                vec![
                    "Consumption".to_string(),
                    format!("{:.0} kWh/yr", household.annual_kwh),
                    "Import rate".to_string(),
                    format!("{:.1} c/kWh", tariff.import_nzd_per_kwh * 100.0),
                ],
                vec![
                    "Feed-in rate".to_string(),
                    format!("{:.1} c/kWh", tariff.feed_in_nzd_per_kwh * 100.0),
                    "Price rise".to_string(),
                    format!("{:.1}%/yr", tariff.annual_escalation * 100.0),
                ],
                vec![
                    "Degradation".to_string(),
                    format!("{:.1}%/yr", crate::PANEL_DEGRADATION_PER_YEAR * 100.0),
                    "Other system losses".to_string(),
                    format!("{:.0}% (wiring, mismatch, availability)", crate::OTHER_SYSTEM_LOSSES * 100.0),
                ],
                vec![
                    "Discount rate".to_string(),
                    format!("{:.1}%", discount_rate * 100.0),
                    "Horizon".to_string(),
                    format!("{} years", crate::LIFETIME_YEARS),
                ],
            ],
            weights: Some(vec![0.18, 0.32, 0.18, 0.32]),
        },
        Row::Head("Results — cash purchase".to_string()),
        Row::Table {
            columns: vec!["Measure".to_string(), "Value".to_string()],
            rows: vec![
                vec![
                    "Year-1 generation".to_string(),
                    format!("{:.0} kWh", balance.annual_generation_kwh),
                ],
                vec![
                    "Self-consumed on-site".to_string(),
                    format!(
                        "{:.0} kWh ({:.0}%)",
                        balance.self_consumed_kwh,
                        balance.self_consumption_fraction() * 100.0
                    ),
                ],
                vec![
                    "Exported to grid".to_string(),
                    format!("{:.0} kWh", balance.exported_kwh),
                ],
                vec![
                    "Year-1 bill savings".to_string(),
                    dollars(financials.year1_savings_nzd),
                ],
                vec![
                    "Payback period".to_string(),
                    payback_text(financials.payback_years),
                ],
                vec![
                    "25-yr lifetime net savings".to_string(),
                    dollars(financials.lifetime_net_savings_nzd),
                ],
                vec![
                    "Return on investment".to_string(),
                    match financials.irr {
                        Some(irr) => format!("IRR {:.1}%", irr * 100.0),
                        None => "not bracketable".to_string(),
                    },
                ],
                vec![
                    "Net present value".to_string(),
                    format!(
                        "{} at {:.1}%",
                        dollars(financials.npv_nzd),
                        discount_rate * 100.0
                    ),
                ],
            ],
            weights: Some(vec![0.5, 0.5]),
        },
        Row::Head("Generation by month".to_string()),
        Row::Table {
            columns: vec![
                "Month".to_string(),
                "Generated".to_string(),
                "Consumed".to_string(),
                "Month".to_string(),
                "Generated".to_string(),
                "Consumed".to_string(),
            ],
            rows: (0..6)
                .map(|i| {
                    let other = i + 6;
                    vec![
                        MONTH_NAMES[i].to_string(),
                        format!("{:.0} kWh", balance.monthly_generation_kwh[i]),
                        format!("{:.0} kWh", balance.monthly_consumption_kwh[i]),
                        MONTH_NAMES[other].to_string(),
                        format!("{:.0} kWh", balance.monthly_generation_kwh[other]),
                        format!("{:.0} kWh", balance.monthly_consumption_kwh[other]),
                    ]
                })
                .collect(),
            weights: Some(vec![0.22, 0.19, 0.19, 0.22, 0.19, 0.19]),
        },
    ];

    if let Some(outcomes) = scenarios {
        if !outcomes.is_empty() {
            rows.push(Row::Head("Scenario comparison".to_string()));
            rows.push(Row::Table {
                columns: vec![
                    "Scenario".to_string(),
                    "Quote".to_string(),
                    "Year-1 savings".to_string(),
                    "Payback".to_string(),
                    "25-yr net".to_string(),
                ],
                rows: outcomes
                    .iter()
                    .map(|outcome| {
                        vec![
                            format!(
                                "{} ({:.1} kWp)",
                                outcome.scenario.name, outcome.scenario.capacity_kw
                            ),
                            dollars(outcome.scenario.installed_cost_nzd),
                            dollars(outcome.report.financials.year1_savings_nzd),
                            payback_text(outcome.report.financials.payback_years),
                            dollars(outcome.report.financials.lifetime_net_savings_nzd),
                        ]
                    })
                    .collect(),
                weights: Some(vec![0.32, 0.15, 0.18, 0.16, 0.19]),
            });
        }
    }

    if let Some((terms, loan)) = loan {
        rows.push(Row::Head("Financing comparison".to_string()));
        rows.push(Row::Table {
            columns: vec!["Measure".to_string(), "Value".to_string()],
            rows: vec![
                vec![
                    "Loan amount".to_string(),
                    dollars(loan.loan_amount_nzd),
                ],
                vec![
                    format!(
                        "Monthly payment ({:.2}%, {} yrs)",
                        terms.annual_interest_rate * 100.0,
                        terms.term_years
                    ),
                    format!("{:.2}", loan.monthly_payment_nzd),
                ],
                vec![
                    "Total interest over the term".to_string(),
                    dollars(loan.total_interest_nzd),
                ],
                vec![
                    "Break-even on out-of-pocket".to_string(),
                    payback_text(loan.payback_years),
                ],
                vec![
                    "25-yr lifetime net (financed)".to_string(),
                    dollars(loan.lifetime_net_savings_nzd),
                ],
            ],
            weights: Some(vec![0.5, 0.5]),
        });
    }

    rows.push(Row::Gap(4.0));
    rows.push(Row::Note(
        "Model: hourly clear-sky simulation (typical New Zealand cloud and temperature \
         by month), representative day per month. Actual output varies with weather, \
         shading, panel soiling beyond the modelled 2%, inverter replacement, and \
         installation quality. Estimates are a second opinion, not a guarantee; check \
         any quote against your own consumption data."
            .to_string(),
    ));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        estimate, Household, LoanResult, LoanTerms, PaybackReport, Scenario, Site, SolarSystem,
        Tariff,
    };

    #[test]
    fn produces_a_parseable_pdf() {
        let rows = [
            Row::Title("Solar Payback Report".to_string()),
            Row::Note("TPT Solar Checker Pro (test)".to_string()),
            Row::Gap(6.0),
            Row::Head("Results".to_string()),
            Row::Table {
                columns: ["Measure".to_string(), "Value".to_string()].to_vec(),
                rows: vec![
                    vec!["Payback period".to_string(), "12.9 yrs".to_string()],
                    vec![
                        "25-yr lifetime net savings".to_string(),
                        "$4,900".to_string(),
                    ],
                ],
                weights: Some(vec![0.5, 0.5]),
            },
        ];
        let pdf = render_pdf("Solar Report", &rows);
        let text = String::from_utf8_lossy(&pdf);
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(text.trim_end().ends_with("%%EOF"));
        assert!(text.contains("/Count 1"));
        assert!(text.contains("Payback period"));
        // Every object offset in the xref must point at "N 0 obj".
        let xref_at = text.rfind("xref").expect("xref");
        for line in text[xref_at..].lines().skip(2) {
            if line.len() == 20 {
                let offset: usize = line[..10].trim().parse().unwrap();
                if offset > 0 {
                    assert!(text[offset..].starts_with(char::is_numeric as fn(char) -> bool));
                }
            }
        }
    }

    #[test]
    fn dollars_groups_thousands() {
        assert_eq!(dollars(0.0), "$0");
        assert_eq!(dollars(999.0), "$999");
        assert_eq!(dollars(11_500.0), "$11,500");
        assert_eq!(dollars(1_234_567.0), "$1,234,567");
        assert_eq!(dollars(-4_200.0), "-$4,200");
    }

    #[test]
    fn renders_a_full_report_from_a_real_estimate() {
        let site = Site::default();
        let system = SolarSystem::default();
        let tariff = Tariff::default();
        let household = Household::default();
        let report: PaybackReport = estimate(&site, &system, &tariff, &household, 0.07);

        let scenarios = vec![Scenario {
            name: "Bigger".to_string(),
            capacity_kw: 8.0,
            installed_cost_nzd: 16_000.0,
        }];
        let outcomes = crate::compare_scenarios(&site, &tariff, &household, 0.07, &scenarios);
        let terms = LoanTerms::default();
        let loan: LoanResult = crate::analyze_financing(&report, system.installed_cost_nzd, &terms);

        let rows = render_report(
            &site,
            &system,
            &tariff,
            &household,
            &report,
            0.07,
            Some(&outcomes),
            Some((&terms, &loan)),
        );
        let pdf = render_pdf("TPT Solar Checker", &rows);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("Scenario comparison"));
        assert!(text.contains("Financing comparison"));
        assert!(text.contains("Independent"));
        assert!(text.starts_with("%PDF-1.4"));
    }
}
