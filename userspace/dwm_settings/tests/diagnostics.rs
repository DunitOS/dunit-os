use dwm_settings::{validate, validate_document, Config};
#[test]
fn diagnostics_are_opt_in_and_resolved_from_the_validated_document() {
    assert!(!Config::defaults().diagnostics.performance);
    let base = "[application.test]\nname=\"Test\"\nexec=\"/app/test\"\n[workspaces]\ncount=1\n";
    assert!(!validate_document(base).unwrap().diagnostics.performance);
    for (text, expected) in [("true", true), ("false", false), ("1", true), ("0", false)] {
        let source = format!("{base}[diagnostics]\nperformance={text}\n");
        let config = validate_document(&source).unwrap();
        assert!(config.valid);
        assert_eq!(config.diagnostics.performance, expected);
    }
}
#[test]
fn malformed_or_duplicate_diagnostics_reject_the_whole_candidate() {
    for body in [
        "performance=maybe",
        "performance=\"true\"",
        "unknown=true",
        "performance=true\nperformance=false",
    ] {
        assert!(
            validate(&format!("[diagnostics]\n{body}")).is_err(),
            "{body}"
        );
    }
    assert!(validate("[diagnostics]\nperformance=true\n[diagnostics]\nperformance=false").is_err());
}
