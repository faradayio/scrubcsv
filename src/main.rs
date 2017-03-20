// Declare a list of external crates.
extern crate csv;
extern crate docopt;
extern crate env_logger;
#[macro_use]
extern crate error_chain;
extern crate humansize;
extern crate libc;
#[macro_use]
extern crate log;
extern crate rustc_serialize;
extern crate time;

// Import from other crates.
use humansize::{FileSize, file_size_opts};
use std::env;
use std::fs;
use std::io;
use std::io::prelude::*;
use std::process;

// Import from our own crates.
use errors::*;

// Modules defined in separate files.
mod errors;

/// Provide a CLI help message, which doctopt will also use to parse our
/// command-line arguments.
const USAGE: &'static str = r#"
Usage: scrubcsv [options] [<input>]
       scrubcsv --help
       scrubcsv --version

Read a CSV file, normalize the "good" lines, and print them to standard
output.  Discard any lines with the wrong number of columns.

Options:
    --help                Show this help message
    --version             Print the version of this program
    -q, --quiet           Do not print performance information
    -d, --delimiter CHAR  Character used to separate fields in a row
                          (must be a single ASCII byte) [default: ,]

Exit code:
    0 on success
    1 on error
    2 if more than 10% of rows were bad
"#;

/// Our command-line arguments.
#[derive(Debug, RustcDecodable)]
struct Args {
    arg_input: Option<String>,
    flag_delimiter: String,
    flag_quiet: bool,
    flag_version: bool,
}

/// This is a helper function called by our `main` function.  Unlike
/// `main`, we return a `Result`, which means that we can use `?` and other
/// standard error-handling machinery.
fn run() -> Result<()> {
    // Set up logging.
    env_logger::init().unwrap();

    // Parse our command-line arguments using `docopt`.
    let args: Args = docopt::Docopt::new(USAGE)
        .and_then(|d| d.argv(env::args()).decode())
        .unwrap_or_else(|e| e.exit());
    debug!("Arguments: {:#?}", args);

    // Print our version if asked to do so.
    if args.flag_version {
        println!("scrubcsv {}", env!("CARGO_PKG_VERSION"));
        process::exit(0);
    }

    // Figure out our field separator.
    let delimiter = if args.flag_delimiter.as_bytes().len() == 1 {
        args.flag_delimiter.as_bytes()[0]
    } else {
        return Err("field delimiter must be exactly one byte".into());
    };

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

    // Create our CSV reader.  Note that we do not allow you to set the
    // quoting character; if you want to do that, please think through the
    // implications carefully.
    let mut rdr = csv::Reader::from_reader(input)
        // Treat headers (if any) as any other record.
        .has_headers(false)
        // Allow records with the wrong number of columns.
        .flexible(true)
        // Configure our delimiter.
        .delimiter(delimiter);

    // Create our CSV writer.  Note that we _don't_ allow variable numbers
    // of columns, non-standard delimiters, or other nonsense: We want our
    // output to be highly normalized.
    let mut wtr = csv::Writer::from_writer(output);

    // Keep track of total rows and malformed rows seen.
    let mut rows: u64 = 0;
    let mut bad_rows: u64 = 0;

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

        // If this is good row, output it.
        if is_good {
            wtr.write(record.into_iter())?;
        } else {
            bad_rows += 1;
        }
        rows += 1;
    }

    // Print out some information about our run.
    if !args.flag_quiet {
        let ellapsed = time::precise_time_s() - start_time;
        let bytes_per_second = (rdr.byte_offset() as f64 / ellapsed) as i64;
        writeln!(io::stderr(),
                 "{} rows ({} bad) in {:.2} seconds, {}/sec",
                 rows,
                 bad_rows,
                 ellapsed,
                 bytes_per_second.file_size(file_size_opts::BINARY)?)?;
    }

    // If more than 10% of rows are bad, assume something has gone horribly
    // wrong.
    if bad_rows * 10 > rows {
        wtr.flush()?;
        writeln!(io::stderr(), "Too many rows ({} of {}) were bad", bad_rows, rows)?;
        process::exit(2);
    }

    Ok(())
}

quick_main!(run);
