use dunit_component::{Font, Renderer, Theme, View};
const DARK: &str = include_str!("../../../assets/dwm/sdk/dark.dss");
const LIGHT: &str = include_str!("../../../assets/dwm/sdk/light.dss");
const BASE: &str = include_str!("../../../assets/dwm/sdk/base.dss");
fn font() -> Font { Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap() }
#[test]
fn theme_change_repaints_full_canvas_and_restores_exact_pixels() {
    let dark = Theme::parse(DARK, BASE, "").unwrap();
    let light = Theme::parse(LIGHT, BASE, "").unwrap();
    assert_ne!(dark.tokens.color("text"), light.tokens.color("text"));
    let mut renderer = Renderer::new(font(), dark.cascade.clone(), dark.background);
    let view = View::<()>::column("root", vec![View::text("title", "Dunit SDK"), View::text("body", "Theme tokens")]);
    renderer.render(&view, 320, 160).unwrap();
    let before = renderer.pixels().to_vec();
    assert_eq!(renderer.render(&view,320,160).unwrap().damaged_pixels,0);
    renderer.set_theme(light, None);
    assert_eq!(renderer.render(&view,320,160).unwrap().damaged_pixels,320*160);
    assert!(renderer.pixels().iter().zip(&before).all(|(a,b)| a != b));
    renderer.set_theme(dark, Some(font()));
    renderer.render(&view,320,160).unwrap();
    assert_eq!(renderer.pixels(), before);
}
#[test]
fn malformed_profiles_and_wrong_token_types_are_rejected() {
    for source in [DARK.replace("$space: 12", "$space: -1"), DARK.replace("160ms", "bad"), DARK.replace("#121820", "#12182080"), DARK.replace("$text: #cdd6f4;", "")] {
        assert!(Theme::parse(&source,BASE,"").is_err());
    }
    assert!(Theme::parse(DARK,BASE,"Text { padding:4 $unknown; }").is_err());
    assert!(Theme::parse(&format!("{DARK} Column#root {{padding:-1;}}"),BASE,"").is_err());
    assert!(Theme::parse(DARK,BASE,"Text {font-size:bad;}").is_err());
}
