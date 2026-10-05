//! TPT Solar Checker engine — vendor-neutral solar payback estimation for
//! a single property.
//!
//! A thin, UI-free adapter over `tpt-energy`'s solar model
//! (`tpt-nrg-solar`), host-unit-testable on any target:
//!
//! - [`SolarModel`] + `PvPlant` give hourly PV AC output (NOCT thermal
//!   derating, inverter clipping, soiling),
//! - `tpt-nrg-lcoe` gives NPV and IRR over the lifetime cash flows,
//! - this crate supplies what tpt-energy has no crate for: the retail
//!   tariff overlay (import rate, feed-in rate, price escalation), the
//!   hourly generation-vs-consumption netting that splits self-consumed
//!   from exported solar, the payback-period search, and (Pro) the loan
//!   annuity math and scenario comparison.
//!
//! Units: the public API speaks **kW / kWh / NZD** (what the UI shows);
//! conversions to the MW / MWp the tpt-energy models use happen at the
//! calls into those crates. Money is a plain `f64` in dollars — the app
//! labels it NZ$ because the defaults (tariff shapes, feed-in rates) are
//! New Zealand residential.
//!
//! Time model: hourly steps over representative days (the 21st of each
//! month) with the site's fixed UTC offset — daylight saving is not
//! modelled. `tpt-nrg-solar` computes solar geometry from UTC instants
//! directly, so hour *h* local maps to UTC `h − timezone_offset_hours`.
//!
//! Weather: `tpt-nrg-solar` is a clear-sky model and keeps its irradiance
//! functions private, so cloudiness is applied through the public output
//! path — take the clear-sky plane-of-array irradiance from
//! `PvPlant::output_clearsky`, undo the soiling factor, scale by the month's
//! cloud factor, and re-run through `PvPlant::output_from_poa` (which
//! re-applies soiling, cell-temperature derating, and inverter clipping).
//! The cloud/temperature tables are New Zealand averages, not per-city —
//! location drives the sun geometry only. On top of the PV model sits an
//! explicit balance-of-system derate ([`OTHER_SYSTEM_LOSSES`]): the
//! estimate should err against the quote, not for it.

#![deny(missing_docs)]

use chrono::{Duration, NaiveDate, TimeZone, Utc};
use tpt_nrg_solar::{PvPlant, PvPlantConfig, SolarModel};

/// Pro edition: the PDF quote-negotiation report (a dependency-free PDF
/// 1.4 writer plus the report layout for one estimate).
#[cfg(feature = "pro")]
pub mod pdf;

/// Hours per simulated day.
pub const HOURS_PER_DAY: usize = 24;
/// Months per simulated year.
pub const MONTHS_PER_YEAR: usize = 12;
/// Representative calendar day simulated for every month.
const REPRESENTATIVE_DAY: u32 = 21;
/// Day counts per month for `REPRESENTATIVE_YEAR` (non-leap) — each
/// representative day is scaled by its month's length.
const MONTH_DAYS: [u64; MONTHS_PER_YEAR] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
/// Calendar year used for solar geometry (declination differs between
/// years by well under the model's accuracy).
const REPRESENTATIVE_YEAR: i32 = 2026;
/// Soiling loss fraction applied by the PV model (matches the
/// `PvPlantConfig` default; restated here because the cloud path inverts
/// and re-applies it).
const SOILING_LOSS: f64 = 0.02;
/// Analysis horizon for payback/lifetime-savings reporting.
pub const LIFETIME_YEARS: u32 = 25;
/// Yearly fraction of nameplate output lost to panel degradation — the
/// industry-standard ~0.5%/yr for crystalline silicon.
pub const PANEL_DEGRADATION_PER_YEAR: f64 = 0.005;
/// Balance-of-system losses beyond the modelled soiling/thermal/inverter
/// terms — wiring, mismatch, availability (PVWatts-style 14% default).
/// Applied on top of the PV model so the estimate errs conservative: a
/// "second opinion" that flatters the quote is worthless.
pub const OTHER_SYSTEM_LOSSES: f64 = 0.14;

