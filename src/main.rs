// Declare a list of external crates.
extern crate csv;
extern crate docopt;
extern crate env_logger;
#[macro_use]
extern crate error_chain;
extern crate humansize;
#[macro_use]
extern crate log;
extern crate rustc_serialize;
extern crate time;

// Import from other crates.
use std::env;
use std::fs;
use std::io;
use std::io::BufRead;
use std::io::Write;
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
    -s, --separator CHAR  Character used to separate fields
                          (must be a single ASCII byte) [default: ","]
"#;

/// Our command-line arguments.
#[derive(Debug, RustcDecodable)]
struct Args {
    arg_input: Option<String>,
    flag_separator: String,
}

/// This is a helper function called by our `main` function.  Unlike
/// `main`, we return a `Result`, which means that we can use `?` and other
/// standard error-handling machinery.
fn run(args: &Args) -> Result<()> {

    // Remember the time we started.
    let start_time = time::precise_time_s();

    // Lock stdout for high-performance use by this thread (Rust is trying
    // to protect us against interleaved lines).  We could replace this
    // with and output file pretty easily; just remember to use
    // `io::BufWriter`.
    let stdout_unlocked = io::stdout();
    let stdout = stdout_unlocked.lock();

    // Don't lock `stderr`, somebody might need it to print errors.
    let mut stderr = io::stderr();

    // Build a CSV writer for our good records.
    let mut good: csv::Writer<_> = csv::Writer::from_writer(stdout);

    // Buffer file input, and read line-by-line.  Buffering is _very_
    // important for all file I/O unless you want to make a kernel call
    // every few bytes.
    let file = io::BufReader::new(fs::File::open("example.csv")?);
    for line in file.lines() {
        // Handle any I/O error from our iterator.
        let line = line?;

        // Make a CSV reader and iterate over the lines.
        let mut rdr = csv::Reader::from_string(&line[..])
            .has_headers(false)
            .delimiter(b'|');
        for record in rdr.records() { // or byte_records for non-UTF8/speed
            // At this point, `record` is `Result`, but we don't just want
            // to fail outright like we did for `line` above.
            match record {
                Ok(record) => {
                    good.write(record.into_iter())?;
                }
                Err(_) => {
                    writeln!(&mut stderr, "{}", line)?;
                }
            }
        }
    }

    let ellapsed = time::precise_time_s() - start_time;
    writeln!(io::stderr(), "Time: {:.2} seconds", ellapsed)?;

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
