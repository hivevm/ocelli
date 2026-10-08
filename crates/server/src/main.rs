//! `ocelli-server --root <folder>`: serves the web interface for the repositories below the
//! folder, on this machine's loopback addresses or over TLS on the address `--listen` names, and
//! prints once the URL that logs a browser in, with the certificate's fingerprint under TLS
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
    match &running.fingerprint {
        None => println!(
            "ocelli-server serves the repositories below {}\nOpen in a browser on this machine:\n\n    {}\n",
            running.root.display(),
            running.url
        ),
        Some(fingerprint) => println!(
            "ocelli-server serves the repositories below {} over TLS\nOpen in a browser:\n\n    {}\n\n\
             Before you accept the certificate, compare its SHA-256 fingerprint with:\n\n    {}\n",
            running.root.display(),
            running.url,
            fingerprint
        ),
    }
    // Runs until it is stopped; a restart ends every session.
    std::future::pending::<()>().await;
}
