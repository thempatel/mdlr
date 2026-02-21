fn main() {
    // Find the rustc sysroot lib directory and embed it as an rpath,
    // so the binary can find librustc_driver at runtime.
    let output = std::process::Command::new("rustc")
        .arg("--print=sysroot")
        .output()
        .expect("failed to run rustc --print=sysroot");
    let sysroot = String::from_utf8(output.stdout)
        .expect("sysroot is not utf-8")
        .trim()
        .to_string();
    let lib_dir = format!("{sysroot}/lib");

    println!("cargo:rustc-link-arg=-Wl,-rpath,{lib_dir}");

    // Embed the sysroot so standalone mode can set RUSTC to the matching compiler.
    // This ensures cargo-as-library uses the same rustc version as our linked rustc_driver.
    println!("cargo:rustc-env=MDLR_RUSTC_SYSROOT={sysroot}");
}