/// Month names, January first — chart/report labels.
pub const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Fraction of clear-sky irradiance reaching the array, per month
/// (New Zealand temperate average: sunny midsummer, gloomy midwinter).
const MONTH_CLOUD: [f64; MONTHS_PER_YEAR] = [
    0.62, 0.60, 0.56, 0.52, 0.48, 0.46, 0.46, 0.50, 0.55, 0.58, 0.60, 0.64,
];

/// Monthly mean ambient temperature, °C (drives the NOCT cell-temperature
/// derating).
const MONTH_AMBIENT_C: [f64; MONTHS_PER_YEAR] = [
    17.0, 17.4, 16.2, 14.1, 12.0, 10.2, 9.5, 9.9, 11.3, 12.9, 14.5, 16.3,
];

/// Monthly load multipliers (winter-peaked, New Zealand temperate).
/// Normalized together with [`MONTH_DAYS`] so the year sums to the
/// household's annual consumption exactly — see
/// [`monthly_consumption_kwh`].
const MONTH_LOAD_FACTOR: [f64; MONTHS_PER_YEAR] = [
    0.90, 0.90, 0.98, 1.05, 1.12, 1.18, 1.20, 1.15, 1.08, 1.00, 0.92, 0.90,
];

/// Shape of a residential day: 24 hourly fractions of that day's total
/// consumption (morning and evening peaks, overnight baseload). Normalized
/// to sum to 1 in [`HOURLY_LOAD_SHAPE`] so the constants above stay
/// hand-editable.
const HOURLY_LOAD_SHAPE_RAW: [f64; HOURS_PER_DAY] = [
    0.028, 0.024, 0.021, 0.020, 0.020, 0.023, 0.034, 0.052, 0.055, 0.048, //
    0.042, 0.040, 0.044, 0.042, 0.040, 0.040, 0.044, 0.054, 0.066, 0.072, //
    0.068, 0.060, 0.048, 0.035,
];

/// [`HOURLY_LOAD_SHAPE_RAW`] normalized to sum to exactly 1.
fn hourly_load_shape() -> [f64; HOURS_PER_DAY] {
    let total: f64 = HOURLY_LOAD_SHAPE_RAW.iter().sum();
    let mut shape = [0.0; HOURS_PER_DAY];
    for (h, fraction) in shape.iter_mut().enumerate() {
        *fraction = HOURLY_LOAD_SHAPE_RAW[h] / total;
    }
    shape
}

/// Consumption split across months so the year sums to `annual_kwh`
/// exactly (monthly load factors weighted by month length, normalized).
pub fn monthly_consumption_kwh(annual_kwh: f64) -> [f64; MONTHS_PER_YEAR] {
    let weights: Vec<f64> = (0..MONTHS_PER_YEAR)
        .map(|m| MONTH_LOAD_FACTOR[m] * MONTH_DAYS[m] as f64)
        .collect();
    let total: f64 = weights.iter().sum();
    let mut months = [0.0; MONTHS_PER_YEAR];
    for (m, month) in months.iter_mut().enumerate() {
        *month = annual_kwh * weights[m] / total;
    }
    months
}

/// Physical site of the property.
#[derive(Debug, Clone)]
pub struct Site {
    /// Degrees north (negative = southern hemisphere).
    pub latitude_deg: f64,
    /// Degrees east.
    pub longitude_deg: f64,
    /// Fixed UTC offset in hours (e.g. `12.0` for NZST).
    pub timezone_offset_hours: f64,
    /// Metres above sea level (enters the clear-sky model).
    pub altitude_m: f64,
}

impl Default for Site {
    /// Wellington, NZ — the tool's home audience.
    fn default() -> Self {
        Self {
            latitude_deg: -41.2866,
            longitude_deg: 174.7756,
            timezone_offset_hours: 12.0,
            altitude_m: 30.0,
        }
    }
}

