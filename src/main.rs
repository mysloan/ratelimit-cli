use ratelimit::{RateLimiter, SlidingWindowLimiter, TokenBucketLimiter};
use std::fs::File;
use std::io::{self, BufRead, BufReader};

enum Algo {
    TokenBucket,
    SlidingWindow,
}

struct Args {
    algo: Algo,
    rate: f64,
    burst: f64,
    limit: usize,
    window_ms: u64,
    input: Option<String>,
}

fn parse_args() -> Args {
    let mut algo = Algo::TokenBucket;
    let mut rate = 1.0;
    let mut burst = 5.0;
    let mut limit = 5;
    let mut window_ms = 1000;
    let mut input = None;

    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--algo" => {
                i += 1;
                algo = match raw
                    .get(i)
                    .unwrap_or_else(|| fail("--algo requires a value"))
                    .as_str()
                {
                    "token-bucket" => Algo::TokenBucket,
                    "sliding-window" => Algo::SlidingWindow,
                    other => fail(&format!(
                        "unknown --algo {other:?}, expected token-bucket or sliding-window"
                    )),
                };
            }
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
            "--limit" => {
                i += 1;
                limit = raw
                    .get(i)
                    .unwrap_or_else(|| fail("--limit requires a value"))
                    .parse()
                    .unwrap_or_else(|_| fail("--limit must be a whole number"));
            }
            "--window-ms" => {
                i += 1;
                window_ms = raw
                    .get(i)
                    .unwrap_or_else(|| fail("--window-ms requires a value"))
                    .parse()
                    .unwrap_or_else(|_| fail("--window-ms must be a whole number"));
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

    Args {
        algo,
        rate,
        burst,
        limit,
        window_ms,
        input,
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    print_usage();
    std::process::exit(1);
}

fn print_usage() {
    eprintln!(
        "usage: ratelimit [--algo token-bucket|sliding-window] [options] [--input <path>]\n\n\
         Reads lines of \"<timestamp_ms> <key>\" from --input, or from stdin if\n\
         --input is omitted or is \"-\". Prints each line back out with ALLOW or\n\
         DENY appended, according to a rate limiter shared per key.\n\n\
         token-bucket options (default algorithm):\n\
         \x20 --rate <tokens/sec>   refill rate, default 1\n\
         \x20 --burst <capacity>    bucket size, default 5\n\n\
         sliding-window options:\n\
         \x20 --limit <count>       max requests per window, default 5\n\
         \x20 --window-ms <ms>      window size in milliseconds, default 1000"
    );
}

fn main() {
    let args = parse_args();
    let mut limiter: Box<dyn RateLimiter> = match args.algo {
        Algo::TokenBucket => Box::new(TokenBucketLimiter::new(args.burst, args.rate)),
        Algo::SlidingWindow => Box::new(SlidingWindowLimiter::new(args.limit, args.window_ms)),
    };

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
