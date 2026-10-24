#![no_std]
#![no_main]
extern crate alloc;
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    libdunit::println("[SVG-TEST] PANIC");
    libdunit::exit(101)
}
fn check(ok: bool, label: &str) {
    if !ok {
        libdunit::println(label);
        libdunit::exit(1);
    }
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let gradient = r##"<svg viewBox="0 0 32 32"><defs><linearGradient id="g"><stop stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/></linearGradient></defs><rect width="32" height="32" fill="url(#g)" fill-opacity=".5"/></svg>"##;
    let p = dunit_svg::render(gradient, 64, 64).unwrap();
    check(p[32 * 64 + 2] >> 24 == 128, "[SVG-TEST] FAIL alpha");
    check(
        (p[32 * 64 + 2] >> 16) & 255 > 230 && p[32 * 64 + 61] & 255 > 230,
        "[SVG-TEST] FAIL gradient",
    );
    libdunit::println("[SVG-TEST] native gradients and straight alpha OK");
    let names = [
        "gui_demo",
        "gui_client",
        "gui_calc",
        "gui_files",
        "gui_terminal",
        "gui_stat",
        "gui_settings",
        "mime_folder",
        "mime_text",
        "mime_exec",
        "mime_image",
        "mime_archive",
        "mime_unknown",
    ];
    for name in names {
        for side in [24u32, 74, 256] {
            let image = dunit_image::icon("/assets/icons/aqua/", name, side).unwrap();
            check(
                image.width == side as usize
                    && image.height == side as usize
                    && image.pixels.iter().any(|p| p >> 24 != 0),
                "[SVG-TEST] FAIL vector size",
            );
        }
    }
    libdunit::println("[SVG-TEST] 13 VFS vectors at 24/74/256px OK");
    let bitmap = dunit_image::icon("/assets/icons/breeze/", "gui_files", 74).unwrap();
    check(
        bitmap.width == 32 && bitmap.pixels.len() == 1024,
        "[SVG-TEST] FAIL legacy fallback",
    );
    let wallpaper = dunit_image::load("/assets/wallpapers/mojave.svg", 480, 270).unwrap();
    check(
        wallpaper.pixels.len() == 480 * 270 && wallpaper.pixels.iter().all(|p| p >> 24 == 255),
        "[SVG-TEST] FAIL wallpaper",
    );
    let fd = libdunit::open(
        "/tmp/svg-rejected.svg",
        libdunit::OPEN_WRITE | libdunit::OPEN_CREATE | libdunit::OPEN_TRUNC,
    );
    check(fd >= 0, "[SVG-TEST] FAIL fixture open");
    let bad = b"<svg><script/></svg>";
    check(
        libdunit::write(fd as usize, bad) == bad.len() as isize,
        "[SVG-TEST] FAIL fixture write",
    );
    libdunit::close(fd as usize);
    let bitmap_fd = libdunit::open(
        "/tmp/svg-rejected.rgba",
        libdunit::OPEN_WRITE | libdunit::OPEN_CREATE | libdunit::OPEN_TRUNC,
    );
    check(bitmap_fd >= 0, "[SVG-TEST] FAIL bitmap fixture open");
    check(
        libdunit::write(bitmap_fd as usize, &[0u8; 4096]) == 4096,
        "[SVG-TEST] FAIL bitmap fixture write",
    );
    libdunit::close(bitmap_fd as usize);
    check(
        matches!(
            dunit_image::icon("/tmp/", "svg-rejected", 32),
            Err(dunit_image::Error::Svg(dunit_svg::Error::Unsupported))
        ),
        "[SVG-TEST] FAIL invalid SVG fallback",
    );
    check(
        dunit_svg::render(
            "<svg viewBox='0 0 32 32'><use id='x' href='#x'/></svg>",
            32,
            32,
        )
        .is_err(),
        "[SVG-TEST] FAIL cycle rejection",
    );
    libdunit::println("[SVG-TEST] bitmap compatibility, wallpaper and invalid SVG rejection OK");
    for theme in [
        "breeze-chameleon-dark",
        "breeze-chameleon-light",
        "breeze-round-chameleon-dark",
        "breeze-round-chameleon-light",
        "chameleon-symbolic-dark",
    ] {
        let mut source = dunit_image::IconTheme::open(theme).unwrap_or_else(|error| {
            libdunit::println(&alloc::format!(
                "[ICON-THEME] FAIL open {} {:?}",
                theme,
                error
            ));
            libdunit::exit(1)
        });
        for name in names {
            for side in [24u32, 32, 74] {
                let image = source.icon(name, side).unwrap_or_else(|error| {
                    libdunit::println(&alloc::format!(
                        "[ICON-THEME] FAIL {} {} {} {:?}",
                        theme,
                        name,
                        side,
                        error
                    ));
                    libdunit::exit(1)
                });
                check(
                    image.width == side as usize && image.pixels.iter().any(|p| p >> 24 != 0),
                    "[ICON-THEME] FAIL pixels",
                );
            }
        }
        let direct = source
            .icon("categories/32/applications-graphics.svg", 48)
            .unwrap();
        check(
            direct.pixels.iter().any(|p| p >> 24 != 0),
            "[ICON-THEME] FAIL explicit path",
        );
        check(
            matches!(
                source.icon("does-not-exist", 32),
                Err(dunit_image::Error::Missing)
            ),
            "[ICON-THEME] FAIL missing",
        );
        check(
            source.icon("../escape.svg", 32).is_err(),
            "[ICON-THEME] FAIL traversal",
        );
        libdunit::println(&alloc::format!(
            "[ICON-THEME] {} aliases, sizes, direct path OK",
            theme
        ));
    }
    let mut settings = dwm_settings::load();
    for preset in [
        "macos-light",
        "mojave-dark",
        "nord",
        "green-tea",
        "symbolic-dark",
    ] {
        settings = dwm_settings::load_overlay(
            &settings,
            &alloc::format!("/assets/dwm/themes/{}.toml", preset),
        )
        .unwrap();
        check(
            dwm_settings::roundtrip_ok(&settings),
            "[ICON-THEME] FAIL config roundtrip",
        );
        let mut source =
            dunit_image::IconTheme::open(settings.desktop.icon_theme.as_str()).unwrap();
        check(
            source
                .icon("gui_files", 32)
                .unwrap()
                .pixels
                .iter()
                .any(|p| p >> 24 != 0),
            "[ICON-THEME] FAIL config selection",
        );
    }
    libdunit::println("[ICON-THEME] five sequential presets and config roundtrips OK");
    let path = "/tmp/icon-theme-cycle.toml";
    let fd = libdunit::open(
        path,
        libdunit::OPEN_WRITE | libdunit::OPEN_CREATE | libdunit::OPEN_TRUNC,
    );
    check(fd >= 0, "[ICON-THEME] FAIL manifest fixture");
    let text =
        b"[theme]\nroot = \"/assets/icons/aqua/\"\nfallback = \"/tmp/icon-theme-cycle.toml\"\n";
    check(
        libdunit::write(fd as usize, text) == text.len() as isize,
        "[ICON-THEME] FAIL manifest write",
    );
    libdunit::close(fd as usize);
    check(
        dunit_image::IconTheme::open(path).is_err(),
        "[ICON-THEME] FAIL fallback cycle",
    );

    libdunit::exit(0)
}
