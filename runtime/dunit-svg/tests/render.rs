use dunit_svg::{render, Error};
fn svg(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">{body}</svg>"#)
}
fn pixel(p: &[u32], x: usize, y: usize, w: usize) -> u32 {
    p[y * w + x]
}

#[test]
fn aspect_ratio_and_nonzero_viewbox_origin() {
    let p=render(r##"<svg viewBox="10 20 20 20"><rect x="10" y="20" width="20" height="20" fill="#ff0000"/></svg>"##,80,40).unwrap();
    assert_eq!(pixel(&p, 0, 20, 80), 0);
    assert_eq!(pixel(&p, 40, 20, 80), 0xffff0000);
    assert_eq!(pixel(&p, 79, 20, 80), 0);
}
#[test]
fn transparent_fill_is_straight_alpha() {
    let p = render(
        &svg(r##"<rect width="64" height="64" fill="#4080c0" fill-opacity=".5"/>"##),
        32,
        32,
    )
    .unwrap();
    let c = p[16 * 32 + 16];
    assert!((126..=129).contains(&(c >> 24)));
    assert!((62..=66).contains(&((c >> 16) & 255)));
    assert!((126..=130).contains(&((c >> 8) & 255)));
    assert!((190..=194).contains(&(c & 255)));
}
#[test]
fn opacity_composites_group_once() {
    let p=render(&svg(r#"<g opacity=".5"><rect width="50" height="50" fill="red"/><rect x="20" y="20" width="44" height="44" fill="blue"/></g>"#),64,64).unwrap();
    assert!((126..=129).contains(&(pixel(&p, 30, 30, 64) >> 24)));
    assert_eq!(pixel(&p, 30, 30, 64) & 0xffffff, 0xff);
}
#[test]
fn linear_gradient_and_transform() {
    let p=render(&svg(r##"<defs><linearGradient id="g"><stop stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/></linearGradient></defs><g transform="translate(8 8)"><rect width="48" height="48" fill="url(#g)"/></g>"##),64,64).unwrap();
    assert_eq!(p[0], 0);
    let left = pixel(&p, 10, 32, 64);
    let right = pixel(&p, 54, 32, 64);
    assert!(((left >> 16) & 255) > 220 && (left & 255) < 40);
    assert!((right & 255) > 220 && ((right >> 16) & 255) < 40);
}
#[test]
fn curves_arcs_strokes_and_scaling() {
    let source = svg(
        r#"<path d="M12 32a20 20 0 1 1 40 0s-10 20-20 20q-20 0-20-20z" fill="none" stroke="white" stroke-width="3"/>"#,
    );
    let a = render(&source, 32, 32).unwrap();
    let b = render(&source, 256, 256).unwrap();
    assert!(a.iter().any(|p| *p >> 24 > 0));
    assert!(
        b.iter().filter(|p| **p >> 24 > 0).count() > a.iter().filter(|p| **p >> 24 > 0).count() * 8
    );
    assert!(b.iter().any(|p| (1..254).contains(&(*p >> 24))));
}
#[test]
fn current_color_inheritance_and_local_use() {
    let p=render(&svg(r##"<defs><rect id="box" width="16" height="16"/></defs><g color="#00ff00" fill="currentColor"><use href="#box" x="20" y="20"/></g>"##),64,64).unwrap();
    assert_eq!(pixel(&p, 25, 25, 64), 0xff00ff00);
    assert_eq!(pixel(&p, 2, 2, 64), 0);
}
#[test]
fn no_active_or_external_content() {
    for body in [
        "<script>alert(1)</script>",
        "<image href='https://invalid/x.png'/>",
        "<foreignObject/>",
        "<rect width='5' height='5' filter='url(#x)'/>",
        "<use href='https://invalid/icon.svg#x'/>",
    ] {
        assert!(render(&svg(body), 32, 32).is_err(), "{body}");
    }
    assert!(render("<!DOCTYPE svg [<!ENTITY x 'oops'>]><svg>&x;</svg>", 32, 32).is_err());
}
#[test]
fn malformed_inputs_and_resource_limits() {
    assert_eq!(render("<svg", 32, 32), Err(Error::Xml));
    assert_eq!(
        render(&svg("<path d='M0 0 L nope'/>"), 32, 32),
        Err(Error::Invalid)
    );
    assert_eq!(
        render(&svg("<use id='a' href='#a'/>"), 32, 32),
        Err(Error::Limit)
    );
    assert_eq!(
        render(&svg("<path d='M0 0L1e100 3'/>"), 32, 32),
        Err(Error::Invalid)
    );
    assert_eq!(render(&svg(""), 0, 32), Err(Error::Limit));
    assert_eq!(render(&svg(""), 8192, 8192), Err(Error::Limit));
    let nested = format!("{}{}", "<g>".repeat(40), "</g>".repeat(40));
    assert_eq!(render(&svg(&nested), 32, 32), Err(Error::Limit));
}
#[test]
fn shipped_vectors_render_without_conversion() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons/aqua");
    let mut count = 0;
    for e in std::fs::read_dir(root).unwrap() {
        let path = e.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("svg") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for side in [24, 32, 74, 256] {
            let p =
                render(&text, side, side).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
            assert_eq!(p.len(), side as usize * side as usize);
            assert!(p.iter().any(|p| p >> 24 != 0));
        }
        count += 1;
    }
    assert_eq!(count, 13);
    let paper = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/wallpapers/mojave.svg"),
    )
    .unwrap();
    assert!(render(&paper, 480, 270)
        .unwrap()
        .iter()
        .all(|p| p >> 24 == 255));
}

#[test]
fn malformed_polygon_rejects_the_entire_document() {
    assert_eq!(
        render(&svg("<polygon points='0,0 4,bad'/>"), 32, 32),
        Err(Error::Invalid)
    );
    assert_eq!(
        render(&svg("<polygon points='0,0 4'/>"), 32, 32),
        Err(Error::Invalid)
    );
}
#[test]
fn local_paint_fallback_and_radial_gradient() {
    let p = render(
        &svg(r##"<rect width="64" height="64" fill="url(#missing) #00ff00"/>"##),
        32,
        32,
    )
    .unwrap();
    assert_eq!(p[16 * 32 + 16], 0xff00ff00);
    let p=render(&svg(r##"<defs><radialGradient id="r"><stop stop-color="#ffffff"/><stop offset="1" stop-color="#000000"/></radialGradient></defs><rect width="64" height="64" fill="url(#r)"/>"##),64,64).unwrap();
    assert!(p[32 * 64 + 32] & 255 > 240);
    assert!(p[32 * 64] & 255 < 20);
}

#[test]
fn editor_metadata_css_and_linked_gradients() {
    let source = r##"<svg xmlns:i="http://www.inkscape.org/namespaces/inkscape" viewBox="0 0 32 32" i:version="1"><metadata><arbitrary>editor data</arbitrary></metadata><i:namedview/><defs><style>.Accent {color:#00ff00;} .Accent {color:#0080ff;}</style><linearGradient id="a"><stop stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/></linearGradient><linearGradient id="b" href="#a" x2="0" y2="1"/></defs><rect width="16" height="32" class="Accent" fill="currentColor"/><rect x="16" width="16" height="32" fill="url(#b)"/></svg>"##;
    let p = render(source, 32, 32).unwrap();
    assert_eq!(p[16 * 32 + 8], 0xff0080ff);
    assert!((p[2 * 32 + 24] >> 16) & 255 > 220);
    assert!(p[30 * 32 + 24] & 255 > 220);
    assert!(render(&svg(r##"<defs><linearGradient id="a" href="#b"/><linearGradient id="b" href="#a"/></defs><rect width="64" height="64" fill="url(#a)"/>"##),32,32).is_err());
    assert!(render(
        &svg("<style>path:hover {fill:red;}</style><path d='M0 0L3 3'/ >"),
        32,
        32
    )
    .is_err());
    assert!(render(
        &svg("<rect width='64' height='64' style='mix-blend-mode:multiply'/>"),
        32,
        32
    )
    .is_err());
}
#[test]
fn gaussian_region_alpha_and_paint_order() {
    let p=render(&svg(r##"<defs><filter id="b" x="-1" y="-1" width="3" height="3" color-interpolation-filters="sRGB"><feGaussianBlur stdDeviation="2"/></filter></defs><rect x="24" y="24" width="16" height="16" fill="#ff0000" filter="url(#b)"/>"##),64,64).unwrap();
    assert!(p[32 * 64 + 32] >> 24 > 240);
    assert!((1..100).contains(&(p[32 * 64 + 21] >> 24)));
    assert_eq!((p[32 * 64 + 21] >> 16) & 255, 255);
    assert_eq!(p[0], 0);
    let source = r##"<rect x="16" y="16" width="32" height="32" fill="#ff0000" stroke="#0000ff" stroke-width="16" style="paint-order:stroke fill markers"/>"##;
    let p = render(&svg(source), 64, 64).unwrap();
    assert_eq!(p[32 * 64 + 19], 0xffff0000);
    assert_eq!(p[32 * 64 + 12], 0xff0000ff);
    let p=render(&svg(r##"<defs><filter id="b" x="0" y="0" width="1" height="1"><feGaussianBlur stdDeviation="2"/></filter></defs><rect x="24" y="24" width="16" height="16" filter="url(#b)"/>"##),64,64).unwrap();
    assert_eq!(p[32 * 64 + 23], 0);
    assert!(p[32 * 64 + 24] >> 24 > 0);
}
