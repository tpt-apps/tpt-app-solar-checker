//! Host-run validation against hand-calculated reference scenarios.
//!
//! The physics (clear-sky PV through `tpt-nrg-solar`) can only be bounded,
//! not hand-checked, so the money math is separated from it: [`financials`]
//! and [`analyze_financing`] take an [`EnergyBalance`] or savings stream as
//! input, and every number they produce is reproducible by hand. The
//! physics gets plausibility bounds instead (Wellington yields, seasonal
//! and orientation ordering, energy conservation).

use tpt_solar_engine::{
    analyze_financing, compare_scenarios, energy_balance, estimate, financials, monthly_payment,
    EnergyBalance, Household, LoanTerms, PaybackReport, Scenario, Site, SolarSystem, Tariff,
    LIFETIME_YEARS, PANEL_DEGRADATION_PER_YEAR,
};

/// A hand-built balance: 4,000 kWh generated, half self-consumed, half
/// exported, 7,000 kWh consumed. Simple numbers on purpose — every
/// assertion below is checkable with a calculator.
fn reference_balance() -> EnergyBalance {
    EnergyBalance {
        system_size_kw: 4.0,
        annual_generation_kwh: 4_000.0,
        annual_consumption_kwh: 7_000.0,
        self_consumed_kwh: 2_000.0,
        exported_kwh: 2_000.0,
        grid_import_without_solar_kwh: 7_000.0,
        grid_import_with_solar_kwh: 5_000.0,
        monthly_generation_kwh: [0.0; 12],
        monthly_consumption_kwh: [0.0; 12],
        monthly_self_consumed_kwh: [0.0; 12],
        monthly_exported_kwh: [0.0; 12],
    }
}

fn flat_tariff() -> Tariff {
    Tariff {
        import_nzd_per_kwh: 0.30,
        feed_in_nzd_per_kwh: 0.08,
        annual_escalation: 0.0,
    }
}

#[test]
fn hand_calculated_reference_payback() {
    // $2,000 x $0.30 + 2,000 x $0.08 = $600 + $160 = $760/yr. With no
    // escalation the savings stream is $760 degraded 0.5%/yr, so year t
    // saves 760 x 0.995^(t-1) — a geometric series with closed-form
    // partial sums, all of which a calculator can reproduce:
    //   cum(k) = 760 x (1 - 0.995^k) / 0.005.
    let balance = reference_balance();
    let tariff = flat_tariff();
    let f = financials(&balance, 10_000.0, &tariff, 0.07);

    assert!((f.year1_savings_nzd - 760.0).abs() < 1e-9);
    let d = PANEL_DEGRADATION_PER_YEAR;
    let cum = |k: u32| 760.0 * (1.0 - (1.0 - d).powi(k as i32)) / d;
    // Lifetime net and NPV check out exactly against the same series.
    assert!((f.lifetime_net_savings_nzd - (cum(LIFETIME_YEARS) - 10_000.0)).abs() < 1e-6);
    let expected_npv: f64 = (0..LIFETIME_YEARS)
        .map(|i| 760.0 * (1.0 - d).powi(i as i32) / 1.07f64.powi(i as i32 + 1))
        .sum::<f64>()
        - 10_000.0;
    assert!((f.npv_nzd - expected_npv).abs() < 1e-6);

    // The break-even search must land in the first year the closed-form
    // cumulative turns positive, interpolated within that year.
    let expected_payback = (1..=LIFETIME_YEARS)
        .find_map(|k| {
            let before = cum(k - 1) - 10_000.0;
            let after = cum(k) - 10_000.0;
            (before < 0.0 && after >= 0.0)
                .then(|| k as f64 - 1.0 + (-before / (after - before)))
        })
        .expect("pays back");
    let payback = f.payback_years.expect("pays back");
    assert!((payback - expected_payback).abs() < 1e-9);

    // The IRR of the degraded annuity: verify by substitution — NPV at the
    // found rate must be ~0.
    let irr = f.irr.expect("degraded annuity brackets an IRR");
    let npv_at_irr: f64 = std::iter::once(-10_000.0)
        .chain(f.annual_savings.iter().copied())
        .enumerate()
        .map(|(t, cf)| cf / (1.0 + irr).powi(t as i32))
        .sum();
    assert!(npv_at_irr.abs() < 1e-4, "NPV at IRR was {npv_at_irr}");
}

