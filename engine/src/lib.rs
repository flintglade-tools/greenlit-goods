//! # Greenlit Goods — engine
//!
//! The diagnosis-and-fix core for Google Merchant Center product feeds. Pure
//! library: no network, no disk, no global state. You hand it bytes and options;
//! it hands you a structured [`report::Report`] or a corrected feed plus a diff.
//!
//! Pipeline: **parse → contextualize → run rules → score → (optionally) fix**.
//!
//! ```no_run
//! use greenlit_engine::{audit_auto, AuditOptions};
//! let bytes = std::fs::read("feed.xml").unwrap();
//! let report = audit_auto(&bytes, &AuditOptions::default()).unwrap();
//! if let Some(score) = report.greenlight_score {
//!     println!("Greenlight Score: {score}/100");
//! }
//! ```

pub mod error;
pub mod finding;
pub mod fix;
pub mod model;
pub mod parse;
pub mod report;
pub mod rules;
pub mod score;
pub mod serialize;
pub mod text;

pub use error::{EngineError, Result};
pub use finding::{Finding, FindingBasis, Severity};
pub use fix::FixRecord;
pub use model::{Feed, Format, Product};
pub use report::{AuditOptions, Destination, Report};
pub use rules::FeedContext;

const ISO_3166_ALPHA_2: &str = "AD AE AF AG AI AL AM AO AQ AR AS AT AU AW AX AZ BA BB BD BE BF BG BH BI BJ BL BM BN BO BQ BR BS BT BV BW BY BZ CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CW CX CY CZ DE DJ DK DM DO DZ EC EE EG EH ER ES ET FI FJ FK FM FO FR GA GB GD GE GF GG GH GI GL GM GN GP GQ GR GS GT GU GW GY HK HM HN HR HT HU ID IE IL IM IN IO IQ IR IS IT JE JM JO JP KE KG KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY MA MC MD ME MF MG MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ NA NC NE NF NG NI NL NO NP NR NU NZ OM PA PE PF PG PH PK PL PM PN PR PS PT PW PY QA RE RO RS RU RW SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR SS ST SV SX SY SZ TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ UA UG UM US UY UZ VA VC VE VG VI VN VU WF WS YE YT ZA ZM ZW";

/// Largest sales assumption that keeps the worst-case revenue sum and its
/// two-decimal rounding finite at the documented product and price limits.
pub const MAX_ASSUMED_MONTHLY_SALES: f64 =
    f64::MAX / (i64::MAX as f64) / (parse::MAX_PRODUCTS as f64) / 4.0;

/// Options for [`fix`] / [`fix_auto`]: the same audit options are used to score
/// the before/after reports.
#[derive(Debug, Clone, Default)]
pub struct FixOptions {
    pub audit: AuditOptions,
}

/// The result of a fix run: the corrected feed bytes, the change log, and the
/// before/after reports so a caller can show the improvement.
#[derive(Debug, Clone)]
pub struct FixResult {
    pub corrected_feed: Vec<u8>,
    pub log: Vec<FixRecord>,
    pub before: Report,
    pub after: Report,
}

/// Audit a feed of a known format.
pub fn audit(bytes: &[u8], format: Format, opts: &AuditOptions) -> Result<Report> {
    validate_audit_options(opts)?;
    let feed = parse::parse(bytes, format)?;
    let ctx = FeedContext::build(&feed, &opts.target_country, opts.destination);
    Ok(report::build_report(&feed, &ctx, opts))
}

/// Audit a feed, auto-detecting the format.
pub fn audit_auto(bytes: &[u8], opts: &AuditOptions) -> Result<Report> {
    let format = parse::detect_format(bytes).ok_or(EngineError::UnknownFormat)?;
    audit(bytes, format, opts)
}

/// Apply safe automatic fixes to a feed of a known format and re-audit.
pub fn fix(bytes: &[u8], format: Format, opts: &FixOptions) -> Result<FixResult> {
    validate_audit_options(&opts.audit)?;
    let feed = parse::parse(bytes, format)?;
    serialize::ensure_rewrite_safe(&feed)?;
    let ctx_before = FeedContext::build(&feed, &opts.audit.target_country, opts.audit.destination);
    let before = report::build_report(&feed, &ctx_before, &opts.audit);

    let (corrected, log) = fix::apply_fixes(&feed);
    let ctx_after = FeedContext::build(
        &corrected,
        &opts.audit.target_country,
        opts.audit.destination,
    );
    let after = report::build_report(&corrected, &ctx_after, &opts.audit);

    let corrected_feed = serialize::serialize(&corrected)?;
    Ok(FixResult {
        corrected_feed,
        log,
        before,
        after,
    })
}

fn validate_audit_options(opts: &AuditOptions) -> Result<()> {
    if !opts.assumed_monthly_sales.is_finite() || opts.assumed_monthly_sales < 0.0 {
        return Err(EngineError::InvalidOptions(
            "assumed_monthly_sales must be finite and non-negative".into(),
        ));
    }
    if opts.assumed_monthly_sales > MAX_ASSUMED_MONTHLY_SALES {
        return Err(EngineError::InvalidOptions(
            "assumed_monthly_sales is too large to keep revenue calculations finite".into(),
        ));
    }
    let country = opts.target_country.to_ascii_uppercase();
    if !ISO_3166_ALPHA_2
        .split_ascii_whitespace()
        .any(|code| code == country)
    {
        return Err(EngineError::InvalidOptions(
            "target_country must be a current ISO 3166-1 alpha-2 code".into(),
        ));
    }
    Ok(())
}

/// Apply safe automatic fixes, auto-detecting the format.
pub fn fix_auto(bytes: &[u8], opts: &FixOptions) -> Result<FixResult> {
    let format = parse::detect_format(bytes).ok_or(EngineError::UnknownFormat)?;
    fix(bytes, format, opts)
}
