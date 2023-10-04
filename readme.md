# ACS DATABASE SCRAPER

A tool written in Rust that:

1. Scrapes every name of every SCP and writes them to a JSON file
2. Scrapes every SCP page to find use of the [Anomaly Classification System](https://scp-wiki.wikidot.com/anomaly-classification-system-guide)
   1. First it finds any page using the Anomaly Classification Bar (Also called the ACS Bar) and pulls specific text from the known structure of the component.
   2. If the ACS Bar is not found, it searches for specific Strings and Text unique to ACS and adds the SCP to the database if they are found
   3. Writes these SCPs using ACS to a JSON file

In order to use with the default values, you can run:

```
cargo run
```

Or, with command line arguments:

```
cargo run -- --start <number> --end <number> --limit <number>
```

There are also 3 command line arguments:
`--start`: The start number used for scraping. The default is `1`.
`--end`: The end number used for scraping. The default is `7999`.
`--limit`: The number of concurrent threads allowed when scraping the scp-wiki. The default is `10`.
