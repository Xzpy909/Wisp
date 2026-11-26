use std::path::PathBuf;

fn main() {
    slint_build::compile("../../ui/popup.slint").expect("compile Slint UI");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
}

/// The app icon (Explorer, Start menu, and the tray loads it as ordinal 1) and
/// the version details shown in the file's Properties and by installers.
fn windows_resources() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    // No canonicalize(): on Windows it gives a \\?\ path that rc.exe rejects.
    let icon = manifest_dir.parent().and_then(|p| p.parent()).expect("workspace root").join("assets/icons/wisp.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let icon = icon.display().to_string().replace('\\', "/");

    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let mut parts: Vec<u16> = version.split(['.', '-']).take(3).map(|p| p.parse().unwrap_or(0)).collect();
    parts.resize(4, 0);
    let numbers = parts.iter().map(u16::to_string).collect::<Vec<_>>().join(",");

    let rc = format!(
        r#"1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {numbers}
PRODUCTVERSION {numbers}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Wisp"
      VALUE "FileDescription", "Wisp clipboard manager"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "wisp"
      VALUE "OriginalFilename", "wisp.exe"
      VALUE "ProductName", "Wisp"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("wisp.rc");
    std::fs::write(&out, rc).expect("write wisp.rc");
    embed_resource::compile(&out, embed_resource::NONE).manifest_optional().expect("compile Windows resources");
}
