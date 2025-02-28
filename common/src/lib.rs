
#[allow(dead_code)]
pub trait XenoPlugin {
    //fn name(&self) -> &str;
    //fn version(&self) -> &str;

    //fn initialize(&self);
    //fn lint(&self);
    //fn generate(&self) -> String;
    //fn execute(&self, data: &str);
    //fn cleanup(&self);
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Plugin<'a> {
    pub name: &'a str,
    pub version: &'a str,

    // initialize: fn(),
    // lint: fn(),
    // generate: fn() -> String,
    // execute: fn(&str),
    // cleanup: fn(),
}