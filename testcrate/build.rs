use micropython_src::Build;
fn main() {
    let mut build = Build::new(Default::default());
    build.build().unwrap();
}
