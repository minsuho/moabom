fn main() {
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .to_path_buf();
    let mut assets = std::process::Command::new("node");
    assets
        .current_dir(&root)
        .arg(root.join("scripts/build-assets.mjs"));
    if std::env::var("PROFILE").as_deref() == Ok("debug") {
        assets.arg("--dev");
    }
    assert!(assets.status().expect("Node.js is required to build assets; install Node.js and run npm ci").success(),
        "Asset protection failed. Run npm ci at the repository root; unprotected release builds are refused.");
    println!(
        "cargo:rustc-env=MOABOM_ASSETS_DIR={}",
        root.join(".generated").display()
    );
    for input in [
        "ui",
        "src-tauri/src/inject",
        "scripts/build-assets.mjs",
        "package.json",
        "package-lock.json",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(input).display());
    }
    // Applied only to this executable, without altering system libraries or debug builds.
    if std::env::var("PROFILE").as_deref() != Ok("debug")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        for flag in [
            "/DEBUG:NONE",
            "/INCREMENTAL:NO",
            "/OPT:REF,ICF",
            "/PDBALTPATH:moabom.pdb",
        ] {
            println!("cargo:rustc-link-arg={flag}");
        }
    }
    tauri_build::build()
}
