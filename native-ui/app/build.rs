use std::path::{Path, PathBuf};

fn main() {
    let source = Path::new("surface.contract");
    contract::rerun_if_changed(source);
    let compiled = contract::compile_path(source).expect("compile Flit UI Contract");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    std::fs::write(output.join("surface.plan"), compiled.encode()).expect("write plan");
}
