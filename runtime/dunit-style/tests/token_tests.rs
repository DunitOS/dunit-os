use dunit_style::{parse, parse_with_tokens, Color, Tokens, Value};
#[test]
fn registry_retains_expanded_types_and_ordered_aliases() {
    let sheet = parse("$bg:#123456; $gap:8px; $fast:0.2s; $inset:4 8; $alias:$bg; $bg:#ffffff;").unwrap();
    assert_eq!(sheet.tokens.color("alias"), Some(Color::rgb(0x12,0x34,0x56)));
    assert_eq!(sheet.tokens.color("bg"), Some(Color::rgb(255,255,255)));
    assert_eq!(sheet.tokens.number("gap"), Some(8.));
    assert_eq!(sheet.tokens.duration("fast"), Some(200.));
    assert_eq!(sheet.tokens.get("inset"), Some(Value::List(vec![Value::Number(4.), Value::Number(8.)])));
    assert_eq!(sheet.tokens.iter().count(), 5);
    assert_eq!(sheet.tokens.color("missing"), None);
    assert_eq!(sheet.tokens.number("bg"), None);
}
#[test]
fn inherited_layers_are_strict_and_do_not_mutate_the_profile() {
    let profile = parse("$fg:#123456;").unwrap();
    let layer = parse_with_tokens("Text {color:$fg;} $local:12;", &profile.tokens).unwrap();
    assert_eq!(layer.rules[0].declarations[0].value, Value::Color(Color::rgb(0x12,0x34,0x56)));
    assert_eq!(profile.tokens.get("local"), None);
    for source in ["Text {padding:8 $missing;}", "$a:$b; $b:$a;", "$a:$a;"] {
        assert!(parse_with_tokens(source, &Tokens::default()).is_err(), "{source}");
    }
}
