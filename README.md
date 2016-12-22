# scrubcsv: Remove bad lines from a CSV file and normalize the rest

To install, first [install Rust](https://www.rustup.rs/) if you haven't
already:

```sh
curl https://sh.rustup.rs -sSf | sh
```

Then install `scrubcsv` using Cargo:

```sh
cargo install scrubcsv
```

Run it:

```sh
$ scrubcsv giant.csv > scrubbed.csv
3000001 rows (1 bad) in 51.58 seconds, 72.23 MiB/sec
```

For more options, run:

```sh
scrubcsv --help
```
