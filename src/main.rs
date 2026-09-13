use ratelimit::TokenBucketLimiter;
use std::fs::File;
use std::io::{self, BufRead, BufReader};

struct Args {
    rate: f64,
    burst: f64,
    input: Option<String>,
}

fn parse_args() -> Args {
    let mut rate = 1.0;
    let mut burst = 5.0;
    let mut input = None;

    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--rate" => {
                i += 1;
                rate = raw
                    .get(i)
                    .unwrap_or_else(|| fail("--rate requires a value"))
                    .parse()
                    .unwrap_or_else(|_| fail("--rate must be a number"));
            }
            "--burst" => {
                i += 1;
                burst = raw
                    .get(i)
                    .unwrap_or_else(|| fail("--burst requires a value"))
                    .parse()
                    .unwrap_or_else(|_| fail("--burst must be a number"));
            }
            "--input" => {
                i += 1;
                input = Some(
                    raw.get(i)
                        .unwrap_or_else(|| fail("--input requires a path"))
                        .clone(),
                );
            }
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            other => fail(&format!("unknown argument: {other}")),
        }
        i += 1;
    }

    Args { rate, burst, input }
}

fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    print_usage();
    std::process::exit(1);
}

fn print_usage() {
    eprintln!(
        "usage: ratelimit --rate <tokens/sec> --burst <capacity> [--input <path>]\n\n\
         Reads lines of \"<timestamp_ms> <key>\" from --input, or from stdin if\n\
         --input is omitted or is \"-\". Prints each line back out with ALLOW or\n\
         DENY appended, according to a token bucket limiter shared per key."
    );
}

fn main() {
    let args = parse_args();
    let mut limiter = TokenBucketLimiter::new(args.burst, args.rate);

    let stdin = io::stdin();
    let reader: Box<dyn BufRead> = match args.input.as_deref() {
        None | Some("-") => Box::new(stdin.lock()),
        Some(path) => {
            let file = File::open(path)
                .unwrap_or_else(|e| fail(&format!("cannot open {path}: {e}")));
            Box::new(BufReader::new(file))
        }
    };

    for (lineno, line) in reader.lines().enumerate() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("line {}: read error: {}", lineno + 1, e);
                continue;
            }
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let mut parts = line.splitn(2, char::is_whitespace);
        let ts_str = parts.next().unwrap();
        let key = parts.next().unwrap_or("").trim();
        if key.is_empty() {
            eprintln!(
                "line {}: expected \"<timestamp_ms> <key>\", got {:?}",
                lineno + 1,
                line
            );
            continue;
        }

        let now_ms: u64 = match ts_str.parse() {
            Ok(v) => v,
            Err(_) => {
                eprintln!("line {}: bad timestamp {:?}", lineno + 1, ts_str);
                continue;
            }
        };

        let allowed = limiter.check(key, now_ms);
        println!("{} {} {}", now_ms, key, if allowed { "ALLOW" } else { "DENY" });
    }
}
