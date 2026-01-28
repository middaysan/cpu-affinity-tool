#[cfg(target_os = "windows")]
fn main() {
    println!("cargo:rerun-if-changed=assets/cpu_presets.json");
    let mut res = winres::WindowsResource::new();
    res.set_icon("assets/icon.ico");
    res.compile().expect("Failed to compile resources");
}

#[cfg(not(target_os = "windows"))]
fn main() {}
