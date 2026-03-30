use crate::comment::{Anchor, format_trailers, parse_trailers};

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
    // Mixed paragraph has an unknown key, so the whole block stays in body.
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
fn roundtrip_object_anchor_with_range() {
    let anchor = Anchor::Object {
        oid: "asdfhjkl".to_string(),
        path: None,
        range: Some("10-20".to_string()),
    };
    let trailers = format_trailers(Some(&anchor), false, None, None);
    let msg = format!("comment body\n\n{trailers}");
    let (body, parsed) = parse_trailers(&msg);
    assert_eq!(body, "comment body");
    assert_eq!(parsed.get("Anchor").unwrap(), "asdfhjkl");
    assert_eq!(parsed.get("Anchor-Range").unwrap(), "10-20");
    assert!(!parsed.contains_key("Anchor-End"));
}

#[test]
fn roundtrip_commit_range_anchor() {
    let anchor = Anchor::CommitRange {
        start: "aaa".to_string(),
        end: "bbb".to_string(),
    };
    let trailers = format_trailers(Some(&anchor), false, None, None);
    let msg = format!("comment body\n\n{trailers}");
    let (body, parsed) = parse_trailers(&msg);
    assert_eq!(body, "comment body");
    assert_eq!(parsed.get("Anchor").unwrap(), "aaa");
    assert_eq!(parsed.get("Anchor-End").unwrap(), "bbb");
    assert!(!parsed.contains_key("Anchor-Range"));
}

#[test]
fn roundtrip_all_trailers() {
    let anchor = Anchor::Object {
        oid: "asdfhjkl".to_string(),
        path: None,
        range: Some("5-10".to_string()),
    };
    let trailers = format_trailers(Some(&anchor), true, Some("orig123"), None);
    let msg = format!("multi-paragraph\n\nbody text\n\n{trailers}");
    let (body, parsed) = parse_trailers(&msg);
    assert_eq!(body, "multi-paragraph\n\nbody text");
    assert_eq!(parsed.get("Anchor").unwrap(), "asdfhjkl");
    assert_eq!(parsed.get("Anchor-Range").unwrap(), "5-10");
    assert_eq!(parsed.get("Resolved").unwrap(), "true");
    assert_eq!(parsed.get("Replaces").unwrap(), "orig123");
}

#[test]
fn roundtrip_no_trailers() {
    let trailers = format_trailers(None, false, None, None);
    assert!(trailers.is_empty());
    let (body, parsed) = parse_trailers("just a body");
    assert_eq!(body, "just a body");
    assert!(parsed.is_empty());
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
fn roundtrip_object_anchor_with_path() {
    let anchor = Anchor::Object {
        oid: "abc123".to_string(),
        path: Some("src/main.rs".to_string()),
        range: Some("42-47".to_string()),
    };
    let trailers = format_trailers(Some(&anchor), false, None, None);
    let msg = format!("body\n\n{trailers}");
    let (body, parsed) = parse_trailers(&msg);
    assert_eq!(body, "body");
    assert_eq!(parsed.get("Anchor").unwrap(), "abc123");
    assert_eq!(parsed.get("Anchor-Path").unwrap(), "src/main.rs");
    assert_eq!(parsed.get("Anchor-Range").unwrap(), "42-47");
}

#[test]
fn roundtrip_object_anchor_no_range() {
    let anchor = Anchor::Object {
        oid: "asdfhjkl".to_string(),
        path: None,
        range: None,
    };
    let trailers = format_trailers(Some(&anchor), false, None, None);
    let msg = format!("body\n\n{trailers}");
    let (body, parsed) = parse_trailers(&msg);
    assert_eq!(body, "body");
    assert_eq!(parsed.get("Anchor").unwrap(), "asdfhjkl");
    assert!(!parsed.contains_key("Anchor-Range"));
}

#[test]
fn parse_trailers_migrated_from() {
    let msg = "migrated comment\n\nMigrated-From: deadbeef";
    let (body, trailers) = parse_trailers(msg);
    assert_eq!(body, "migrated comment");
    assert_eq!(trailers.get("Migrated-From").unwrap(), "deadbeef");
}

#[test]
fn roundtrip_migrated_from_with_anchor() {
    let anchor = Anchor::Object {
        oid: "newblob".to_string(),
        path: Some("src/lib.rs".to_string()),
        range: Some("22-25".to_string()),
    };
    let trailers = format_trailers(Some(&anchor), false, None, Some("oldcomment123"));
    let msg = format!("body\n\n{trailers}");
    let (body, parsed) = parse_trailers(&msg);
    assert_eq!(body, "body");
    assert_eq!(parsed.get("Anchor").unwrap(), "newblob");
    assert_eq!(parsed.get("Anchor-Path").unwrap(), "src/lib.rs");
    assert_eq!(parsed.get("Anchor-Range").unwrap(), "22-25");
    assert_eq!(parsed.get("Migrated-From").unwrap(), "oldcomment123");
}
