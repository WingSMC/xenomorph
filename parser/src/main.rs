use xenomorph_common::module::XenoRegistry;

fn main() {
    let reg = match XenoRegistry::load_workspace(true) {
        Ok(r) => r,
        Err(e) => {
            for err in e {
                eprintln!("[Error]: {}", err);
            }
            std::process::exit(1);
        }
    };

    let cache = reg.module_cache.read().unwrap();
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
