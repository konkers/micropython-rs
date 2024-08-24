use micropython_src::{Build, Config};
fn main() {
    let mut build = Build::new(Config::default().qstr("<stdin>"));
    build.build().unwrap();
}