impl Site {
    /// Validity errors, phrased for direct display in the UI.
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if !(-90.0..=90.0).contains(&self.latitude_deg) {
            issues.push("Latitude must be between -90 and 90 degrees.".to_string());
        }
        if !(-180.0..=180.0).contains(&self.longitude_deg) {
            issues.push("Longitude must be between -180 and 180 degrees.".to_string());
        }
        if !(-14.0..=14.0).contains(&self.timezone_offset_hours) {
            issues.push("UTC offset must be between -14 and +14 hours.".to_string());
        }
        if !(-500.0..=9000.0).contains(&self.altitude_m) {
            issues.push("Altitude must be between -500 and 9000 metres.".to_string());
        }
        issues
    }
}

/// The proposed photovoltaic system and its installed price.
#[derive(Debug, Clone)]
pub struct SolarSystem {
    /// DC nameplate capacity in kWp.
    pub capacity_kw: f64,
    /// Panel tilt from horizontal, degrees.
    pub tilt_deg: f64,
    /// Panel azimuth, degrees clockwise from north (`0` = north-facing —
    /// correct for southern-hemisphere sites; `180` = south-facing).
    pub azimuth_deg: f64,
    /// Total installed quote, dollars (parts + labour, before any rebate).
    pub installed_cost_nzd: f64,
}

impl Default for SolarSystem {
    /// 5 kWp north-facing at 30°, quoted at NZ$11,500 (≈$2,300/kWp —
    /// typical New Zealand residential installed pricing).
    fn default() -> Self {
        Self {
            capacity_kw: 5.0,
            tilt_deg: 30.0,
            azimuth_deg: 0.0,
            installed_cost_nzd: 11_500.0,
        }
    }
}

impl SolarSystem {
    /// Validity errors, phrased for direct display in the UI.
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if !(0.05..=100.0).contains(&self.capacity_kw) {
            issues.push("System size must be between 0.05 and 100 kWp.".to_string());
        }
        if !(0.0..=90.0).contains(&self.tilt_deg) {
            issues.push("Tilt must be between 0 and 90 degrees.".to_string());
        }
        if !(-360.0..=360.0).contains(&self.azimuth_deg) {
            issues.push("Azimuth must be between -360 and 360 degrees.".to_string());
        }
        if !(0.0..=1_000_000.0).contains(&self.installed_cost_nzd) {
            issues.push("The installed quote must be between $0 and $1,000,000.".to_string());
        }
        issues
    }
}

/// The property's retail electricity tariff.
#[derive(Debug, Clone)]
pub struct Tariff {
    /// Import price, $/kWh (New Zealand residential is typically
    /// 0.25–0.35).
    pub import_nzd_per_kwh: f64,
    /// Feed-in price for exported solar, $/kWh (typically 0.05–0.17).
    pub feed_in_nzd_per_kwh: f64,
    /// Yearly electricity price escalation, as a fraction (0.03 = 3%/yr).
    pub annual_escalation: f64,
}

impl Default for Tariff {
    fn default() -> Self {
        Self {
            import_nzd_per_kwh: 0.30,
            feed_in_nzd_per_kwh: 0.08,
            annual_escalation: 0.03,
        }
    }
}

impl Tariff {
    /// Validity errors, phrased for direct display in the UI.
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if !(0.02..=2.0).contains(&self.import_nzd_per_kwh) {
            issues.push("Import rate must be between 2c and 200c per kWh.".to_string());
        }
        if !(0.0..=2.0).contains(&self.feed_in_nzd_per_kwh) {
            issues.push("Feed-in rate must be between 0c and 200c per kWh.".to_string());
        }
        if self.feed_in_nzd_per_kwh > self.import_nzd_per_kwh {
            issues.push(
                "Feed-in rate is higher than the import rate — check the tariff; export \
                 buy-back is always cheaper than what you pay."
                    .to_string(),
            );
        }
        if !(-0.5..=0.5).contains(&self.annual_escalation) {
            issues.push("Annual price rise must be between -50% and +50%.".to_string());
        }
        issues
    }
}

