use crate::comment::parse_trailers;

#[test]
fn parse_trailers_no_trailers() {
    let (body, trailers) = parse_trailers("just a plain body");
    assert_eq!(body, "just a plain body");
    assert!(trailers.is_empty());
}

#[test]
fn parse_trailers_known_keys() {
    let msg = "comment body\n\nAnchor: asdfhjkl\nAnchor-Range: 10-20";
    let (body, trailers) = parse_trailers(msg);
    assert_eq!(body, "comment body");
    assert_eq!(trailers.get("Anchor").unwrap(), "asdfhjkl");
    assert_eq!(trailers.get("Anchor-Range").unwrap(), "10-20");
}

#[test]
fn parse_trailers_only_trailers() {
    let msg = "Resolved: true";
    let (body, trailers) = parse_trailers(msg);
    assert!(body.is_empty());
    assert_eq!(trailers.get("Resolved").unwrap(), "true");
}

#[test]
fn parse_trailers_unknown_key_stays_in_body() {
    let msg = "comment body\n\nSigned-off-by: someone@example.com";
    let (body, trailers) = parse_trailers(msg);
    assert_eq!(body, "comment body\n\nSigned-off-by: someone@example.com");
    assert!(trailers.is_empty());
}

#[test]
fn parse_trailers_mixed_known_unknown_stays_in_body() {
    let msg = "body\n\nAnchor: asdfhjkl\nSigned-off-by: someone";
    let (body, trailers) = parse_trailers(msg);
    assert_eq!(body, "body\n\nAnchor: asdfhjkl\nSigned-off-by: someone");
    assert!(trailers.is_empty());
}

#[test]
fn parse_trailers_multiline_body_with_colons() {
    let msg = "This has Key: value-like text in it\n\nResolved: true";
    let (body, trailers) = parse_trailers(msg);
    assert_eq!(body, "This has Key: value-like text in it");
    assert_eq!(trailers.get("Resolved").unwrap(), "true");
}

#[test]
fn parse_trailers_github_id() {
    let msg = "imported comment\n\nGithub-Id: 42";
    let (body, trailers) = parse_trailers(msg);
    assert_eq!(body, "imported comment");
    assert_eq!(trailers.get("Github-Id").unwrap(), "42");
}

#[test]
fn parse_trailers_replaces() {
    let msg = "updated body\n\nReplaces: abc123";
    let (body, trailers) = parse_trailers(msg);
    assert_eq!(body, "updated body");
    assert_eq!(trailers.get("Replaces").unwrap(), "abc123");
}

#[test]
fn parse_trailers_colon_in_value() {
    let msg = "body\n\nAnchor: abc:def";
    let (body, trailers) = parse_trailers(msg);
    assert_eq!(body, "body");
    assert_eq!(trailers.get("Anchor").unwrap(), "abc:def");
}

#[test]
fn parse_trailers_multiple_paragraphs_body() {
    let msg = "first paragraph\n\nsecond paragraph\n\nAnchor: asdfhjkl";
    let (body, trailers) = parse_trailers(msg);
    assert_eq!(body, "first paragraph\n\nsecond paragraph");
    assert_eq!(trailers.get("Anchor").unwrap(), "asdfhjkl");
}

#[test]
fn parse_trailers_migrated_from() {
    let msg = "migrated comment\n\nMigrated-From: abc12340";
    let (body, trailers) = parse_trailers(msg);
    assert_eq!(body, "migrated comment");
    assert_eq!(trailers.get("Migrated-From").unwrap(), "abc12340");
}
