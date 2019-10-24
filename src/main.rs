// Declare a list of external crates.
#[macro_use]
extern crate error_chain;

// Import from other crates.
use humansize::{file_size_opts, FileSize};
use lazy_static::lazy_static;
use log::debug;
use regex::bytes::Regex;
use std::{
    borrow::Cow,
    fs,
    io::{self, prelude::*},
    process,
};
use structopt::StructOpt;

// Modules defined in separate files.
mod errors;
mod util;

// Import from our own crates.
use crate::errors::*;
use crate::util::parse_char_specifier;

/// Our command-line arguments.
#[derive(Debug, StructOpt)]
#[structopt(
    name = "scrubcsv",
    about = "Clean and normalize a CSV file.",
    after_help = "Read a CSV file, normalize the \"good\" lines, and print them to standard
output.  Discard any lines with the wrong number of columns.

Regular expressions use Rust syntax, as described here:
https://doc.rust-lang.org/regex/regex/index.html#syntax

scrubcsv should work with any ASCII-compatible encoding, but it will not
attempt to transcode.

Exit code:
    0 on success
    1 on error
    2 if more than 10% of rows were bad"
)]
struct Opt {
    /// Input file (uses stdin if omitted).
    input: Option<String>,

    /// Character used to separate fields in a row (must be a single ASCII
    /// byte, or "tab").
    #[structopt(
        value_name = "CHAR",
        short = "d",
        long = "delimiter",
        default_value = ","
    )]
    delimiter: String,

    /// Convert values matching NULLREGEX to an empty string.
    #[structopt(value_name = "NULLREGEX", short = "n", long = "null")]
    null: Option<String>,

    // Replace LF and CRLF sequences in values with spaces. This should improve
    // compatibility with systems like BigQuery that don't expect newlines
    // inside escaped strings.
    #[structopt(long = "replace-newlines")]
    replace_newlines: bool,

    // Drop any rows where the specified column is empty or NULL. Can be passed
    // more than once.
    #[structopt(value_name = "COL", long = "drop-row-if-null")]
    drop_row_if_null: Vec<String>,

    // Do not print performance information.
    #[structopt(short = "q", long = "quiet")]
    quiet: bool,

    // Character used to quote entries. May be set to "none" to ignore all
    // quoting.
    #[structopt(value_name = "CHAR", long = "quote", default_value = "\"")]
    quote: String,
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
    let opt = Opt::from_args();
    debug!("Options: {:#?}", opt);

    // Figure out our field delimiter and quote character.
    let delimiter = match parse_char_specifier(&opt.delimiter)? {
        Some(d) => d,
        _ => return Err("field delimiter is required".into()),
    };
    let quote = parse_char_specifier(&opt.quote)?;

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
    let unbuffered_input: Box<dyn Read> = if let Some(ref path) = opt.input {
        Box::new(fs::File::open(path)?)
    } else {
        Box::new(stdin.lock())
    };
    let input = io::BufReader::new(unbuffered_input);

    // Build a set containing all our `--null` values.
    let null_re = if let Some(null_re_str) = opt.null.as_ref() {
        let s = format!("^{}$", null_re_str);
        let re = Regex::new(&s)
            .chain_err(|| -> Error { "can't compile regular expression".into() })?;
        Some(re)
    } else {
        None
    };

    // Create our CSV reader.
    let mut rdr_builder = csv::ReaderBuilder::new();
    // We need headers so that we can honor --drop-row-if-null.
    rdr_builder.has_headers(true);
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
    let hdr = rdr.byte_headers()?.to_owned();

    // Just in --drop-row-if-null was passed, precompute which columns are
    // required.
    let required_cols = hdr
        .iter()
        .map(|name| -> bool {
            opt.drop_row_if_null
                .iter()
                .any(|requried_name| requried_name.as_bytes() == name)
        })
        .collect::<Vec<bool>>();

    // Create our CSV writer.  Note that we _don't_ allow variable numbers
    // of columns, non-standard delimiters, or other nonsense: We want our
    // output to be highly normalized.
    let mut wtr = csv::WriterBuilder::new().flexible(true).from_writer(output);
    wtr.write_byte_record(&hdr)?;

    // Keep track of total rows and malformed rows seen.
    let mut rows: u64 = 1;
    let mut bad_rows: u64 = 0;

    // Can we use the fast path and copy the data through unchanged? Or do we
    // need to clean up emebedded newlines in our data? (These break BigQuery,
    // for example.)
    let use_fast_path =
        null_re.is_none() && !opt.replace_newlines && opt.drop_row_if_null.is_empty();

    // Iterate over all the rows, checking to make sure they look
    // reasonable.
    //
    // If we use the lowest-level, zero-copy API for `csv`, we can process
    // about 225 MB/s.  But it turns out we can't do that, because we need to
    // have a copy of all the row's fields before deciding whether or not
    // to write it out.
    'next_row: for record in rdr.byte_records() {
        let record = record?;

        // Keep track of how many rows we've seen.
        rows += 1;

        // Check if we have the right number of columns in this row.
        if record.len() != hdr.len() {
            bad_rows += 1;
            continue 'next_row;
        }

        // Decide how to handle this row.
        if use_fast_path {
            // We don't need to do anything fancy, so just pass it through.
            // I'm not sure how much this actually buys us in current Rust
            // versions, but it seemed like a good idea at the time.
            wtr.write_record(record.into_iter())?;
        } else {
            // We need to apply one or more cleanups, so run the slow path.
            let cleaned = record.into_iter().map(|mut val: &[u8]| -> Cow<[u8]> {
                // Convert values matching `--null` regex to empty strings.
                if let Some(ref null_re) = null_re {
                    if null_re.is_match(&val) {
                        val = &[]
                    }
                }

                // Fix newlines.
                if opt.replace_newlines
                    && (val.contains(&b'\n') || val.contains(&b'\r'))
                {
                    NEWLINE_RE.replace_all(val, &b" "[..])
                } else {
                    Cow::Borrowed(val)
                }
            });
            if opt.drop_row_if_null.is_empty() {
                // Still somewhat fast!
                wtr.write_record(cleaned)?;
            } else {
                // We need to rebuild the record, check for null columns,
                // and only output the record if everything's OK.
                let row = cleaned.collect::<Vec<Cow<[u8]>>>();
                for (value, &is_required_col) in row.iter().zip(required_cols.iter()) {
                    // If the column is NULL but shouldn't be, bail on this row.
                    if is_required_col && value.is_empty() {
                        bad_rows += 1;
                        continue 'next_row;
                    }
                }
                wtr.write_record(row)?;
            }
        }
    }

    // Print out some information about our run.
    if !opt.quiet {
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