/// The household's energy consumption.
#[derive(Debug, Clone)]
pub struct Household {
    /// Annual grid consumption, kWh (New Zealand average ≈ 7,000).
    pub annual_kwh: f64,
}

impl Default for Household {
    fn default() -> Self {
        Self { annual_kwh: 7_000.0 }
    }
}

impl Household {
    /// Validity errors, phrased for direct display in the UI.
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if !(100.0..=200_000.0).contains(&self.annual_kwh) {
            issues.push("Annual consumption must be between 100 and 200,000 kWh.".to_string());
        }
        issues
    }
}

/// Where the solar energy goes over a year.
#[derive(Debug, Clone)]
pub struct EnergyBalance {
    /// The system size the balance was computed for, kWp.
    pub system_size_kw: f64,
    /// Annual PV AC production, kWh.
    pub annual_generation_kwh: f64,
    /// Annual household consumption, kWh.
    pub annual_consumption_kwh: f64,
    /// Solar consumed on-site the instant it is produced, kWh/yr — saves
    /// imported power at the full import rate.
    pub self_consumed_kwh: f64,
    /// Solar exported to the grid, kWh/yr — earns the (lower) feed-in rate.
    pub exported_kwh: f64,
    /// Grid import with no solar, kWh/yr (equals consumption).
    pub grid_import_without_solar_kwh: f64,
    /// Residual grid import with the system installed, kWh/yr.
    pub grid_import_with_solar_kwh: f64,
    /// Generation by month, kWh (January first).
    pub monthly_generation_kwh: [f64; MONTHS_PER_YEAR],
    /// Consumption by month, kWh (January first).
    pub monthly_consumption_kwh: [f64; MONTHS_PER_YEAR],
    /// Generation consumed on-site, by month, kWh (January first).
    pub monthly_self_consumed_kwh: [f64; MONTHS_PER_YEAR],
    /// Generation exported to the grid, by month, kWh (January first).
    pub monthly_exported_kwh: [f64; MONTHS_PER_YEAR],
}

impl EnergyBalance {
    /// Fraction of generation consumed on-site (the rest is exported).
    pub fn self_consumption_fraction(&self) -> f64 {
        if self.annual_generation_kwh <= 1e-9 {
            0.0
        } else {
            (self.self_consumed_kwh / self.annual_generation_kwh).clamp(0.0, 1.0)
        }
    }

    /// Generation as a fraction of consumption (how much of the household's
    /// year the solar covers in energy terms).
    pub fn solar_fraction(&self) -> f64 {
        if self.annual_consumption_kwh <= 1e-9 {
            0.0
        } else {
            (self.annual_generation_kwh / self.annual_consumption_kwh).clamp(0.0, 1.0)
        }
    }
}

/// Money results for a cash purchase, over the [`LIFETIME_YEARS`] horizon.
#[derive(Debug, Clone)]
pub struct Financials {
    /// Year-1 bill savings: self-consumed energy at the import rate plus
    /// exported energy at the feed-in rate, $/yr.
    pub year1_savings_nzd: f64,
    /// Savings in analysis year t (0-based; year 0 = the first full year of
    /// operation), with price escalation applied and generation degraded.
    pub annual_savings: Vec<f64>,
    /// Cumulative net cash position at the *start* of each year, index 0 =
    /// `-cost` (the purchase), through index `LIFETIME_YEARS`.
    pub cumulative_net_cash: Vec<f64>,
    /// Years until cumulative savings reach the installed cost,
    /// interpolated within the break-even year. `None` if the system never
    /// pays back within the horizon.
    pub payback_years: Option<f64>,
    /// Total undiscounted savings minus the installed cost, over the
    /// horizon.
    pub lifetime_net_savings_nzd: f64,
    /// Net present value of [-cost, savings...] at the given discount rate.
    pub npv_nzd: f64,
    /// Internal rate of return of the same cash flows, when bracketable.
    pub irr: Option<f64>,
}

