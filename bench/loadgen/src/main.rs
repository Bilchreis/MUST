// SPDX-License-Identifier: LGPL-3.0-or-later

//! SECoP load client for MUST's benchmarks (see `bench/README.md`).
//!
//! ```text
//! loadgen read ADDR CONNECTIONS IN_FLIGHT SECONDS REQUEST...
//!     CONNECTIONS clients each keep IN_FLIGHT requests outstanding for SECONDS, taking
//!     the REQUEST lines in turn; prints replies per second and latency percentiles
//! loadgen sub ADDR CONNECTIONS SECONDS
//!     CONNECTIONS clients send `activate` and count the lines they receive
//! ```

use std::collections::VecDeque;
use std::process::exit;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

fn usage() -> ! {
    eprintln!(
        "usage: loadgen read ADDR CONNECTIONS IN_FLIGHT SECONDS REQUEST...\n       loadgen sub ADDR CONNECTIONS SECONDS"
    );
    exit(2)
}

fn number<T: std::str::FromStr>(args: &[String], i: usize) -> T {
    args.get(i)
        .and_then(|a| a.parse().ok())
        .unwrap_or_else(|| usage())
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let addr = args.get(2).cloned().unwrap_or_else(|| usage());
    match args.get(1).map(String::as_str) {
        Some("read") if args.len() > 6 => {
            let requests: Vec<String> = args[6..].iter().map(|r| format!("{r}\n")).collect();
            read(
                addr,
                number(&args, 3),
                number(&args, 4),
                number(&args, 5),
                requests,
            )
            .await;
        }
        Some("sub") if args.len() == 5 => sub(addr, number(&args, 3), number(&args, 4)).await,
        _ => usage(),
    }
}

/// Keeps `in_flight` requests outstanding on each of `connections` clients.
async fn read(
    addr: String,
    connections: usize,
    in_flight: usize,
    seconds: f64,
    requests: Vec<String>,
) {
    let requests = Arc::new(requests);
    let latencies = Arc::new(Mutex::new(Vec::<f64>::new()));
    let errors = Arc::new(AtomicU64::new(0));
    let end = Instant::now() + Duration::from_secs_f64(seconds);
    let start = Instant::now();
    let mut tasks = Vec::new();
    for c in 0..connections {
        let (addr, requests, latencies, errors) = (
            addr.clone(),
            Arc::clone(&requests),
            Arc::clone(&latencies),
            Arc::clone(&errors),
        );
        tasks.push(tokio::spawn(async move {
            let stream = TcpStream::connect(&addr).await.expect("connect");
            stream.set_nodelay(true).expect("nodelay");
            let (r, mut w) = stream.into_split();
            let mut r = BufReader::new(r);
            let mut sent = VecDeque::new();
            let mut line = String::new();
            let mut mine = Vec::new();
            let mut next = c;
            for _ in 0..in_flight {
                w.write_all(requests[next % requests.len()].as_bytes())
                    .await
                    .expect("write");
                sent.push_back(Instant::now());
                next += 1;
            }
            loop {
                line.clear();
                if r.read_line(&mut line).await.expect("read") == 0 {
                    break;
                }
                if line.starts_with("error") && errors.fetch_add(1, Ordering::Relaxed) < 3 {
                    eprintln!("{}", line.trim_end());
                }
                let Some(at) = sent.pop_front() else { continue };
                mine.push(at.elapsed().as_secs_f64());
                if Instant::now() < end {
                    w.write_all(requests[next % requests.len()].as_bytes())
                        .await
                        .expect("write");
                    sent.push_back(Instant::now());
                    next += 1;
                } else if sent.is_empty() {
                    break;
                }
            }
            latencies.lock().await.extend(mine);
        }));
    }
    for task in tasks {
        task.await.expect("client task");
    }
    let elapsed = start.elapsed().as_secs_f64();
    let mut all = latencies.lock().await.clone();
    if all.is_empty() {
        println!("no replies");
        return;
    }
    all.sort_by(f64::total_cmp);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let ms = |q: f64| all[((all.len() - 1) as f64 * q) as usize] * 1e3;
    #[allow(clippy::cast_precision_loss)]
    let rate = all.len() as f64 / elapsed;
    println!(
        "{} replies in {elapsed:.2} s = {rate:.0}/s  latency ms p50 {:.3} p90 {:.3} p99 {:.3} max {:.3}  errors {}",
        all.len(),
        ms(0.5),
        ms(0.9),
        ms(0.99),
        ms(1.0),
        errors.load(Ordering::Relaxed)
    );
}

/// Activates `connections` clients and counts the lines they receive.
async fn sub(addr: String, connections: usize, seconds: f64) {
    let lines = Arc::new(AtomicU64::new(0));
    let mut tasks = Vec::new();
    for _ in 0..connections {
        let (addr, lines) = (addr.clone(), Arc::clone(&lines));
        tasks.push(tokio::spawn(async move {
            let stream = TcpStream::connect(&addr).await.expect("connect");
            let (r, mut w) = stream.into_split();
            let mut r = BufReader::new(r);
            w.write_all(b"activate\n").await.expect("write");
            let mut line = String::new();
            let end = tokio::time::Instant::now() + Duration::from_secs_f64(seconds);
            while let Ok(Ok(n)) = tokio::time::timeout_at(end, r.read_line(&mut line)).await {
                if n == 0 {
                    break;
                }
                lines.fetch_add(1, Ordering::Relaxed);
                line.clear();
            }
        }));
    }
    for task in tasks {
        task.await.expect("client task");
    }
    println!(
        "{} lines received by {connections} clients",
        lines.load(Ordering::Relaxed)
    );
}
