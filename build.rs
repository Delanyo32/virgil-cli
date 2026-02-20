fn main() {
    // DuckDB's AdditionalLockInfo uses the Windows Restart Manager API
    // (RmStartSession, RmEndSession, RmRegisterResources, RmGetList).
    // The bundled libduckdb-sys build doesn't always link rstrtmgr, so we
    // need to do it ourselves.
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        println!("cargo:rustc-link-lib=rstrtmgr");
    }
}