#[test]
fn escalation_and_degradation_hand_check() {
    // Year-1 = $600 exactly (2,000 kWh self-consumed at $0.30, the 2,000
    // exported earn nothing), growing 10%/yr against a 0.5%/yr degradation.
    let balance = reference_balance();
    let tariff = Tariff {
        import_nzd_per_kwh: 0.30,
        feed_in_nzd_per_kwh: 0.0,
        annual_escalation: 0.10,
    };
    let f = financials(&balance, 1_000.0, &tariff, 0.05);
    assert!((f.year1_savings_nzd - 600.0).abs() < 1e-9);
    let year2 = f.annual_savings[1];
    assert!((year2 - 600.0 * 1.1 * (1.0 - PANEL_DEGRADATION_PER_YEAR)).abs() < 1e-9);
    // cum(1) = 600 < 1,000 <= 600 + year2: the break-even is year 2,
    // interpolated with year-2 savings.
    let payback = f.payback_years.expect("pays back");
    assert!((payback - (1.0 + 400.0 / year2)).abs() < 1e-9);

    // Degradation alone: year 2 = 760 x 0.995.
    let f = financials(&reference_balance(), 10_000.0, &flat_tariff(), 0.07);
    assert!((f.annual_savings[1] - 760.0 * (1.0 - PANEL_DEGRADATION_PER_YEAR)).abs() < 1e-9);
    // Cumulative cash starts at the negative cost and ends at the lifetime net.
    assert!((f.cumulative_net_cash[0] - (-10_000.0)).abs() < 1e-9);
    assert!(
        (f.cumulative_net_cash[LIFETIME_YEARS as usize] - f.lifetime_net_savings_nzd).abs() < 1e-6
    );
}

#[test]
fn payback_none_when_never_breaks_even() {
    // $1/yr of savings against a $10,000 system never pays back.
    let balance = EnergyBalance {
        system_size_kw: 0.01,
        annual_generation_kwh: 10.0,
        annual_consumption_kwh: 7_000.0,
        self_consumed_kwh: 10.0,
        exported_kwh: 0.0,
        grid_import_without_solar_kwh: 7_000.0,
        grid_import_with_solar_kwh: 6_990.0,
        monthly_generation_kwh: [0.0; 12],
        monthly_consumption_kwh: [0.0; 12],
        monthly_self_consumed_kwh: [0.0; 12],
        monthly_exported_kwh: [0.0; 12],
    };
    let f = financials(&balance, 10_000.0, &flat_tariff(), 0.07);
    assert!(f.payback_years.is_none());
    assert!(f.lifetime_net_savings_nzd < 0.0);
}

#[test]
fn zero_cost_pays_back_immediately() {
    let f = financials(&reference_balance(), 0.0, &flat_tariff(), 0.07);
    assert_eq!(f.payback_years, Some(0.0));
}

#[test]
fn loan_amortization_closes() {
    // Independent check of the annuity: simulate the amortization schedule
    // month by month and the balance must land on ~zero at the term.
    let principal = 12_000.0;
    let rate = 0.12;
    let term = 5;
    let payment = monthly_payment(principal, rate, term);
    assert!(payment > 0.0);

    let monthly_rate = rate / 12.0;
    let mut balance = principal;
    for _ in 0..(term * 12) {
        let interest = balance * monthly_rate;
        balance = balance + interest - payment;
    }
    assert!(balance.abs() < 1e-6, "amortized to {balance}");

    // Zero interest divides the principal evenly.
    assert!((monthly_payment(12_000.0, 0.0, 5) - 200.0).abs() < 1e-9);
}

#[test]
fn financed_results_hand_check() {
    // $10,000 system, $2,000 deposit, interest-free over 5 years:
    // payments are 8,000/5 = $1,600/yr; the financed position starts at
    // -2,000 and nets savings - 1,600 for years 1-5.
    let report = {
        let balance = reference_balance();
        let money = financials(&balance, 10_000.0, &flat_tariff(), 0.07);
        PaybackReport {
            balance,
            financials: money,
        }
    };
    let terms = LoanTerms {
        deposit_nzd: 2_000.0,
        annual_interest_rate: 0.0,
        term_years: 5,
    };
    let loan = analyze_financing(&report, 10_000.0, &terms);
    assert!((loan.loan_amount_nzd - 8_000.0).abs() < 1e-9);
    assert!((loan.monthly_payment_nzd - 8_000.0 / 60.0).abs() < 1e-9);
    assert!((loan.total_interest_nzd).abs() < 1e-9);
    assert!((loan.annual_payment_nzd - 1_600.0).abs() < 1e-9);
    // Year-1 net = 760 - 1,600 = -840 (cum -2,840), and the loan ends
    // after year 5, so break-even must happen after the term.
    let payback = loan.payback_years.expect("interest-free loan breaks even");
    assert!(payback > terms.term_years as f64);
    // Interest-free financing never does worse than cash over the horizon.
    assert!(loan.lifetime_net_savings_nzd >= report.financials.lifetime_net_savings_nzd - 1e-6);
}

