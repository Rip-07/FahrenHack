// Draws the FahrenHack icon (see src/logo.rs) into an .ico and embeds it into the exe.

include!("src/logo.rs");

fn main() {
    println!("cargo:rerun-if-changed=src/logo.rs");
    println!("cargo:rerun-if-changed=build.rs");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = out.join("fahrenhack.ico");
    std::fs::write(&ico, make_ico(&[16, 24, 32, 48, 64, 128, 256])).expect("write icon");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico.to_str().unwrap());
        res.set("ProductName", "FahrenHack");
        res.set("FileDescription", "FahrenHack: trainer for single-player games");
        res.set("LegalCopyright", "MIT License");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Иконка не встроена в exe (нет rc.exe из Windows SDK?): {e}");
        }
    }
}

/// .ico with 32-bit BMP images (BGRA, bottom-up, empty AND mask).
fn make_ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&s| {
            let rgba = logo_rgba(s);
            let mask_row = ((s + 31) / 32 * 4) as usize;
            let mut img = Vec::new();
            img.extend_from_slice(&40u32.to_le_bytes());
            img.extend_from_slice(&(s as i32).to_le_bytes());
            img.extend_from_slice(&((s * 2) as i32).to_le_bytes());
            img.extend_from_slice(&1u16.to_le_bytes());
            img.extend_from_slice(&32u16.to_le_bytes());
            img.extend_from_slice(&0u32.to_le_bytes());
            img.extend_from_slice(&((s * s * 4) as u32 + (mask_row * s as usize) as u32).to_le_bytes());
            img.extend_from_slice(&[0u8; 16]);
            for y in (0..s).rev() {
                for x in 0..s {
                    let i = ((y * s + x) * 4) as usize;
                    img.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
                }
            }
            img.extend(std::iter::repeat(0u8).take(mask_row * s as usize));
            img
        })
        .collect();

    let mut ico = Vec::new();
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (img, &s) in images.iter().zip(sizes) {
        let dim = if s >= 256 { 0u8 } else { s as u8 };
        ico.extend_from_slice(&[dim, dim, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(img.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        ico.extend(img);
    }
    ico
}
