fn main() {
    // `sqlx::migrate!` bettet die Migrationen ein; neue Dateien (S2, WPs) sollen neu bauen.
    println!("cargo:rerun-if-changed=migrations");
}
