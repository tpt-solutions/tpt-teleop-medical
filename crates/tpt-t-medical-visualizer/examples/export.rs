//! Writes the demonstration scene as a self-contained HTML page.
//!
//! Run with `cargo run -p tpt-t-medical-visualizer --example export -- <path>`.
//! The path is required, so the page is never written to a location chosen for
//! you.

use std::path::PathBuf;

use tpt_t_medical_visualizer::{html, scene};

const WIDTH_PX: u32 = 900;
const HEIGHT_PX: u32 = 640;

fn main() -> std::io::Result<()> {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: export <output.html>");
        std::process::exit(2);
    };
    let demo = scene::demo();
    let view = scene::fit(
        &demo.trajectory_m,
        f64::from(WIDTH_PX),
        f64::from(HEIGHT_PX),
    );
    std::fs::write(&path, html::render(&demo, &view, WIDTH_PX, HEIGHT_PX))?;
    println!("wrote {}", path.display());
    Ok(())
}
