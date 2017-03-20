//! Integration tests for our CLI.

extern crate cli_test_dir;

use cli_test_dir::TestDir;
use std::str::from_utf8;

#[test]
fn flag_help() {
    let testdir = TestDir::new("scrubcsv", "flag_help");
    let output = testdir.cmd()
        .arg("--help")
        .output()
        .expect("could not run scrubcsv");
    assert!(output.status.success());
    assert!(from_utf8(&output.stdout).unwrap().find("scrubcsv --help").is_some());
}

#[test]
fn flag_version() {
    let testdir = TestDir::new("scrubcsv", "flag_version");
    let output = testdir.cmd()
        .arg("--version")
        .output()
        .expect("could not run scrubcsv");
    assert!(output.status.success());
    assert!(from_utf8(&output.stdout).unwrap().find("scrubcsv ").is_some());
}
