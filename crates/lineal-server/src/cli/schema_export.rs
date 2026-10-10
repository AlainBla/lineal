//! `lineal schema-export`: schreibt das GraphQL-SDL (braucht weder Datenbank noch Umgebung).

use anyhow::{Context, Result};
use std::path::Path;

pub fn run(out: &Path) -> Result<()> {
    let mut sdl = lineal_api::graphql::schema_sdl();
    if !sdl.ends_with('\n') {
        sdl.push('\n');
    }
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    std::fs::write(out, sdl).with_context(|| format!("cannot write {}", out.display()))?;
    println!("wrote {}", out.display());
    Ok(())
}
