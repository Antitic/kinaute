#![allow(clippy::expect_used)]
fn main() {
    slint_build::compile("ui/app.slint").expect("failed to compile the Slint UI");
}
