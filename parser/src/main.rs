use xenomorph_common::module::XenoRegistry;

fn main() {
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
}
