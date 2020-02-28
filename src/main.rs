#![warn(clippy::all)]
#![forbid(unsafe_code)]

// Import from other crates.
use csv::ByteRecord;
use humansize::{file_size_opts, FileSize};
use lazy_static::lazy_static;
use log::debug;
use regex::bytes::Regex;
use std::{
    borrow::Cow,
    fs,
    io::{self, prelude::*},
    path::PathBuf,
    process,
};
use structopt::StructOpt;

// Modules defined in separate files.
#[macro_use]
mod errors;
mod uniquifier;
mod util;

// Import from our own crates.
use crate::errors::*;
use crate::uniquifier::Uniquifier;
use crate::util::{now, CharSpecifier};

/// Use reasonably large input and output buffers. This seems to give us a
/// performance boost of around 5-10% compared to the standard 8 KiB buffer used
/// by `csv`.
const BUFFER_SIZE: usize = 256 * 1024;

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
    input: Option<PathBuf>,

    /// Character used to separate fields in a row (must be a single ASCII
    /// byte, or "tab").
    #[structopt(
        value_name = "CHAR",
        short = "d",
        long = "delimiter",
        default_value = ","
    )]
    delimiter: CharSpecifier,

    /// Convert values matching NULL_REGEX to an empty string. For a case-insensitive
    /// match, use `(?i)`: `--null '(?i)NULL'`.
    #[structopt(value_name = "NULL_REGEX", short = "n", long = "null")]
    null: Option<String>,

    /// Replace LF and CRLF sequences in values with spaces. This should improve
    /// compatibility with systems like BigQuery that don't expect newlines
    /// inside escaped strings.
    #[structopt(long = "replace-newlines")]
    replace_newlines: bool,

    /// Remove whitespace at beginning and end of each cell.
    #[structopt(long = "trim-whitespace")]
    trim_whitespace: bool,

    /// Make sure column names are unique, and use only lowercase letters, numbers
    /// and underscores.
    #[structopt(long = "clean-column-names")]
    clean_column_names: bool,

    /// Drop any rows where the specified column is empty or NULL. Can be passed
    /// more than once. Useful for cleaning primary key columns before
    /// upserting. Uses the cleaned form of column names.
    #[structopt(value_name = "COL", long = "drop-row-if-null")]
    drop_row_if_null: Vec<String>,

    /// Do not print performance information.
    #[structopt(short = "q", long = "quiet")]
    quiet: bool,

    /// Character used to quote entries. May be set to "none" to ignore all
    /// quoting.
    #[structopt(value_name = "CHAR", long = "quote", default_value = "\"")]
    quote: CharSpecifier,

    /// Save badly formed rows to a file.
    #[structopt(value_name = "PATH", long = "bad-rows-path")]
    bad_rows_path: Option<PathBuf>,
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
    let opt: Opt = Opt::from_args();
    debug!("Options: {:#?}", opt);

    // Remember the time we started.
    let start_time = now();

    // Build a regex containing our `--null` value.
    let null_re = if let Some(null_re_str) = opt.null.as_ref() {
        // Always match the full CSV value.
        let s = format!("^{}$", null_re_str);
        let re = Regex::new(&s).context("can't compile regular expression")?;
        Some(re)
    } else {
        None
    };

    // Fetch our input from either standard input or a file.  The only tricky
    // detail here is that we use a `Box<dyn Read>` to represent "some object
    // implementing `Read`, stored on the heap."  This allows us to do runtime
    // dispatch (as if Rust were object oriented).  But because `csv` wraps a
    // `BufReader` around the box, we only do that dispatch once per buffer
    // flush, not on every tiny write.
    let stdin = io::stdin();
    let input: Box<dyn Read> = if let Some(ref path) = opt.input {
        Box::new(
            fs::File::open(path)
                .with_context(|_| format!("cannot open {}", path.display()))?,
        )
    } else {
        Box::new(stdin.lock())
    };

    // Create our CSV reader.
    let mut rdr_builder = csv::ReaderBuilder::new();
    // Set a reasonable buffer size.
    rdr_builder.buffer_capacity(BUFFER_SIZE);
    // We need headers so that we can honor --drop-row-if-null.
    rdr_builder.has_headers(true);
    // Allow records with the wrong number of columns.
    rdr_builder.flexible(true);
    // Configure our delimiter.
    if let Some(delimiter) = opt.delimiter.char() {
        rdr_builder.delimiter(delimiter);
    } else {
        return Err(format_err!("field delimiter is required"));
    }
    // Configure our quote character.
    if let Some(quote) = opt.quote.char() {
        rdr_builder.quote(quote);
    } else {
        rdr_builder.quoting(false);
    }
    let mut rdr = rdr_builder.from_reader(input);

    // We lock `stdout`, giving us exclusive access. In the past, this has made
    // an enormous difference in performance.
    let stdout = io::stdout();
    let output = stdout.lock();

    // Create our CSV writer.  Note that we _don't_ allow variable numbers
    // of columns, non-standard delimiters, or other nonsense: We want our
    // output to be highly normalized.
    let mut wtr = csv::WriterBuilder::new()
        .buffer_capacity(BUFFER_SIZE)
        .from_writer(output);

    // Create out CSV writer for bad rows if it is requested.
    let mut bad_rows_wtr = if let Some(ref path) = opt.bad_rows_path {
        Some(csv::WriterBuilder::new().from_path(path)?)
    } else {
        None
    };

    // Get our header and, if we were asked, make sure all the column names are unique.
    let mut hdr = rdr
        .byte_headers()
        .context("cannot read headers")?
        .to_owned();
    if opt.clean_column_names {
        let mut uniquifier = Uniquifier::default();
        let mut new_hdr = ByteRecord::default();
        for col in hdr.into_iter() {
            // Convert from bytes to UTF-8, make unique (and clean), and convert back to bytes.
            let col = String::from_utf8_lossy(col);
            let col = uniquifier.unique_id_for(&col)?.to_owned();
            new_hdr.push_field(col.as_bytes());
        }
        hdr = new_hdr;
    }

    // Write our header to our output.
    wtr.write_byte_record(&hdr)
        .context("cannot write headers")?;

    // Calculate the number of expected columns.
    let expected_cols = hdr.len();

    // Just in case --drop-row-if-null was passed, precompute which columns are
    // required to contain a value.
    let required_cols = hdr
        .iter()
        .map(|name| -> bool {
            opt.drop_row_if_null
                .iter()
                .any(|requried_name| requried_name.as_bytes() == name)
        })
        .collect::<Vec<bool>>();

    // Keep track of total rows and malformed rows seen. We count the header as
    // a row for backwards compatibility.
    let mut rows: u64 = 1;
    let mut bad_rows: u64 = 0;

    // Can we use the fast path and copy the data through unchanged? Or do we
    // need to clean up emebedded newlines in our data? (These break BigQuery,
    // for example.)
    let use_fast_path = null_re.is_none()
        && !opt.replace_newlines
        && !opt.trim_whitespace
        && opt.drop_row_if_null.is_empty();

    // Iterate over all the rows, checking to make sure they look reasonable.
    //
    // If we use the lowest-level, zero-copy API for `csv`, we can process about
    // 225 MB/s.  But it turns out we can't do that, because we need to count
    // all the row's fields before deciding whether or not to write it out.
    'next_row: for record in rdr.byte_records() {
        let record = record.context("cannot read record")?;

        // Keep track of how many rows we've seen.
        rows += 1;

        // Check if we have the right number of columns in this row.
        if record.len() != expected_cols {
            bad_rows += 1;
            if let Some(ref mut wtr_bad) = bad_rows_wtr {
                wtr_bad
                    .write_record(record.into_iter())
                    .context("cannot write record")?;
            };
            continue 'next_row;
        }

        // Decide how to handle this row.
        if use_fast_path {
            // We don't need to do anything fancy, so just pass it through.
            // I'm not sure how much this actually buys us in current Rust
            // versions, but it seemed like a good idea at the time.
            wtr.write_record(record.into_iter())
                .context("cannot write record")?;
        } else {
            // We need to apply one or more cleanups, so run the slow path.
            let cleaned = record.into_iter().map(|mut val: &[u8]| -> Cow<[u8]> {
                // Convert values matching `--null` regex to empty strings.
                if let Some(ref null_re) = null_re {
                    if null_re.is_match(&val) {
                        val = &[]
                    }
                }

                // Remove whitespace from our cells.
                if opt.trim_whitespace {
                    // We do this manually, because the built-in `trim` only
                    // works on UTF-8 strings, and we work on any
                    // "ASCII-compatible" encoding.
                    let first = val.iter().position(|c| !c.is_ascii_whitespace());
                    let last = val.iter().rposition(|c| !c.is_ascii_whitespace());
                    val = match (first, last) {
                        (Some(first), Some(last)) if first <= last => {
                            &val[first..=last]
                        }
                        (None, None) => &[],
                        _ => panic!(
                            "tried to trim {:?}, got impossible indices {:?} {:?}",
                            val, first, last,
                        ),
                    };
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
                wtr.write_record(cleaned).context("cannot write record")?;
            } else {
                // We need to rebuild the record, check for null columns,
                // and only output the record if everything's OK.
                let row = cleaned.collect::<Vec<Cow<[u8]>>>();
                for (value, &is_required_col) in row.iter().zip(required_cols.iter()) {
                    // If the column is NULL but shouldn't be, bail on this row.
                    if is_required_col && value.is_empty() {
                        bad_rows += 1;
                        if let Some(ref mut wtr_bad) = bad_rows_wtr {
                            wtr_bad
                                .write_record(record.into_iter())
                                .context("cannot write record")?;
                        };
                        continue 'next_row;
                    }
                }
                wtr.write_record(row).context("cannot write record")?;
            }
        }
    }

    // Flush all our buffers.
    wtr.flush().context("error writing records")?;

    // Print out some information about our run.
    if !opt.quiet {
        let ellapsed = (now() - start_time).as_seconds_f64();
        let bytes_per_second = (rdr.position().byte() as f64 / ellapsed) as i64;
        eprintln!(
            "{} rows ({} bad) in {:.2} seconds, {}/sec",
            rows,
            bad_rows,
            ellapsed,
            bytes_per_second.file_size(file_size_opts::BINARY)?,
        );
    }

    // If more than 10% of rows are bad, assume something has gone horribly
    // wrong.
    if bad_rows.checked_mul(10).expect("multiplication overflow") > rows {
        eprintln!("Too many rows ({} of {}) were bad", bad_rows, rows);
        process::exit(2);
    }

    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("ERROR: {}", err);
        let mut source = err.source();
        while let Some(cause) = source {
            eprintln!("  caused by: {}", cause);
            source = cause.source();
        }
        process::exit(1);
    }
}
