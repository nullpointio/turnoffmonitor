fn main() {
    // no_std binary: skip the MinGW CRT startup object and use our own entry.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TEST").as_deref() != Ok("true")
    {
        println!("cargo:rustc-link-arg=-nostartfiles");
        println!("cargo:rustc-link-arg=-Wl,-e,mainCRTStartup");
    }
}
