use super::*;

/// The check to run after adding a theme or a role: every bundled theme
/// parses, gives every role a color, and names nothing that is not a role.
#[test]
fn every_bundled_theme_gives_every_role_a_color() {
    let mut ids = Vec::new();
    let mut base = None;
    for &(id, json) in BUNDLED {
        let (theme, problems) = Theme::parse(id, json, base.as_ref())
            .unwrap_or_else(|error| panic!("themes/{id}.json does not parse: {error}"));
        assert!(
            problems.is_empty(),
            "themes/{id}.json:\n  {}",
            problems.join("\n  ")
        );
        assert!(
            !theme.name.trim().is_empty(),
            "themes/{id}.json has no name"
        );
        assert!(!ids.contains(&id), "theme id `{id}` is bundled twice");
        ids.push(id);
        base.get_or_insert(theme.tokens);
    }
}

#[test]
fn a_theme_falls_back_on_the_default_for_roles_it_gets_wrong() {
    let (default, _) = Theme::parse(BUNDLED[0].0, BUNDLED[0].1, None).unwrap();
    let json = r##"{ "name": "Odd", "appearance": "light", "colors": {
        "text": "nope", "accent": "#ff0000", "glow": "#00ff00"
    } }"##;
    let (theme, problems) = Theme::parse("odd", json, Some(&default.tokens)).unwrap();
    assert!(problems.contains(&"`glow` is not a color role".to_owned()));
    assert!(problems.contains(&"`text`: \"nope\" is not a color".to_owned()));
    assert!(problems.contains(&"`surface` is missing".to_owned()));
    let color = |tokens: &Tokens, role: Role| tokens.color(role.name()).unwrap();
    assert_eq!(color(&theme.tokens, TEXT), color(&default.tokens, TEXT));
    assert_eq!(
        color(&theme.tokens, SURFACE),
        color(&default.tokens, SURFACE)
    );
    assert_ne!(color(&theme.tokens, ACCENT), color(&default.tokens, ACCENT));
    // NSH's own names follow the roles.
    assert_eq!(
        theme.tokens.color("primary"),
        Some(color(&theme.tokens, ACCENT))
    );
}
