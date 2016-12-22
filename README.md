# scrubcsv: Remove bad lines from a CSV file and normalize the rest

This is a CSV cleaning tool based on BurntSushi's
excellent [`csv`](http://burntsushi.net/rustdoc/csv/) library.  It's
intended to be used for cleaning up and normalizing large data sets before
feeding them to other CSV parsers, at the cost of discarding the occasional
row.

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

## Performance notes

This is designed to be relatively fast.  For comparison purposes, on
particular laptop:

- `cat /dev/zero | pv > /dev/null` shows a throughput of about 5 GB/s.
- The raw output string-writing routines in `scrubcsv` can reach about 3.5
  GB/s.
- The `csv` parser can reach roughly 235 MB/s in zero-copy mode.
- With full processing, `scrubcsv` hits 67 GB/s.
- A lot of old-school C command-line tools hit about 50 to 75 GB/s.

Unfortunately, we can't really use `csv`'s zero-copy mode because we need
to see an entire row at once to decide whether or not it's valid before
deciding to output it.  We could, I suppose, `memmove` each field as we see
it into an existing buffer to avoid `malloc` overhead (which is almost
certianly the bottleneck here), but that would require more code.  Still,
file an issue if performance is a problem.  We could probably make this a
couple of times faster and it would be fun.
