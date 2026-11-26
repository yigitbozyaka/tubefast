fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-env-changed=TUBEFAST_BETA_NUMBER");
    let release = std::env::var("CARGO_PKG_VERSION").expect("cargo sets the package version");
    let beta_number = std::env::var("TUBEFAST_BETA_NUMBER").unwrap_or_default();
    let version = match beta_number.trim() {
        "" => release,
        number => format!("{release}-beta.{number}"),
    };
    println!("cargo:rustc-env=TUBEFAST_VERSION={version}");
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("assets/icon.ico")
        .compile()
        .expect("the application icon should embed");
}
