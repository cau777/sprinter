use serde::Serialize;
use std::{error::Error, fs, path::PathBuf};
use ts_rs::TS;

#[derive(Serialize, TS)]
#[ts(export, export_to = "../../web/src/api/types.gen.ts")]
pub struct HealthResponse {
    pub status: String,
}

pub fn export() -> Result<(), Box<dyn Error>> {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../web/src/api/types.gen.ts");
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    HealthResponse::export_all_to(env!("CARGO_MANIFEST_DIR"))?;
    println!("Wrote {}", target.display());
    Ok(())
}
