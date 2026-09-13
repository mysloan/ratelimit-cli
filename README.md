# ratelimit-cli

A token bucket rate limiter, as a library, plus a CLI that replays a log of
timestamped events through it so you can see which ones would have been
allowed or denied.

## Why

Rate limit rules are easy to write and hard to reason about once real traffic
patterns are involved. "5 requests per second per API key, burst of 20" sounds
simple, but the question that actually matters is: given yesterday's access
log, how many of those requests would this rule have rejected, and which
clients would have felt it? This tool answers that by running your recorded
timestamps through the same token bucket algorithm a live limiter would use,
without having to stand up the live limiter first.

## Library

`ratelimit::TokenBucketLimiter` keeps one bucket per key (client id, IP,
route, whatever you pass in). Each bucket refills continuously at a fixed
rate up to a capacity, and `check` consumes one token if available.

```rust
use ratelimit::TokenBucketLimiter;

let mut limiter = TokenBucketLimiter::new(/* capacity */ 20.0, /* per second */ 5.0);

if limiter.check("api-key-123", now_ms) {
    // proceed
} else {
    // reject with 429
}
```

Timestamps are passed in by the caller rather than read from the system
clock, which is what makes replaying historical logs deterministic — the
same log always produces the same allow/deny sequence.

## CLI

The `ratelimit` binary reads lines of `<timestamp_ms> <key>` from a file or
from stdin, and prints each line back with `ALLOW` or `DENY` appended.

```
$ cat requests.log
1000 alice
1000 alice
1000 alice
1200 alice
5000 alice

$ ratelimit --rate 1 --burst 3 --input requests.log
1000 alice ALLOW
1000 alice ALLOW
1000 alice ALLOW
1200 alice DENY
5000 alice ALLOW
```

It reads from stdin when `--input` is omitted (or is `-`), so it fits into a
pipeline:

```
$ tail -f /var/log/app/access.log | ./extract-key-and-ts.sh | ratelimit --rate 10 --burst 50
```

### Flags

- `--rate <tokens/sec>` — refill rate, default `1`
- `--burst <capacity>` — bucket size, default `5`
- `--input <path>` — file to read, or `-`/omitted for stdin

## Status

Early. Only the token bucket algorithm is implemented; see the roadmap for
what's next.

## License

MIT, see [LICENSE](LICENSE).
