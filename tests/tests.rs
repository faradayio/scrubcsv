//! Integration tests for our CLI.

extern crate cli_test_dir;

use cli_test_dir::*;

#[test]
fn help_flag() {
    let testdir = TestDir::new("scrubcsv", "flag_help");
    let output = testdir.cmd().arg("--help").expect_success();
    assert!(output.stdout_str().contains("scrubcsv"));
    assert!(output.stdout_str().contains("--help"));
}

#[test]
fn version_flag() {
    let testdir = TestDir::new("scrubcsv", "flag_version");
    let output = testdir.cmd().arg("--version").expect_success();
    assert!(output.stdout_str().contains("scrubcsv "));
}

#[test]
fn basic_file_scrubbing() {
    let testdir = TestDir::new("scrubcsv", "basic_scrubbing");
    testdir.create_file(
        "in.csv",
        "\
a,b,c
1,\"2\",3
\"Paris, France\",\"Broken \" quotes\",
",
    );
    let output = testdir.cmd().arg("in.csv").expect_success();
    // We reserve the right to change the exact output we generate for "Broken
    // \" quotes".  We could do a better job of guessing here.
    assert_eq!(
        output.stdout_str(),
        "\
a,b,c
1,2,3
\"Paris, France\",\"Broken  quotes\"\"\",
"
    );
    assert!(output.stderr_str().contains("3 rows (0 bad)"));
}

#[test]
fn stdin_and_delimiter_and_quiet() {
    let testdir = TestDir::new("scrubcsv", "stdin_and_delimiter_and_quiet");
    let output = testdir
        .cmd()
        .args(&["-d", "|"])
        .arg("-q")
        .output_with_stdin(
            "\
a|b|c
1|2|3
",
        )
        .expect_success();
    assert_eq!(
        output.stdout_str(),
        "\
a,b,c
1,2,3
"
    );
    assert!(!output.stderr_str().contains("rows"));
}

#[test]
fn quote_and_delimiter() {
    let testdir = TestDir::new("scrubcsv", "basic_scrubbing");
    testdir.create_file(
        "in.csv",
        "\
a\tb\tc
1\t\"2\t3
",
    );
    let output = testdir
        .cmd()
        .args(&["-d", r"\t"])
        .args(&["--quote", "none"])
        .arg("in.csv")
        .expect_success();
    assert_eq!(
        output.stdout_str(),
        "\
a,b,c
1,\"\"\"2\",3
"
    );
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
    let output = testdir.cmd().output_with_stdin(&bad_rows).expect_success();
    assert_eq!(output.stdout_str(), &good_rows);
    assert!(output.stderr_str().contains("102 rows (1 bad)"));
}

#[test]
fn bad_rows_saved() {
    let mut good_rows = "a,b,c\n".to_owned();
    for _ in 0..100 {
        good_rows.push_str("1,2,3\n");
    }
    let mut bad_rows = good_rows.clone();
    bad_rows.push_str("1,2\n");

    let testdir = TestDir::new("scrubcsv", "bad_rows_saved");
    let output = testdir
        .cmd()
        .args(&["--bad-rows-path", "bad.csv"])
        .output_with_stdin(&bad_rows)
        .expect_success();
    testdir.expect_file_contents("bad.csv", "1,2\n");
    assert_eq!(output.stdout_str(), &good_rows);
    assert!(output.stderr_str().contains("102 rows (1 bad)"));
}

#[test]
fn too_many_bad_rows() {
    let testdir = TestDir::new("scrubcsv", "too_many_bad_rows");
    let output = testdir
        .cmd()
        .output_with_stdin(
            "\
a,b,c
1,2
",
        )
        .expect("could not run scrubcsv");
    assert!(!output.status.success());
    assert_eq!(output.stdout_str(), "a,b,c\n");
    assert!(output
        .stderr_str()
        .contains("Too many rows (1 of 2) were bad"));
}

#[test]
fn null_normalization() {
    let testdir = TestDir::new("scrubcsv", "null_normalization");
    let output = testdir
        .cmd()
        .args(&["--null", "(?i)null|NIL"])
        .output_with_stdin("a,b,c,d,e\nnull,NIL,nil,,not null\n")
        .expect_success();
    assert_eq!(output.stdout_str(), "a,b,c,d,e\n,,,,not null\n")
}

#[test]
fn replace_newlines() {
    let testdir = TestDir::new("scrubcsv", "replace_newlines");
    let output = testdir
        .cmd()
        .arg("--replace-newlines")
        .output_with_stdin("a,b\n\"line\r\nbreak\r1\",\"line\nbreak\n2\"\n")
        .expect_success();
    assert_eq!(output.stdout_str(), "a,b\nline break 1,line break 2\n");
}

#[test]
fn trim_whitespace() {
    let testdir = TestDir::new("scrubcsv", "trim_whitespace");
    let output = testdir
        .cmd()
        .arg("--trim-whitespace")
        .output_with_stdin("a,b,c,d\n 1 , 2, ,\n")
        .expect_success();
    assert_eq!(output.stdout_str(), "a,b,c,d\n1,2,,\n");
}

#[test]
fn clean_column_names() {
    let testdir = TestDir::new("scrubcsv", "clean_column_names");
    let output = testdir
        .cmd()
        .arg("--clean-column-names")
        .output_with_stdin(",,a,a\n")
        .expect_success();
    assert_eq!(output.stdout_str(), "_,__2,a,a_2\n");
}

#[test]
fn drop_row_if_null() {
    let testdir = TestDir::new("scrubcsv", "replace_newlines");
    let output = testdir
        .cmd()
        .arg("--drop-row-if-null=c1")
        .arg("--drop-row-if-null=c2")
        .args(&["--null", "NULL"])
        .output_with_stdin(
            r#"c1,c2,c3
1,,
,2,
NULL,3,
a,b,c
"#,
        )
        .expect("error running scrubcsv");
    eprintln!("{}", output.stderr_str());
    //assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        output.stdout_str(),
        r#"c1,c2,c3
a,b,c
"#
    );
}

#[test]
fn drop_row_if_null_saved() {
    let testdir = TestDir::new("scrubcsv", "drop_row_if_null_saved");
    let output = testdir
        .cmd()
        .arg("--drop-row-if-null=c1")
        .arg("--drop-row-if-null=c2")
        .args(&["--bad-rows-path", "bad.csv"])
        .output_with_stdin(
            r#"c1,c2,c3
1,,
a,b,c
1,2,3
3,2,1
1,4,5
2,2,2
1,1,1
5,5,5
2,2,2
1,1,1
"#,
        )
        .expect("error running scrubcsv");
    eprintln!("{}", output.stderr_str());
    testdir.expect_file_contents("bad.csv", "1,,\n");
}
