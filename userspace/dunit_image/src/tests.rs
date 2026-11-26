use crate::{codecs, decode, load_auto, Error, ImageFormat};

#[test]
fn decodes_shipped_bmp_and_png_assets() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/images");

    let logo_bmp = load_auto(&format!("{}/logo.bmp", root)).expect("logo.bmp");
    assert_eq!((logo_bmp.width, logo_bmp.height), (500, 500));

    let dr15_bmp = load_auto(&format!("{}/dr15.bmp", root)).expect("dr15.bmp");
    assert_eq!((dr15_bmp.width, dr15_bmp.height), (1024, 768));

    let banner_bmp = load_auto(&format!("{}/banner.bmp", root)).expect("banner.bmp");
    let banner_png = load_auto(&format!("{}/banner.png", root)).expect("banner.png");
    assert_eq!((banner_bmp.width, banner_bmp.height), (869, 287));
    assert_eq!((banner_png.width, banner_png.height), (869, 287));
    assert_eq!(banner_png.pixels.len(), banner_png.width * banner_png.height);

    let logo_png = load_auto(&format!("{}/logo.png", root)).expect("logo.png");
    assert_eq!((logo_png.width, logo_png.height), (500, 500));

    let terminal_png = load_auto(&format!("{}/terminal.png", root)).expect("terminal.png");
    assert_eq!((terminal_png.width, terminal_png.height), (270, 251));

    let text_png = load_auto(&format!("{}/text.png", root)).expect("text.png");
    assert_eq!((text_png.width, text_png.height), (271, 253));

    // Progressive JPEG (SOF2) is rejected cleanly without panicking.
    assert_eq!(load_auto(&format!("{}/boot.png", root)), Err(Error::Format));
}

#[test]
fn decodes_synthetic_bmp_and_minimal_jpeg() {
    // Minimal 2x1 24bpp BMP (green, blue)
    let mut bmp = vec![0u8; 54 + 8];
    bmp[0..2].copy_from_slice(b"BM");
    bmp[2..6].copy_from_slice(&(62u32).to_le_bytes());
    bmp[10..14].copy_from_slice(&(54u32).to_le_bytes());
    bmp[14..18].copy_from_slice(&(40u32).to_le_bytes());
    bmp[18..22].copy_from_slice(&(2i32).to_le_bytes());
    bmp[22..26].copy_from_slice(&(1i32).to_le_bytes());
    bmp[26..28].copy_from_slice(&(1u16).to_le_bytes());
    bmp[28..30].copy_from_slice(&(24u16).to_le_bytes());
    bmp[54..60].copy_from_slice(&[0x00, 0xFF, 0x00, 0xFF, 0x00, 0x00]); // BGR: green, blue
    let img = decode(&bmp).expect("24bpp bmp");
    assert_eq!((img.width, img.height), (2, 1));
    assert_eq!(img.pixels, vec![0xFF00_FF00, 0xFF00_00FF]);

    // Minimal valid 8x8 grayscale Baseline JPEG (all 128s -> mid-gray 0xFF808080)
    let jpeg: &[u8] = &[
        0xFF, 0xD8, // SOI
        0xFF, 0xDB, 0x00, 0x43, 0x00, // DQT id=0 8-bit
        1,1,1,1,1,1,1,1, 1,1,1,1,1,1,1,1,
        1,1,1,1,1,1,1,1, 1,1,1,1,1,1,1,1,
        1,1,1,1,1,1,1,1, 1,1,1,1,1,1,1,1,
        1,1,1,1,1,1,1,1, 1,1,1,1,1,1,1,1,
        0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x08, 0x00, 0x08, 0x01, 0x01, 0x11, 0x00, // SOF0 8x8 1-comp
        0xFF, 0xC4, 0x00, 0x14, 0x00, // DHT DC 0: 1 sym = 0x00
        1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00,
        0xFF, 0xC4, 0x00, 0x14, 0x10, // DHT AC 0: 1 sym = 0x00 (EOB)
        1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00,
        0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, // SOS
        0x00, // bit 0 (DC=0) + bit 0 (AC EOB) + padding
        0xFF, 0xD9, // EOI
    ];
    assert_eq!(codecs::sniff_format(jpeg), Some(ImageFormat::Jpeg));
    let jimg = decode(jpeg).expect("minimal baseline jpeg");
    assert_eq!((jimg.width, jimg.height), (8, 8));
    assert_eq!(jimg.pixels.len(), 64);
    assert!(jimg.pixels.iter().all(|&p| p == 0xFF80_8080));
}

#[test]
fn fuzz_malformed_images_never_panic() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let headers: [&[u8]; 3] = [b"BM", b"\x89PNG\r\n\x1a\n", b"\xFF\xD8\xFF\xE0"];
    for round in 0..2000usize {
        let mut buf = headers[round % 3].to_vec();
        let len = (next() % 192) as usize;
        for _ in 0..len {
            buf.push((next() & 0xFF) as u8);
        }
        let res = decode(&buf);
        if let Ok(img) = res {
            assert!(img.width > 0 && img.height > 0);
            assert_eq!(img.pixels.len(), img.width * img.height);
        }
    }
    assert_eq!(decode(b"garbage"), Err(Error::Format));
}
