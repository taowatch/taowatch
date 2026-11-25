// `sqlx::migrate!` embeds the migration files at compile time, but Cargo does not know
// about them: without this, adding a migration would not rebuild the crate.
fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
