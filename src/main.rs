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
output.  Discard any lines with the wrong number of columns

Options:
    --help                Show this help message
    --version             Print the version of this program
    -v, --verbose
    -d, --delimiter CHAR  Character used to separate fields in a row
                          (must be a single ASCII byte) [default: ","]
"#;

/// Our command-line arguments.
#[derive(Debug, RustcDecodable)]
struct Args {
    arg_input: Option<String>,
    flag_delimiter: String,
}

/// This is a helper function called by our `main` function.  Unlike
/// `main`, we return a `Result`, which means that we can use `?` and other
/// standard error-handling machinery.
fn run(args: &Args) -> Result<()> {

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
    let mut output = io::BufWriter::new(stdout.lock());

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

    // Use the low-level CSV parser API, which doesn't allocate memory.
    let mut rows: u64 = 0;
    let mut rdr = csv::Reader::from_reader(input)
        .flexible(true)
        .delimiter(delimiter);
    while !rdr.done() {
        loop {
            match rdr.next_bytes() {
                csv::NextField::EndOfCsv => break,
                csv::NextField::EndOfRecord => { rows += 1; break; },
                csv::NextField::Error(err) => { return Err(From::from(err)); }
                csv::NextField::Data(_) => {}
            }
        }
    }

    // Print out some information about our run.
    let ellapsed = time::precise_time_s() - start_time;
    let bytes_per_second = (rdr.byte_offset() as f64 / ellapsed) as i64;
    writeln!(io::stderr(),
             "{} rows in {:.2} seconds, {}/sec",
             rows,
             ellapsed,
             bytes_per_second.file_size(file_size_opts::BINARY)?)?;

    Ok(())
}

fn main() {
    // Set up logging.
    env_logger::init().unwrap();

    // Parse our command-line arguments using `docopt`.
    let args: Args = docopt::Docopt::new(USAGE)
        .and_then(|d| d.argv(env::args()).decode())
        .unwrap_or_else(|e| e.exit());
    debug!("Arguments: {:#?}", args);

    // Call our helper function to do the real work, and handle any errors.
    // If we can't write to standard error, these I/O calls might return
    // errors.  We `unwrap` these and panic, because if standard error is
    // closed, it's pretty hopeless.
    if let Err(err) = run(&args) {
        let mut stderr = io::stderr();
        write!(&mut stderr, "ERROR").unwrap();
        for e in err.iter() {
            write!(&mut stderr, ": {}", e).unwrap();
        }
        writeln!(&mut stderr, "").unwrap();
        if let Some(backtrace) = err.backtrace() {
            writeln!(&mut stderr, "Backtrace:\n{:?}", backtrace).unwrap();
        }
        process::exit(1);
    }
}
