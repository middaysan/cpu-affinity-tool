#[cfg(target_os = "windows")]
fn main() {
    println!("cargo:rerun-if-changed=assets/cpu_presets.json");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=app.manifest");
    let mut res = winres::WindowsResource::new();
    res.set_icon("assets/icon.ico");
    if matches!(std::env::var("PROFILE").as_deref(), Ok("release")) {
        res.set_manifest_file("app.manifest");
    }
    res.compile().expect("Failed to compile resources");
}

#[cfg(not(target_os = "windows"))]
fn main() {}
