use xenomorph_common::{config::Config, module::XenoRegistry, plugins::load_plugins};

fn main() {
    let config = Config::get();
    let plugins = load_plugins();

    let dbg_config = &config.debug;
    if dbg_config.plugins {
        println!("{:?}", &plugins);
        println!(
            "{:?}",
            &plugins
                .iter()
                .map(|p| p.provide_types.map(|provide| provide()))
        );
    }

    let file_path = config.workdir.join(&config.parser.path);
    // Load the full module graph starting from the entry file
    let abs_entry = match file_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            println!(
                "Error: Cannot resolve entry file '{}': {}",
                file_path.display(),
                e
            );
            return;
        }
    };

    let (registry, module_errors) = XenoRegistry::load_workspace(&abs_entry);
    for me in &module_errors {
        println!("[module] {}", me);
    }

    let reg = match registry {
        Some(r) => r,
        None => {
            println!("Error: Failed to initialize module registry. Aborting.");
            return;
        }
    };

    let decl_cache = reg.build_declaration_cache();
    if dbg_config.ast {
        println!("Declaration cache:");
        for (name, info) in &decl_cache {
            println!("  {} (from {})", name, info.module_path);
        }
    }
}