#[test]
fn full_deposit_is_just_cash() {
    let report = estimate(&Site::default(), &SolarSystem::default(), &Tariff::default(), &Household::default(), 0.07);
    let terms = LoanTerms {
        deposit_nzd: 11_500.0,
        annual_interest_rate: 0.109,
        term_years: 7,
    };
    let loan = analyze_financing(&report, 11_500.0, &terms);
    assert_eq!(loan.loan_amount_nzd, 0.0);
    assert_eq!(loan.monthly_payment_nzd, 0.0);
    assert_eq!(loan.total_interest_nzd, 0.0);
    assert_eq!(loan.payback_years, report.financials.payback_years);
}

#[test]
fn tiny_system_self_consumes_everything() {
    // A 0.05 kWp array next to a 7,000 kWh/yr load: every generated kWh
    // is consumed on-site (the load never drops below the tiny output),
    // so export is ~0 and the residual import is consumption - generation.
    let site = Site::default();
    let system = SolarSystem {
        capacity_kw: 0.05,
        ..SolarSystem::default()
    };
    let household = Household::default();
    let balance = energy_balance(&site, &system, &household);
    assert!(
        (balance.self_consumed_kwh - balance.annual_generation_kwh).abs() < 1.0,
        "self-consumed {:.1} of {:.1} generated",
        balance.self_consumed_kwh,
        balance.annual_generation_kwh
    );
    assert!(balance.exported_kwh < 1.0);
    assert!(
        (balance.grid_import_with_solar_kwh - (household.annual_kwh - balance.self_consumed_kwh)).abs()
            < 1e-6
    );
    // Without solar, the grid carries the entire load.
    assert_eq!(balance.grid_import_without_solar_kwh, household.annual_kwh);
}

#[test]
fn wellington_generation_is_plausible() {
    // Wellington yields ~1,000-1,400 kWh/kWp/yr for a north-facing roof
    // after the balance-of-system derate; bound generously either side to
    // catch physics regressions without being brittle.
    let site = Site::default();
    let system = SolarSystem::default();
    let balance = energy_balance(&site, &system, &Household::default());
    let yield_per_kwp = balance.annual_generation_kwh / system.capacity_kw;
    assert!(
        (1_000.0..=1_550.0).contains(&yield_per_kwp),
        "yield was {yield_per_kwp} kWh/kWp"
    );
    // Self-consumption for this load/system mix should land in a sane band:
    // not everything exported, not everything used.
    let fraction = balance.self_consumption_fraction();
    assert!(
        (0.15..=0.75).contains(&fraction),
        "self-consumption was {fraction}"
    );
    // Energy conservation: self-consumed + exported = generation; residual
    // import + self-consumed = consumption.
    assert!(
        (balance.self_consumed_kwh + balance.exported_kwh - balance.annual_generation_kwh).abs()
            < 1e-6
    );
    assert!(
        (balance.grid_import_with_solar_kwh + balance.self_consumed_kwh
            - balance.annual_consumption_kwh)
            .abs()
            < 1e-6
    );
}

#[test]
fn summer_beats_winter_and_north_beats_south() {
    let site = Site::default();
    let household = Household::default();

    let north = energy_balance(&site, &SolarSystem::default(), &household);
    let south = energy_balance(
        &site,
        &SolarSystem {
            azimuth_deg: 180.0,
            ..SolarSystem::default()
        },
        &household,
    );
    assert!(
        north.annual_generation_kwh > south.annual_generation_kwh,
        "north {:.0} vs south {:.0}",
        north.annual_generation_kwh,
        south.annual_generation_kwh
    );
    // December (index 11) out-produces June (index 5) in the southern
    // hemisphere by a wide margin.
    assert!(
        north.monthly_generation_kwh[11] > north.monthly_generation_kwh[5] * 2.5,
        "December {:.0} vs June {:.0}",
        north.monthly_generation_kwh[11],
        north.monthly_generation_kwh[5]
    );
}

