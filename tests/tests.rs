//! Integration tests for our CLI.

extern crate cli_test_dir;

use cli_test_dir::*;

#[test]
fn help_flag() {
    let testdir = TestDir::new("scrubcsv", "flag_help");
    let output = testdir.cmd()
        .arg("--help")
        .expect_success();
    assert!(output.stdout_str().contains("scrubcsv --help"));
}

#[test]
fn version_flag() {
    let testdir = TestDir::new("scrubcsv", "flag_version");
    let output = testdir.cmd()
        .arg("--version")
        .expect_success();
    assert!(output.stdout_str().contains("scrubcsv "));
}

#[test]
fn basic_file_scrubbing() {
    let testdir = TestDir::new("scrubcsv", "basic_scrubbing");
    testdir.create_file("in.csv", "\
a,b,c
1,\"2\",3
\"Paris, France\",\"Broken \" quotes\",
");
    let output = testdir.cmd()
        .arg("in.csv")
        .expect_success();
    // We reserve the right to change the exact output we generate for "Broken
    // \" quotes".  We could do a better job of guessing here.
    assert_eq!(output.stdout_str(), "\
a,b,c
1,2,3
\"Paris, France\",\"Broken  quotes\"\"\",
");
    assert!(output.stderr_str().contains("3 rows (0 bad)"));
}

#[test]
fn stdin_and_delimiter_and_quiet() {
    let testdir = TestDir::new("scrubcsv", "stdin_and_delimiter_and_quiet");
    let output = testdir.cmd()
        .args(&["-d", "|"])
        .arg("-q")
        .output_with_stdin("\
a|b|c
1|2|3
")
        .expect_success();
    assert_eq!(output.stdout_str(), "\
a,b,c
1,2,3
");
    assert!(!output.stderr_str().contains("rows"));
}

#[test]
fn bad_rows() {
    // Create a file with lots of good rows--enough to avoid triggering the
    // "too many bad rows" detection. This is an inefficient use of
    // `put_str`, but it doesn't matter for a test.
    let mut good_rows = "a,b,c\n".to_owned();
    for _ in 0..100 {
        good_rows.push_str("1,2,3\n");
    }
    let mut bad_rows = good_rows.clone();
    bad_rows.push_str("1,2\n");

    let testdir = TestDir::new("scrubcsv", "bad_rows");
    let output = testdir.cmd()
        .output_with_stdin(&bad_rows)
        .expect_success();
    assert_eq!(output.stdout_str(), &good_rows);
    assert!(output.stderr_str().contains("102 rows (1 bad)"));
}

#[test]
fn too_many_bad_rows() {
    let testdir = TestDir::new("scrubcsv", "too_many_bad_rows");
    let output = testdir.cmd()
        .output_with_stdin("\
a,b,c
1,2
")
        .expect("could not run scrubcsv");
    assert!(!output.status.success());
    assert_eq!(output.stdout_str(), "a,b,c\n");
    assert!(output.stderr_str().contains("Too many rows (1 of 2) were bad"));
}
