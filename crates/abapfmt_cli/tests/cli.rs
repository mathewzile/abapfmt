use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_abapfmt"))
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("abapfmt-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const UNFORMATTED: &str = "DATA   lv_a   TYPE i.\n";
const FORMATTED: &str = "DATA lv_a TYPE i.\n";

#[test]
fn stdin_is_formatted() {
    let mut child = bin()
        .arg("--stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(UNFORMATTED.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap(), FORMATTED);
}

#[test]
fn check_and_write() {
    let dir = temp_dir("check");
    let file = dir.join("a.abap");
    std::fs::write(&file, UNFORMATTED).unwrap();

    let out = bin().arg("--check").arg(&dir).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("a.abap"));

    assert!(bin().arg("--write").arg(&file).status().unwrap().success());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), FORMATTED);
    assert!(bin().arg("--check").arg(&dir).status().unwrap().success());
}

#[test]
fn nearest_config_switches_rules_off() {
    let dir = temp_dir("config");
    std::fs::write(
        dir.join("abapfmt.json"),
        r#"{"rules":{"ALIGN_DECLARATIONS":false}}"#,
    )
    .unwrap();
    let file = dir.join("a.abap");
    std::fs::write(&file, UNFORMATTED).unwrap();
    let out = bin().arg(&file).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap(), UNFORMATTED);
    // --no-config ignores it
    let out = bin().arg("--no-config").arg(&file).output().unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), FORMATTED);
}

#[test]
fn invalid_config_is_an_error() {
    let dir = temp_dir("bad");
    std::fs::write(dir.join("abapfmt.json"), r#"{"rules":{"NOPE":{}}}"#).unwrap();
    let file = dir.join("a.abap");
    std::fs::write(&file, UNFORMATTED).unwrap();
    let out = bin().arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown rule"));
}