/// One complete single-property estimate: the physical balance and its
/// cash-purchase financials.
#[derive(Debug, Clone)]
pub struct PaybackReport {
    /// Where the energy goes.
    pub balance: EnergyBalance,
    /// What it is worth.
    pub financials: Financials,
}

/// Hourly PV AC output (kW) for the representative day of `month`
/// (0 = January) under the month's cloud and temperature conditions.
fn solar_profile_kw(
    site: &Site,
    capacity_kw: f64,
    tilt_deg: f64,
    azimuth_deg: f64,
    month: usize,
) -> Vec<f64> {
    let solar_model = SolarModel::new(
        site.latitude_deg,
        site.longitude_deg,
        site.altitude_m,
        site.timezone_offset_hours,
    );
    let mut config = PvPlantConfig::new(
        capacity_kw / 1000.0,
        tilt_deg,
        azimuth_deg,
        MONTH_AMBIENT_C[month % MONTHS_PER_YEAR],
    );
    config.soiling_loss = SOILING_LOSS;
    let plant = PvPlant::new(solar_model.clone(), config);
    let cloud = MONTH_CLOUD[month % MONTHS_PER_YEAR];
    let base_utc = month_base_utc(site, month);

    (0..HOURS_PER_DAY)
        .map(|h| {
            // Hour `h` covers [h, h+1): sample the sun at the midpoint.
            let pos = solar_model.solar_position(base_utc + Duration::minutes(h as i64 * 60 + 30));
            let clear = plant.output_clearsky(&pos);
            // Clear-sky POA (undo soiling), scale by cloud, re-run the
            // public output path (re-applies soiling + thermal + clipping),
            // then the balance-of-system derate.
            let poa_clear = clear.poa_w_per_m2 / (1.0 - SOILING_LOSS);
            plant.output_from_poa(poa_clear * cloud).ac_power_mw * 1000.0
                * (1.0 - OTHER_SYSTEM_LOSSES)
        })
        .collect()
}

/// Local midnight of the representative day of `month`, as a UTC instant.
fn month_base_utc(site: &Site, month: usize) -> chrono::DateTime<Utc> {
    let month1 = (month % MONTHS_PER_YEAR) as u32 + 1;
    let date = NaiveDate::from_ymd_opt(REPRESENTATIVE_YEAR, month1, REPRESENTATIVE_DAY)
        .expect("representative date is valid");
    let local_midnight = date.and_hms_opt(0, 0, 0).expect("midnight is valid");
    let offset_secs = (site.timezone_offset_hours * 3600.0).round() as i64;
    Utc.from_utc_datetime(&(local_midnight - Duration::seconds(offset_secs)))
}

