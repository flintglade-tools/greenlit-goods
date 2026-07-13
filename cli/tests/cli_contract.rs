use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn greenlit() -> Command {
    Command::new(env!("CARGO_BIN_EXE_greenlit"))
}

fn sample(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("samples")
        .join(name)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn run(command: &mut Command) -> Output {
    command.output().expect("greenlit process should start")
}

#[test]
fn audit_success_and_strict_failure_have_stable_exit_codes() {
    let success = run(greenlit().args(["audit", sample("clean.xml").to_str().unwrap(), "--json"]));
    assert_eq!(success.status.code(), Some(0), "{}", text(&success.stderr));
    assert!(text(&success.stdout).contains("\"greenlight_score\""));
    assert!(text(&success.stdout).contains("\"schema_version\": 1"));

    let failure = run(greenlit().args([
        "audit",
        sample("broken.xml").to_str().unwrap(),
        "--strict",
        "--no-color",
    ]));
    assert_eq!(failure.status.code(), Some(1), "{}", text(&failure.stderr));
}

#[test]
fn invalid_cli_value_is_usage_error() {
    let output = run(greenlit().args([
        "audit",
        sample("clean.xml").to_str().unwrap(),
        "--assumed-monthly-sales",
        "NaN",
    ]));
    assert_eq!(output.status.code(), Some(2));
    assert!(text(&output.stderr).contains("finite and non-negative"));
}

#[test]
fn fix_refuses_to_replace_an_existing_output() {
    let dir = tempfile::tempdir().unwrap();
    let output_path = dir.path().join("existing.xml");
    fs::write(&output_path, b"keep me").unwrap();

    let output = run(greenlit().args([
        "fix",
        sample("broken.xml").to_str().unwrap(),
        "--output",
        output_path.to_str().unwrap(),
    ]));

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read(&output_path).unwrap(), b"keep me");
    assert!(text(&output.stderr).contains("refusing to overwrite"));
}

#[test]
fn fix_refuses_lossy_xml_and_creates_no_output() {
    let dir = tempfile::tempdir().unwrap();
    let feed_path = dir.path().join("nested.xml");
    let output_path = dir.path().join("fixed.xml");
    fs::write(
        &feed_path,
        br#"<rss xmlns:g="http://base.google.com/ns/1.0"><channel><item><g:id>1</g:id><g:title><b>Nested</b></g:title></item></channel></rss>"#,
    )
    .unwrap();

    let output = run(greenlit().args([
        "fix",
        feed_path.to_str().unwrap(),
        "--output",
        output_path.to_str().unwrap(),
    ]));

    assert_eq!(output.status.code(), Some(2));
    assert!(!output_path.exists());
    assert!(
        text(&output.stderr).contains("refusing to rewrite this feed safely"),
        "{}",
        text(&output.stderr)
    );
}
