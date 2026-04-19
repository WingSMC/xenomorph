use xenomorph_common::{config::Config, module::XenoRegistry, plugins::load_plugins};

fn main() {
    let config = Config::get();
    let plugins = load_plugins();

    if config.debug.plugins {
        println!("[Debug] Loaded plugins: {:?}", &plugins);
    }

    let reg = match XenoRegistry::load_workspace() {
        Ok(r) => r,
        Err(e) => {
            for err in e {
                eprintln!("[Module Error]: {}", err);
            }
            return;
        }
    };

    for module in reg.module_cache.read().unwrap().values() {
        println!("Module: {}", module.borrow_module_path());
        println!("  AST: {:?}", module.borrow_ast());
        println!("  Declarations:");
        for d in module.borrow_declarations().values() {
            println!("{:?}", d);
        }
    }

    // let decl_cache = reg.get_all_declarations_in_scope(reg.entry.as_str());
    // if config.debug.ast {
    //     println!("[Debug] Declaration cache:");
    //     for (name, info) in &decl_cache {
    //         println!("  {} (from {})", name, info.module_path);
    //     }
    // }
}
