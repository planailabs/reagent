// sqlx::migrate! embeds the migrations at compile time: a new one must rebuild this crate.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
