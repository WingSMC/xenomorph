// Per-edit LSP update cost: `cargo run --release -p xenomorph_common --example bench_edit -- <module>...`
use std::time::Instant;
use xenomorph_common::{config::Config, module::XenoRegistry};

fn main() {
    let workdir = std::env::current_dir().unwrap();
    let config = Config::discover(&workdir).unwrap();
    let start = Instant::now();
    let registry = XenoRegistry::with_config(config, false).unwrap();
    registry.load_module(&[&registry.entry], true, None);
    registry.refresh_name_collision_diagnostics();
    println!(
        "initial: {:?} (root {})",
        start.elapsed(),
        registry.root.display()
    );
    for module in std::env::args().skip(1) {
        let path = registry.root.join(format!("{module}.xen"));
        let source = std::fs::read_to_string(&path).unwrap();
        let start = Instant::now();
        for i in 0..5 {
            registry.load_module_from_source(&path, format!("{source}\n// {i}\n"));
            registry.refresh_name_collision_diagnostics();
        }
        println!("{module}: {:?} per edit", start.elapsed() / 5);
    }
}
