#[cfg(target_os = "windows")]
fn main() {
    let mut res = winres::WindowsResource::new();
    res.set_icon("assets/icon.ico");
    res.compile().expect("Failed to compile resources");
}

#[cfg(not(target_os = "windows"))]
fn main() {}