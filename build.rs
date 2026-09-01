fn main() {
    println!("cargo:rerun-if-changed=wit/world.wit");
    println!("cargo:rerun-if-changed=wkg.toml");
    println!("cargo:rerun-if-changed=wkg.lock");

    let status = std::process::Command::new("wkg")
        .args(["wit", "fetch"])
        .status()
        .expect("Failed to run `wkg wit fetch`. `wkg` is likely not installed.");

    if !status.success() {
        println!("cargo:warning=`wkg wit fetch` failed. Using existing WIT deps if available.");
    }
}
