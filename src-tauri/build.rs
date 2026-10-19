use std::{fs, fs::File, path::Path};

fn main() {
    // Keep source checkouts buildable even when generated Tauri icons are not committed.
    // The placeholder is a simple 128px app mark; projects can replace it with branded assets.
    let icon_path = Path::new("icons/icon.png");
    if !icon_path.is_file() {
        fs::create_dir_all("icons").expect("could not create Tauri icons directory");
        let file = File::create(icon_path).expect("could not create fallback Tauri icon");
        let mut encoder = png::Encoder::new(file, 128, 128);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("could not write fallback icon header");
        let mut pixels = vec![0_u8; 128 * 128 * 4];
        let glyph = [0b11110_u8, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001];
        for y in 0..128_usize {
            for x in 0..128_usize {
                let offset = (y * 128 + x) * 4;
                let mut color = [197_u8, 243, 108, 255];
                if (34..84).contains(&x) && (25..95).contains(&y) {
                    let col = (x - 34) / 10;
                    let row = (y - 25) / 10;
                    if col < 5 && row < 7 && (glyph[row] & (1 << (4 - col))) != 0 {
                        color = [17, 22, 14, 255];
                    }
                }
                pixels[offset..offset + 4].copy_from_slice(&color);
            }
        }
        writer.write_image_data(&pixels).expect("could not write fallback Tauri icon");
    }
    println!("cargo:rerun-if-changed=icons/icon.png");
    tauri_build::build()
}