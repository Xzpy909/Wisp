fn main() {
    slint_build::compile("../../ui/popup.slint").expect("compile Slint UI");
    // The app icon, shown in Explorer and the Start menu, and used for the tray.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=wisp.rc");
        println!("cargo:rerun-if-changed=../../assets/icons/wisp.ico");
        embed_resource::compile("wisp.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("compile Windows resources");
    }
}
