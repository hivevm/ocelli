//! `ocelli-server --root <folder>`: serves the web interface for the repositories below the
//! folder, on this machine's loopback addresses, and prints once the URL that logs a browser in
//! (ADR-0037, ADR-0038, ADR-0039).

use ocelli_server::{Options, start};

#[tokio::main]
async fn main() {
    let options = match Options::parse(std::env::args()) {
        Ok(options) => options,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let running = match start(options).await {
        Ok(running) => running,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    println!(
        "ocelli-server serves the repositories below {}\nOpen in a browser on this machine:\n\n    {}\n",
        running.root.display(),
        running.url
    );
    // Runs until it is stopped; a restart ends every session.
    std::future::pending::<()>().await;
}
