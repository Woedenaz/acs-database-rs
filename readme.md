# ACS DATABASE SCRAPER

A tool written in Rust that can do the following:

- Scrapes every name of every SCP and writes them to a JSON file
- Scrapes every SCP page to find use of the [Anomaly Classification System](https://scp-wiki.wikidot.com/anomaly-classification-system-guide)
   1. First it finds any page using the Anomaly Classification Bar (Also called the ACS Bar) and pulls specific text from the known structure of the component.
   2. If the ACS Bar is not found, it searches for specific Strings and Text unique to ACS and adds the SCP to the database if they are found
   3. Writes these SCPs using ACS to a JSON file
- Pulls the backlinks for 3 different components utilizing ACS and writes them to a JSON file
- Cross compares the current Database JSON with the backlinks JSON and adds any missing pages.

## How To Use

To run, use the following command:

```
cargo run
```

Without command-line flags, it will not do anything. Utilize the flags and arguments below to customize the tool:

```
cargo run -- -s -g -b- c -s <number> -e <number> -l <number> -r <number>
```

In the command line, there are 4 arguments and 4 flags:

### Flags
- `--scraper` or `-s`: Enables the base function of scraping the SCP-Wiki for pages using ACS
- `--getnames` or `-g`: Enables the scraping of the SCP Names from the series pages. 
- `--backlinks` or `-b`: Enables the scraping of SCPs using the following component pages:
 - [Anomaly Classification Bar Source](https://scp-wiki.wikidot.com/component:anomaly-class-bar-source)
 - [Flops Header Template](https://scp-wiki.wikidot.com/component:flops-header)
 - [Advanced Information Methodology (AIM) Component](https://scp-wiki.wikidot.com/component:advanced-information-methodology)
- `--cross` or `-c`: Enables the cross-comparison of the current `acs_database.json` with the `acs_backlinks.json` created by the `--backlinks` flag. Any missing SCPs will be added to the database.


### Arguments
- `--start #` or `-s #`: The start number used for scraping. The default is `1`.
- `--end #` or `-e #`: The end number used for scraping. The default is `7999`.
- `--limit #` or `-l #`: The number of concurrent threads allowed when scraping the scp-wiki. The default is `10`.
- `--retries #` or `-r #`: When calling a initially page fails, this is the number of times it will try before continuing. The default is `5`.