#[test]
fn vertical_tilt_loses_badly_to_30_degrees() {
    // A vertical array is for facades, not roofs: at Wellington's latitude
    // it must give up a large chunk of the annual yield of a ~latitude
    // tilt. (Fine 10°-vs-60° orderings are within the model's uncertainty
    // and are deliberately not asserted.)
    let site = Site::default();
    let household = Household::default();
    let roof = energy_balance(&site, &SolarSystem::default(), &household);
    let vertical = energy_balance(
        &site,
        &SolarSystem {
            tilt_deg: 90.0,
            ..SolarSystem::default()
        },
        &household,
    );
    assert!(
        vertical.annual_generation_kwh < roof.annual_generation_kwh * 0.8,
        "vertical {:.0} vs 30° {:.0}",
        vertical.annual_generation_kwh,
        roof.annual_generation_kwh
    );
}

#[test]
fn scenario_comparison_ranks_sensibly() {
    // Two quotes for the same site: the cheaper small system pays back
    // sooner; the bigger system saves more in absolute terms per year.
    let site = Site::default();
    let tariff = Tariff::default();
    let household = Household::default();
    let outcomes = compare_scenarios(
        &site,
        &tariff,
        &household,
        0.07,
        &[
            Scenario {
                name: "small".to_string(),
                capacity_kw: 3.0,
                installed_cost_nzd: 7_500.0,
            },
            Scenario {
                name: "large".to_string(),
                capacity_kw: 8.0,
                installed_cost_nzd: 17_000.0,
            },
        ],
    );
    assert_eq!(outcomes.len(), 2);
    let small = &outcomes[0];
    let large = &outcomes[1];

    // Physics: the 8 kWp array generates ~8/3 x the 3 kWp array's energy
    // (minus clipping effects — assert the coarse ordering only).
    assert!(large.report.balance.annual_generation_kwh > small.report.balance.annual_generation_kwh);
    // Money: the cheap small system breaks even sooner.
    let small_payback = small.report.financials.payback_years.expect("pays back");
    let large_payback = large.report.financials.payback_years.expect("pays back");
    assert!(small_payback < large_payback);
    // Both reports share the site's consumption.
    assert_eq!(
        small.report.balance.annual_consumption_kwh,
        large.report.balance.annual_consumption_kwh
    );
}

#[test]
fn east_and_west_face_symmetrically() {
    // Regression guard for the tpt-nrg-solar hour-angle wrap fix: with the
    // old unwrapped solar hour, NZ mornings reported the sun in the west,
    // so an east-facing array simulated to a fraction of its west-facing
    // twin. East/west must now be symmetric, and north (0°) must beat both
    // comfortably at a southern-hemisphere site.
    let site = Site::default();
    let household = Household::default();
    let east = energy_balance(
        &site,
        &SolarSystem {
            azimuth_deg: 90.0,
            ..SolarSystem::default()
        },
        &household,
    );
    let west = energy_balance(
        &site,
        &SolarSystem {
            azimuth_deg: 270.0,
            ..SolarSystem::default()
        },
        &household,
    );
    let north = energy_balance(&site, &SolarSystem::default(), &household);
    let ratio = east.annual_generation_kwh / west.annual_generation_kwh;
    assert!((0.97..=1.03).contains(&ratio), "east/west ratio was {ratio}");
    assert!(north.annual_generation_kwh > east.annual_generation_kwh * 1.2);
    assert!(north.annual_generation_kwh > west.annual_generation_kwh * 1.2);
}

#[test]
fn validation_flags_out_of_range_inputs() {
    let issues = Site {
        latitude_deg: 200.0,
        ..Site::default()
    }
    .validate();
    assert!(issues.iter().any(|i| i.contains("Latitude")));

    let issues = SolarSystem {
        capacity_kw: 0.0,
        installed_cost_nzd: -5.0,
        ..SolarSystem::default()
    }
    .validate();
    assert!(issues.iter().any(|i| i.contains("System size")));
    assert!(issues.iter().any(|i| i.contains("quote")));

    let issues = Tariff {
        feed_in_nzd_per_kwh: 0.50,
        import_nzd_per_kwh: 0.30,
        ..Tariff::default()
    }
    .validate();
    assert!(issues.iter().any(|i| i.contains("Feed-in")));

    let issues = Household { annual_kwh: 0.0 }.validate();
    assert!(!issues.is_empty());

    let issues = LoanTerms {
        annual_interest_rate: 0.9,
        ..LoanTerms::default()
    }
    .validate();
    assert!(issues.iter().any(|i| i.contains("Interest")));
}
