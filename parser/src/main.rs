use xenomorph_common::config::{write_rc_schema, Config, RC_SCHEMA_RELATIVE_PATH};
use xenomorph_common::module::XenoRegistry;
use xenomorph_common::plugins::XenoPlugin;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("schema") {
        generate_rc_schema();
        return;
    }

    run_parser();
}

/// Generates the `xenomorph.toml` JSON Schema (base + plugin contributions) and
/// writes it to `.xenomorph/xenomorph.schema.json` in the workspace root.
fn generate_rc_schema() {
    let plugins = XenoPlugin::get_plugins();
    let out_path = Config::get().workdir.join(RC_SCHEMA_RELATIVE_PATH);

    match write_rc_schema(plugins, &out_path) {
        Ok(()) => println!("✓ Wrote xenomorph.toml schema → {}", out_path.display()),
        Err(e) => {
            eprintln!("✗ Failed to write xenomorph.toml schema: {}", e);
            std::process::exit(1);
        }
    }
}

fn run_parser() {
    let reg = match XenoRegistry::load_workspace(true) {
        Ok(r) => r,
        Err(e) => {
            for err in e {
                eprintln!("[Error]: {}", err);
            }
            std::process::exit(1);
        }
    };

    let cache = reg.module_cache.blocking_read();
    let module_count = cache.len();
    let total_errors: usize = cache
        .values()
        .map(|m| {
            m.borrow_lexer_errors().len()
                + m.borrow_parser_errors().len()
                + m.borrow_analyzer_errors().len()
                + m.borrow_module_errors().len()
        })
        .sum();

    for module in cache.values() {
        let path = module.borrow_module_path();
        let decl_count = module.borrow_declarations().len();
        let errors: Vec<_> = module
            .borrow_analyzer_errors()
            .iter()
            .chain(module.borrow_parser_errors())
            .chain(module.borrow_lexer_errors())
            .chain(module.borrow_module_errors())
            .collect();

        if errors.is_empty() {
            println!("✓ {} ({} declarations)", path, decl_count);
        } else {
            eprintln!("✗ {} ({} errors)", path, errors.len());
            for err in &errors {
                eprintln!("  └ {}", err);
            }
        }
    }

    println!(
        "\n{} module(s) processed, {} error(s)",
        module_count, total_errors
    );
}
