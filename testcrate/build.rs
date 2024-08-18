use micropython_src::Build;
fn main() {
    let mut build = Build::new(Default::default());
    let _artifacts = build.build();
}
