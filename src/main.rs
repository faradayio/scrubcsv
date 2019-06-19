// Declare a list of external crates.
extern crate csv;
extern crate docopt;
extern crate env_logger;
#[macro_use]
extern crate error_chain;
extern crate humansize;
extern crate libc;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate regex;
#[macro_use]
extern crate serde_derive;
extern crate time;

// Import from other crates.
use humansize::{file_size_opts, FileSize};
use regex::bytes::Regex;
use std::{
    borrow::Cow,
    fs,
    io::{self, prelude::*},
    process,
};

// Modules defined in separate files.
mod errors;
mod util;

// Import from our own crates.
use crate::errors::*;
use crate::util::parse_char_specifier;

/// Provide a CLI help message, which doctopt will also use to parse our
/// command-line arguments.
const USAGE: &str = r#"
Usage: scrubcsv [options] [<input>]
       scrubcsv --help
       scrubcsv --version

Read a CSV file, normalize the "good" lines, and print them to standard
output.  Discard any lines with the wrong number of columns.

Options:
    --help                Show this help message.
    --version             Print the version of this program.
    -q, --quiet           Do not print performance information.
    -d, --delimiter CHAR  Character used to separate fields in a row.
                          (must be a single ASCII byte). [default: ,]
    --quote CHAR          Character used to quote entries. May be set to
                          "none" to ignore all quoting. [default: "]
    -n, --null NULLREGEX  Convert values matching NULLREGEX to an empty
                          string.
    --replace-newlines    Replace LF and CRLF sequences in values with
                          spaces. This should improve compatibility with
                          systems like BigQuery that don't expect newlines
                          inside escaped strings.

Regular expressions use Rust syntax, as described here:
https://doc.rust-lang.org/regex/regex/index.html#syntax

scrubcsv should work with any ASCII-compatible encoding, but it will not
attempt to transcode.

Exit code:
    0 on success
    1 on error
    2 if more than 10% of rows were bad
"#;

/// Our command-line arguments.
#[derive(Debug, Deserialize)]
struct Args {
    arg_input: Option<String>,
    flag_delimiter: String,
    flag_null: Option<String>,
    flag_replace_newlines: bool,
    flag_quiet: bool,
    flag_quote: String,
    flag_version: bool,
}

lazy_static! {
    /// Either a CRLF newline, a LF newline, or a CR newline. Any of these
    /// will break certain CSV parsers, including BigQuery's CSV importer.
    static ref NEWLINE_RE: Regex = Regex::new(r#"\n|\r\n?"#)
        .expect("regex in source code is unparseable");
}

/// This is a helper function called by our `main` function.  Unlike
/// `main`, we return a `Result`, which means that we can use `?` and other
/// standard error-handling machinery.
fn run() -> Result<()> {
    // Set up logging.
    env_logger::init();

    // Parse our command-line arguments using `docopt`.
    let args: Args = docopt::Docopt::new(USAGE)
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| e.exit());
    debug!("Arguments: {:#?}", args);

    // Print our version if asked to do so.
    if args.flag_version {
        println!("scrubcsv {}", env!("CARGO_PKG_VERSION"));
        process::exit(0);
    }

    // Figure out our field delimiter and quote character.
    let delimiter = match parse_char_specifier(&args.flag_delimiter)? {
        Some(d) => d,
        _ => return Err("field delimiter is required".into()),
    };
    let quote = parse_char_specifier(&args.flag_quote)?;

    // Remember the time we started.
    let start_time = time::precise_time_s();

    // VERY TRICKY!  The precise details of next two lines raise our
    // maximum output speed from 100 MB/s to 3,000 MB/s!  Thanks to talchas
    // and bluss on #rust IRC for helping figure this out.  Rust's usual
    // `stdout` is thread-safe and line-buffered, and we need to bypass all
    // that when writing gigabytes of line-oriented CSV files.
    //
    // If we forget to `lock` standard output, or if don't use a
    // `BufWriter`, then there's some "flush on newline" code that will
    // slow us way down.  We also have the option of being rude and doing
    // this:
    //
    // ```
    // use libc::STDOUT_FILENO;
    // use std::os::unix::io::FromRawFd;
    //
    // fn raw_stdout() -> io::BufWriter<fs::File> {
    //     io::BufWriter::new(unsafe { FromRawFd::from_raw_fd(STDOUT_FILENO) })
    // }
    // ```
    //
    // ...or opening up `/dev/stdout`, but the solution below is within
    // measurement error in Rust 1.13.0, so no need to get cute.
    let stdout = io::stdout();
    let output = io::BufWriter::new(stdout.lock());

    // Fetch our input from either standard input or a file.  The only
    // tricky detail here is that we use a `Box<Read>` to represent "some
    // object implementing `Read`, stored on the heap."  This allows us to
    // do runtime dispatch (as if Rust Rust were object oriented).  But
    // because we wrap the `BufReader` around the box, we only do that
    // dispatch once per buffer flush, not on every tiny write.
    let stdin = io::stdin();
    let unbuffered_input: Box<Read> = if let Some(ref path) = args.arg_input {
        Box::new(fs::File::open(path)?)
    } else {
        Box::new(stdin.lock())
    };
    let input = io::BufReader::new(unbuffered_input);

    // Build a set containing all our `--null` values.
    let null_re = if let Some(null_re_str) = args.flag_null.as_ref() {
        let s = format!("^{}$", null_re_str);
        let re = Regex::new(&s)
            .chain_err(|| -> Error { "can't compile regular expression".into() })?;
        Some(re)
    } else {
        None
    };

    // Create our CSV reader.
    let mut rdr_builder = csv::ReaderBuilder::new();
    // Treat headers (if any) as any other record.
    rdr_builder.has_headers(false);
    // Allow records with the wrong number of columns.
    rdr_builder.flexible(true);
    // Configure our delimiter.
    rdr_builder.delimiter(delimiter);
    // Configure our quote character.
    if let Some(quote) = quote {
        rdr_builder.quote(quote);
    } else {
        rdr_builder.quoting(false);
    }
    let mut rdr = rdr_builder.from_reader(input);

    // Create our CSV writer.  Note that we _don't_ allow variable numbers
    // of columns, non-standard delimiters, or other nonsense: We want our
    // output to be highly normalized.
    let mut wtr = csv::WriterBuilder::new().flexible(true).from_writer(output);

    // Keep track of total rows and malformed rows seen.
    let mut rows: u64 = 0;
    let mut bad_rows: u64 = 0;

    // Can we use the fast path and copy the data through unchanged? Or do we
    // need to clean up emebedded newlines in our data? (These break BigQuery,
    // for example.)
    let use_fast_path = null_re.is_none() && !args.flag_replace_newlines;

    // Iterate over all the rows, checking to make sure they look
    // reasonable.
    //
    // If we use the lowest-level, zero-copy API for `csv`, we can process
    // about 225 MB/s.  But it turns out we can't do that, because we need to
    // have a copy of all the row's fields before deciding whether or not
    // to write it out.
    let mut columns_expected = None;
    for record in rdr.byte_records() {
        let record = record?;

        // Keep track of how many columns we expected.
        let is_good = match columns_expected {
            // This is the first row.
            None => {
                columns_expected = Some(record.len());
                true
            }
            // We know how many columns we expect, and it matches.
            Some(expected) if record.len() == expected => true,
            // The current row is weird.
            Some(_) => false,
        };

        // If this is a good row, output it.
        if is_good {
            if use_fast_path {
                // We don't need to do anything fancy, so just pass it through.
                // I'm not sure how much this actually buys us in current Rust
                // versions, but it seemed like a good idea at the time.
                wtr.write_record(record.into_iter())?;
            } else {
                // We need to apply one or more cleanups, so run the slow path.
                wtr.write_record(record.into_iter().map(|mut val| {
                    // Convert values matching `--null` regex to empty strings.
                    if let Some(ref null_re) = null_re {
                        if null_re.is_match(&val) {
                            val = &[]
                        }
                    }

                    // Fix newlines.
                    if args.flag_replace_newlines
                        && (val.contains(&b'\n') || val.contains(&b'\r'))
                    {
                        NEWLINE_RE.replace_all(val, &b" "[..])
                    } else {
                        Cow::Borrowed(val)
                    }
                }))?;
            }
        } else {
            bad_rows += 1;
        }
        rows += 1;
    }

    // Print out some information about our run.
    if !args.flag_quiet {
        let ellapsed = time::precise_time_s() - start_time;
        let bytes_per_second = (rdr.position().byte() as f64 / ellapsed) as i64;
        writeln!(
            io::stderr(),
            "{} rows ({} bad) in {:.2} seconds, {}/sec",
            rows,
            bad_rows,
            ellapsed,
            bytes_per_second.file_size(file_size_opts::BINARY)?
        )?;
    }

    // If more than 10% of rows are bad, assume something has gone horribly
    // wrong.
    if bad_rows * 10 > rows {
        wtr.flush()?;
        writeln!(
            io::stderr(),
            "Too many rows ({} of {}) were bad",
            bad_rows,
            rows
        )?;
        process::exit(2);
    }

    Ok(())
}

quick_main!(run);