/// Nets hourly generation against hourly consumption for the whole year:
/// solar serves the load as it happens, the surplus is exported, and the
/// shortfall stays on the grid.
///
/// The battery-less model is the honest default for payback: New Zealand
/// feed-in rates are low enough that self-consumed energy is worth roughly
/// 3–4× exported energy, and storage changes the split, not the physics.
pub fn energy_balance(site: &Site, system: &SolarSystem, household: &Household) -> EnergyBalance {
    let monthly_load = monthly_consumption_kwh(household.annual_kwh);
    let shape = hourly_load_shape();

    let mut monthly_generation = [0.0; MONTHS_PER_YEAR];
    let mut monthly_self_consumed = [0.0; MONTHS_PER_YEAR];
    let mut monthly_exported = [0.0; MONTHS_PER_YEAR];
    let mut self_consumed = 0.0;
    let mut exported = 0.0;

    for m in 0..MONTHS_PER_YEAR {
        let solar = solar_profile_kw(
            site,
            system.capacity_kw,
            system.tilt_deg,
            system.azimuth_deg,
            m,
        );
        let days = MONTH_DAYS[m] as f64;
        for h in 0..HOURS_PER_DAY {
            // This hour's energy across every repetition of the
            // representative day in the month.
            let generation = solar[h].max(0.0) * days;
            let load = monthly_load[m] * shape[h];
            monthly_generation[m] += generation;
            monthly_self_consumed[m] += generation.min(load);
            monthly_exported[m] += (generation - load).max(0.0);
        }
        self_consumed += monthly_self_consumed[m];
        exported += monthly_exported[m];
    }

    let generation: f64 = monthly_generation.iter().sum();
    let consumption: f64 = monthly_load.iter().sum();
    EnergyBalance {
        system_size_kw: system.capacity_kw,
        annual_generation_kwh: generation,
        annual_consumption_kwh: consumption,
        self_consumed_kwh: self_consumed,
        exported_kwh: exported,
        grid_import_without_solar_kwh: consumption,
        grid_import_with_solar_kwh: (consumption - self_consumed).max(0.0),
        monthly_generation_kwh: monthly_generation,
        monthly_consumption_kwh: monthly_load,
        monthly_self_consumed_kwh: monthly_self_consumed,
        monthly_exported_kwh: monthly_exported,
    }
}

/// Cash-purchase financials over the [`LIFETIME_YEARS`] horizon:
/// escalating import prices and degraded generation, break-even search,
/// NPV, and IRR.
pub fn financials(
    balance: &EnergyBalance,
    installed_cost_nzd: f64,
    tariff: &Tariff,
    discount_rate: f64,
) -> Financials {
    let year1_savings = balance.self_consumed_kwh * tariff.import_nzd_per_kwh
        + balance.exported_kwh * tariff.feed_in_nzd_per_kwh;

    let annual_savings: Vec<f64> = (0..LIFETIME_YEARS)
        .map(|t| {
            year1_savings
                * (1.0 + tariff.annual_escalation).powi(t as i32)
                * (1.0 - PANEL_DEGRADATION_PER_YEAR).powi(t as i32)
        })
        .collect();

    // Cumulative cash at the start of each year: the purchase, then each
    // year's savings on top.
    let mut cumulative_net_cash = Vec::with_capacity(LIFETIME_YEARS as usize + 1);
    cumulative_net_cash.push(-installed_cost_nzd);
    for savings in &annual_savings {
        let last = *cumulative_net_cash.last().expect("seeded above");
        cumulative_net_cash.push(last + savings);
    }

    // Break-even: first year the cumulative position crosses zero,
    // interpolated within that year.
    let payback_years = if installed_cost_nzd <= 0.0 {
        Some(0.0)
    } else {
        (1..=LIFETIME_YEARS as usize).find_map(|k| {
            let before = cumulative_net_cash[k - 1];
            let after = cumulative_net_cash[k];
            if before < 0.0 && after >= 0.0 {
                Some((k as f64 - 1.0) + (-before / (after - before)))
            } else {
                None
            }
        })
    };

    let cash_flows: Vec<f64> = std::iter::once(-installed_cost_nzd)
        .chain(annual_savings.iter().copied())
        .collect();
    let lifetime_net_savings_nzd = annual_savings.iter().sum::<f64>() - installed_cost_nzd;

    Financials {
        year1_savings_nzd: year1_savings,
        annual_savings,
        cumulative_net_cash,
        payback_years,
        lifetime_net_savings_nzd,
        npv_nzd: tpt_nrg_lcoe::net_present_value(&cash_flows, discount_rate),
        irr: tpt_nrg_lcoe::internal_rate_of_return(&cash_flows),
    }
}

/// Runs the full single-property estimate: energy balance, then
/// cash-purchase financials.
pub fn estimate(
    site: &Site,
    system: &SolarSystem,
    tariff: &Tariff,
    household: &Household,
    discount_rate: f64,
) -> PaybackReport {
    let balance = energy_balance(site, system, household);
    let financials = financials(&balance, system.installed_cost_nzd, tariff, discount_rate);
    PaybackReport {
        balance,
        financials,
    }
}

