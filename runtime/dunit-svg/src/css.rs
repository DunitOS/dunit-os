//! Small static stylesheet profile: class selectors and presentation properties.
use crate::{Error, Result};
use roxmltree::Node;
pub fn editor(n: Node<'_, '_>) -> bool {
    matches!(
        n.tag_name().namespace(),
        Some(
            "http://www.inkscape.org/namespaces/inkscape"
                | "http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd"
        )
    )
}
pub fn metadata(n: Node<'_, '_>) -> bool {
    editor(n) || matches!(n.tag_name().name(), "metadata" | "namedview")
}
pub fn rules(source: &str) -> impl Iterator<Item = Result<(&str, &str)>> {
    struct Rules<'a>(&'a str);
    impl<'a> Iterator for Rules<'a> {
        type Item = Result<(&'a str, &'a str)>;
        fn next(&mut self) -> Option<Self::Item> {
            loop {
                self.0 = self.0.trim();
                if self.0.is_empty() {
                    return None;
                }
                if !self.0.starts_with("/*") {
                    break;
                }
                match self.0.find("*/") {
                    Some(p) => self.0 = &self.0[p + 2..],
                    None => {
                        self.0 = "";
                        return Some(Err(Error::Invalid));
                    }
                }
            }
            let Some((selector, tail)) = self.0.split_once('{') else {
                self.0 = "";
                return Some(Err(Error::Invalid));
            };
            let Some((declarations, rest)) = tail.split_once('}') else {
                self.0 = "";
                return Some(Err(Error::Invalid));
            };
            self.0 = rest;
            let selector = selector.trim();
            if !selector.starts_with('.')
                || selector.len() < 2
                || !selector[1..]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
            {
                return Some(Err(Error::Unsupported));
            }
            Some(Ok((&selector[1..], declarations)))
        }
    }
    Rules(source)
}
pub fn stylesheet(n: Node<'_, '_>) -> bool {
    n.is_element()
        && n.tag_name().name() == "style"
        && n.ancestors()
            .skip(1)
            .all(|p| !p.is_element() || !metadata(p))
}
pub fn property<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    let classes = n.attribute("class")?;
    let mut value = None;
    for sheet in n.document().descendants().filter(|n| stylesheet(*n)) {
        for (selector, body) in rules(sheet.text().unwrap_or("")).flatten() {
            if classes.split_ascii_whitespace().any(|c| c == selector) {
                for (k, v) in body.split(';').filter_map(|s| s.split_once(':')) {
                    if k.trim() == name {
                        value = Some(v.trim());
                    }
                }
            }
        }
    }
    value
}
