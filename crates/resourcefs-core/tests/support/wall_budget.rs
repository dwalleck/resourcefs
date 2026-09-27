//! Applies the wall-clock budget policy for a test target (rfs-cn1r).
//!
//! The policy itself is `resourcefs_core::test_support::wall_budget`; this
//! module adds the side effects a test needs: failing, printing, and
//! appending each measurement to the file named by `RFS_BUDGET_REPORT`. It
//! lives here rather than in core's `test_support` because appending to a
//! file is platform I/O, which `core_holds_no_platform_io` forbids in
//! `resourcefs-core/src`. Test targets include it with `#[path]`.
//!
//! The report file exists because the harnesses discard a passing test's
//! output: nextest keeps only failures, and the release leg runs libtest
//! captured. `scripts/ci-gates.py` points the variable at a per-phase file
//! and prints the collected lines, over-budget first, when the phase ends.
#![allow(dead_code, reason = "each test target uses a subset")]

use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

pub use resourcefs_core::test_support::wall_budget::HardCeiling;
use resourcefs_core::test_support::wall_budget::{
    BUDGET_REPORT_VARIABLE, Bound, BudgetPolicy, judge,
};

/// Checks `elapsed <= budget` under the environment's policy.
///
/// The measurement is named after the running test, taken from the test
/// thread's name, which libtest and nextest both set to the test's path; call
/// this on that thread, not from a spawned one. `label` is free text that
/// says what was measured, typically the plan criterion and subject the old
/// assertion message carried (`"[C4] 10,000 journal transitions"`), or `""`.
/// The line reads `budget <test>[ <label>]: <elapsed> against <budget> ...`.
///
/// # Panics
///
/// Panics when the policy fails the measurement, when either variable is
/// invalid, when called off a named test thread, or when the report file
/// cannot be written.
pub fn check_wall_budget(label: &str, elapsed: Duration, budget: Duration) {
    apply(Bound::AtMost, HardCeiling::DEFAULT, label, elapsed, budget);
}

/// [`check_wall_budget`] with this row's own report-mode hard ceiling, for a
/// row where the default of 10x its budget is wrong. Say why at the call site.
///
/// # Panics
///
/// As [`check_wall_budget`], and when `hard` is below `budget`.
pub fn check_wall_budget_capped(
    label: &str,
    elapsed: Duration,
    budget: Duration,
    hard: HardCeiling,
) {
    apply(Bound::AtMost, hard, label, elapsed, budget);
}

/// Checks `elapsed < budget` under the environment's policy, for a ceiling
/// that was a strict comparison before it was routed through the policy.
///
/// # Panics
///
/// As [`check_wall_budget`].
pub fn check_wall_budget_below(label: &str, elapsed: Duration, budget: Duration) {
    apply(Bound::Below, HardCeiling::DEFAULT, label, elapsed, budget);
}

/// The running test's name, with `label` appended when there is one.
///
/// # Panics
///
/// Panics on an unnamed thread: the harness names each test's thread, so an
/// unnamed one means the call moved off it and the line would not say which
/// test it measured.
fn measurement_name(label: &str) -> String {
    let current = std::thread::current();
    let test = current.name().unwrap_or_else(|| {
        panic!("check_wall_budget must run on the test's own thread, which the harness names")
    });
    if label.is_empty() {
        test.to_owned()
    } else {
        format!("{test} {label}")
    }
}

fn apply(bound: Bound, hard: HardCeiling, label: &str, elapsed: Duration, budget: Duration) {
    let policy = BudgetPolicy::from_environment();
    let judgement = judge(
        policy,
        bound,
        hard,
        &measurement_name(label),
        elapsed,
        budget,
    );
    if let Some(path) = report_path() {
        append_report_line(path, judgement.line());
    }
    if policy == BudgetPolicy::Report {
        eprintln!("{}", judgement.line());
    }
    if let Some(failure) = judgement.failure() {
        panic!("{failure}");
    }
}

/// The report file named by `RFS_BUDGET_REPORT`, read once per process.
fn report_path() -> Option<&'static Path> {
    static REPORT: OnceLock<Option<PathBuf>> = OnceLock::new();
    REPORT
        .get_or_init(|| {
            let value = std::env::var_os(BUDGET_REPORT_VARIABLE)?;
            assert!(
                !value.is_empty(),
                "{BUDGET_REPORT_VARIABLE} is set but empty; name a file or unset it"
            );
            Some(PathBuf::from(value))
        })
        .as_deref()
}

/// Appends one line to `path` with a single write on an append-mode handle,
/// so lines from concurrent test processes do not interleave.
///
/// # Panics
///
/// Panics when the file cannot be opened or written: the gate runner asked
/// for the record, and losing it silently would hide the numbers the report
/// exists to show.
pub fn append_report_line(path: &Path, line: &str) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap_or_else(|error| {
            panic!(
                "{BUDGET_REPORT_VARIABLE}: cannot open {}: {error}",
                path.display()
            )
        });
    file.write_all(format!("{line}\n").as_bytes())
        .unwrap_or_else(|error| {
            panic!(
                "{BUDGET_REPORT_VARIABLE}: cannot append to {}: {error}",
                path.display()
            )
        });
}