/// Loan terms for the financed-purchase comparison (Pro).
#[derive(Debug, Clone)]
pub struct LoanTerms {
    /// Paid up front, dollars; the rest of the quote is borrowed.
    pub deposit_nzd: f64,
    /// Nominal annual interest rate, as a fraction (0.109 = 10.9%).
    pub annual_interest_rate: f64,
    /// Loan length in whole years; payments are monthly.
    pub term_years: u32,
}

impl Default for LoanTerms {
    fn default() -> Self {
        Self {
            deposit_nzd: 0.0,
            annual_interest_rate: 0.109,
            term_years: 7,
        }
    }
}

impl LoanTerms {
    /// Validity errors, phrased for direct display in the UI.
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if !(0.0..=1_000_000.0).contains(&self.deposit_nzd) {
            issues.push("The deposit must be between $0 and $1,000,000.".to_string());
        }
        if !(0.0..=0.5).contains(&self.annual_interest_rate) {
            issues.push("Interest rate must be between 0% and 50%.".to_string());
        }
        if !(1..=30).contains(&self.term_years) {
            issues.push("Loan term must be between 1 and 30 years.".to_string());
        }
        issues
    }
}

/// Financed-purchase results (Pro): the annuity, what the loan really
/// costs, and the break-even on the out-of-pocket deposit.
#[derive(Debug, Clone)]
pub struct LoanResult {
    /// Amount borrowed (quote minus deposit). `0` when the deposit covers
    /// the whole quote — every other field is then zero.
    pub loan_amount_nzd: f64,
    /// Fixed monthly payment over the term (standard annuity).
    pub monthly_payment_nzd: f64,
    /// `monthly_payment_nzd * 12`.
    pub annual_payment_nzd: f64,
    /// All payments over the term, dollars.
    pub total_paid_nzd: f64,
    /// Interest component of [`Self::total_paid_nzd`].
    pub total_interest_nzd: f64,
    /// Years until cumulative (savings − loan payments) cover the deposit,
    /// interpolated. `None` if the financed position never breaks even
    /// within the horizon.
    pub payback_years: Option<f64>,
    /// Cumulative net cash position at the start of each year with the
    /// loan in place, index 0 = `-deposit`, through `LIFETIME_YEARS`.
    pub cumulative_net_cash: Vec<f64>,
    /// Total undiscounted savings minus loan payments and deposit, over the
    /// horizon.
    pub lifetime_net_savings_nzd: f64,
}

/// Standard annuity payment: `P·i / (1 − (1+i)^(−n))` with `i` the monthly
/// rate and `n` the number of months.
pub fn monthly_payment(principal_nzd: f64, annual_interest_rate: f64, term_years: u32) -> f64 {
    let months = (term_years.max(1)) as f64 * 12.0;
    let monthly_rate = annual_interest_rate / 12.0;
    if principal_nzd <= 0.0 {
        return 0.0;
    }
    if monthly_rate <= 0.0 {
        return principal_nzd / months;
    }
    principal_nzd * monthly_rate / (1.0 - (1.0 + monthly_rate).powf(-months))
}

