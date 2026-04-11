use xenomorph_common::{config::Config, module::XenoRegistry, plugins::load_plugins};

fn main() {
    let config = Config::get();
    let plugins = load_plugins();

    if config.debug.plugins {
        println!("[Debug] Loaded plugins: {:?}", &plugins);
    }

    let file_path = config.workdir.join(&config.parser.entry);
    let abs_entry = match file_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "[Module Error]: Cannot resolve entry file '{}': {}",
                file_path.display(),
                e
            );
            return;
        }
    };

    let reg = match XenoRegistry::load_workspace() {
        Ok(r) => r,
        Err(e) => {
            for err in e {
                eprintln!("[Module Error]: {}", err);
            }
            return;
        }
    };

    let decl_cache = reg.build_declaration_cache();
    if config.debug.ast {
        println!("[Debug] Declaration cache:");
        for (name, info) in &decl_cache {
            println!("  {} (from {})", name, info.module_path);
        }
    }
}
