//! Explicit, offscreen API probe. This example never starts the Herdr runtime.
#[cfg(windows)]
#[path = "../src/platform/windows/offscreen_probe.rs"]
mod offscreen_probe;

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .ok_or("supply an output PNG path")?;
    offscreen_probe::run(std::path::Path::new(&output))
}

#[cfg(not(windows))]
fn main() {
    eprintln!("This isolated Direct2D/DirectWrite probe requires Windows.");
    std::process::exit(1);
}