/// Compares a financed purchase against the cash savings stream from a
/// [`PaybackReport`] (Pro). The borrower pays the deposit up front and the
/// annuity monthly; savings offset both.
pub fn analyze_financing(
    report: &PaybackReport,
    installed_cost_nzd: f64,
    terms: &LoanTerms,
) -> LoanResult {
    let loan_amount = (installed_cost_nzd - terms.deposit_nzd).max(0.0);
    let monthly = monthly_payment(loan_amount, terms.annual_interest_rate, terms.term_years);
    let annual_payment = monthly * 12.0;

    // Net cash each year: savings minus the annuity while the loan runs.
    let annual_net: Vec<f64> = report
        .financials
        .annual_savings
        .iter()
        .enumerate()
        .map(|(i, savings)| {
            if (i as u32) < terms.term_years {
                savings - annual_payment
            } else {
                *savings
            }
        })
        .collect();

    let mut cumulative_net_cash = Vec::with_capacity(LIFETIME_YEARS as usize + 1);
    cumulative_net_cash.push(-terms.deposit_nzd);
    for net in &annual_net {
        let last = *cumulative_net_cash.last().expect("seeded above");
        cumulative_net_cash.push(last + net);
    }

    let payback_years = if terms.deposit_nzd <= 0.0 {
        // Nothing out of pocket: the financed position starts at zero, so
        // the meaningful break-even is the same cumulative-savings-vs-cost
        // crossing as the cash purchase (the loan payments ARE the cost,
        // paid over time).
        report.financials.payback_years
    } else {
        (1..=LIFETIME_YEARS as usize).find_map(|k| {
            let before = cumulative_net_cash[k - 1];
            let after = cumulative_net_cash[k];
            if before < 0.0 && after >= 0.0 {
                Some((k as f64 - 1.0) + (-before / (after - before)))
            } else {
                None
            }
        })
    };

    let term_payments = annual_payment * terms.term_years as f64;
    LoanResult {
        loan_amount_nzd: loan_amount,
        monthly_payment_nzd: monthly,
        annual_payment_nzd: annual_payment,
        total_paid_nzd: term_payments,
        total_interest_nzd: (term_payments - loan_amount).max(0.0),
        payback_years,
        lifetime_net_savings_nzd: annual_net.iter().sum::<f64>() - terms.deposit_nzd,
        cumulative_net_cash,
    }
}

/// One alternative quote in the multi-scenario comparison (Pro).
#[derive(Debug, Clone)]
pub struct Scenario {
    /// Display name, e.g. `"5 kW — Acme Solar"`.
    pub name: String,
    /// DC nameplate capacity in kWp.
    pub capacity_kw: f64,
    /// Total installed quote, dollars.
    pub installed_cost_nzd: f64,
}

/// A scenario paired with its full estimate (Pro).
#[derive(Debug, Clone)]
pub struct ScenarioOutcome {
    /// The scenario as configured.
    pub scenario: Scenario,
    /// The single-property estimate for it.
    pub report: PaybackReport,
}

/// Runs the same site/tariff/household through every scenario (Pro). Each
/// scenario may differ in system size, price, or both; orientation and tilt
/// are shared — vary them by running separate estimates.
pub fn compare_scenarios(
    site: &Site,
    tariff: &Tariff,
    household: &Household,
    discount_rate: f64,
    scenarios: &[Scenario],
) -> Vec<ScenarioOutcome> {
    scenarios
        .iter()
        .map(|scenario| {
            let system = SolarSystem {
                capacity_kw: scenario.capacity_kw,
                tilt_deg: SolarSystem::default().tilt_deg,
                azimuth_deg: SolarSystem::default().azimuth_deg,
                installed_cost_nzd: scenario.installed_cost_nzd,
            };
            ScenarioOutcome {
                scenario: scenario.clone(),
                report: estimate(site, &system, tariff, household, discount_rate),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hourly_load_shape_sums_to_one() {
        let total: f64 = hourly_load_shape().iter().sum();
        assert!((total - 1.0).abs() < 1e-9);
        // The evening peak is the day's maximum.
        let shape = hourly_load_shape();
        let peak = shape
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .expect("non-empty");
        assert_eq!(peak.0, 19);
    }

    #[test]
    fn monthly_consumption_sums_to_annual() {
        let months = monthly_consumption_kwh(7_000.0);
        let total: f64 = months.iter().sum();
        assert!((total - 7_000.0).abs() < 1e-6);
        // Winter-peaked: July beats January.
        assert!(months[6] > months[0]);
    }

    #[test]
    fn month_days_cover_a_non_leap_year() {
        assert_eq!(MONTH_DAYS.iter().sum::<u64>(), 365);
    }
}
