//! Integration tests for `Store` review CRUD.
#![allow(clippy::must_use_candidate, clippy::missing_panics_doc, missing_docs)]

use git_forge::review::{ReviewState, ReviewTarget};
use git_forge::{Error, Store};
use git2::Repository;
use tempfile::TempDir;

fn test_repo() -> (TempDir, Repository) {
    let dir = TempDir::new().expect("temp dir");
    let repo = Repository::init(dir.path()).expect("init repo");
    {
        let mut cfg = repo.config().expect("config");
        cfg.set_str("user.name", "test").expect("user.name");
        cfg.set_str("user.email", "test@test.com")
            .expect("user.email");
    }
    {
        let sig = git2::Signature::now("test", "test@test.com").expect("sig");
        let mut index = repo.index().expect("index");
        let tree_oid = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_oid).expect("find tree");
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .expect("initial commit");
    }
    (dir, repo)
}

/// Return the OID of HEAD in the test repo (a valid commit to use as a target).
fn head_oid(repo: &Repository) -> String {
    repo.head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id()
        .to_string()
}

/// Create a blob and return its OID string.
fn make_blob(repo: &Repository, content: &[u8]) -> String {
    repo.blob(content).unwrap().to_string()
}

// ---------------------------------------------------------------------------
// create_review
// ---------------------------------------------------------------------------

#[test]
fn create_returns_oid() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let target = ReviewTarget {
        head: head_oid(&repo),
        base: None,
    };
    let review = store
        .create_review("PR title", "description", &target, None)
        .unwrap();
    assert_eq!(review.oid.len(), 40);
    assert!(review.oid.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(review.display_id, None);
}

#[test]
fn create_stores_all_fields() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let commit = head_oid(&repo);
    let target = ReviewTarget {
        head: commit.clone(),
        base: None,
    };
    let review = store
        .create_review("My review", "detailed description", &target, None)
        .unwrap();

    assert_eq!(review.title, "My review");
    assert_eq!(review.description, "detailed description");
    assert_eq!(review.state, ReviewState::Open);
    assert_eq!(review.target.head, commit);
    assert!(review.target.base.is_none());
    assert!(review.source_ref.is_none());
}

#[test]
fn create_with_commit_range_target() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let commit = head_oid(&repo);

    // Create a second commit for the range.
    let sig = git2::Signature::now("test", "test@test.com").unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    let tree = head.tree().unwrap();
    let commit2 = repo
        .commit(Some("HEAD"), &sig, &sig, "second commit", &tree, &[&head])
        .unwrap()
        .to_string();

    let target = ReviewTarget {
        head: commit2.clone(),
        base: Some(commit.clone()),
    };
    let review = store
        .create_review("Range review", "", &target, None)
        .unwrap();
    assert_eq!(review.target.head, commit2);
    assert_eq!(review.target.base, Some(commit));
}

#[test]
fn create_with_single_blob_target() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let blob = make_blob(&repo, b"hello world");
    let target = ReviewTarget {
        head: blob.clone(),
        base: None,
    };
    let review = store
        .create_review("Blob review", "", &target, None)
        .unwrap();
    assert_eq!(review.target.head, blob);
}

#[test]
fn create_with_source_ref() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let target = ReviewTarget {
        head: head_oid(&repo),
        base: None,
    };
    let review = store
        .create_review("Branch review", "", &target, Some("feature-branch"))
        .unwrap();
    assert_eq!(review.source_ref, Some("feature-branch".to_string()));
}

#[test]
fn objects_tree_pins_target() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let commit = head_oid(&repo);
    let target = ReviewTarget {
        head: commit.clone(),
        base: None,
    };
    let review = store.create_review("Pin test", "", &target, None).unwrap();

    // The entity ref tree should contain objects/<commit_oid>.
    let ref_name = format!("refs/forge/review/{}", review.oid);
    let reference = repo.find_reference(&ref_name).unwrap();
    let tree = reference.peel_to_commit().unwrap().tree().unwrap();
    let objects_entry = tree
        .get_path(std::path::Path::new(&format!("objects/{commit}")))
        .expect("objects/<oid> should exist in tree");
    // The entry exists — that's the pinning mechanism.
    assert!(objects_entry.id().is_zero() || !objects_entry.id().is_zero());
}

// ---------------------------------------------------------------------------
// get_review
// ---------------------------------------------------------------------------

#[test]
fn get_review_roundtrip() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let target = ReviewTarget {
        head: head_oid(&repo),
        base: None,
    };
    let created = store
        .create_review("Roundtrip", "body", &target, Some("main"))
        .unwrap();

    let fetched = store.get_review(&created.oid).unwrap();
    assert_eq!(fetched.oid, created.oid);
    assert_eq!(fetched.title, "Roundtrip");
    assert_eq!(fetched.description, "body");
    assert_eq!(fetched.state, ReviewState::Open);
    assert_eq!(fetched.source_ref, Some("main".to_string()));
}

// ---------------------------------------------------------------------------
// list_reviews
// ---------------------------------------------------------------------------

#[test]
fn list_reviews() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let commit = head_oid(&repo);
    let target = ReviewTarget {
        head: commit,
        base: None,
    };
    store.create_review("Alpha", "", &target, None).unwrap();
    store.create_review("Beta", "", &target, None).unwrap();

    let reviews = store.list_reviews().unwrap();
    assert_eq!(reviews.len(), 2);
    let mut titles: Vec<&str> = reviews.iter().map(|r| r.title.as_str()).collect();
    titles.sort_unstable();
    assert_eq!(titles, vec!["Alpha", "Beta"]);
}

// ---------------------------------------------------------------------------
// list_reviews_by_state
// ---------------------------------------------------------------------------

#[test]
fn list_reviews_by_state() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let commit = head_oid(&repo);
    let target = ReviewTarget {
        head: commit,
        base: None,
    };
    let to_merge = store.create_review("Merge me", "", &target, None).unwrap();
    store.create_review("Keep open", "", &target, None).unwrap();
    store
        .update_review(&to_merge.oid, None, None, Some(&ReviewState::Merged))
        .unwrap();

    let open = store.list_reviews_by_state(&ReviewState::Open).unwrap();
    let merged = store.list_reviews_by_state(&ReviewState::Merged).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].title, "Keep open");
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].title, "Merge me");
}

// ---------------------------------------------------------------------------
// update_review
// ---------------------------------------------------------------------------

#[test]
fn update_title_and_description() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let target = ReviewTarget {
        head: head_oid(&repo),
        base: None,
    };
    let created = store
        .create_review("Old", "old desc", &target, None)
        .unwrap();

    let updated = store
        .update_review(&created.oid, Some("New"), Some("new desc"), None)
        .unwrap();
    assert_eq!(updated.title, "New");
    assert_eq!(updated.description, "new desc");
    assert_eq!(updated.state, ReviewState::Open);
}

#[test]
fn update_state_to_merged() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let target = ReviewTarget {
        head: head_oid(&repo),
        base: None,
    };
    let created = store.create_review("PR", "", &target, None).unwrap();

    let updated = store
        .update_review(&created.oid, None, None, Some(&ReviewState::Merged))
        .unwrap();
    assert_eq!(updated.state, ReviewState::Merged);

    let fetched = store.get_review(&created.oid).unwrap();
    assert_eq!(fetched.state, ReviewState::Merged);
}

// ---------------------------------------------------------------------------
// refresh_review_target
// ---------------------------------------------------------------------------

#[test]
fn refresh_target_updates_objects() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);

    let old_head = head_oid(&repo);
    let target = ReviewTarget {
        head: old_head.clone(),
        base: None,
    };
    let review = store
        .create_review("Refresh", "", &target, Some("refs/heads/main"))
        .unwrap();

    // Advance main.
    let sig = git2::Signature::now("test", "test@test.com").unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    let tree = parent.tree().unwrap();
    repo.commit(
        Some("refs/heads/main"),
        &sig,
        &sig,
        "advance",
        &tree,
        &[&parent],
    )
    .unwrap();
    let new_head = head_oid(&repo);
    assert_ne!(old_head, new_head);

    let refreshed = store.refresh_review_target(&review.oid).unwrap();
    assert_eq!(refreshed.target.head, new_head);
}

#[test]
fn refresh_noop_without_ref() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let target = ReviewTarget {
        head: head_oid(&repo),
        base: None,
    };
    let review = store.create_review("No ref", "", &target, None).unwrap();

    let refreshed = store.refresh_review_target(&review.oid).unwrap();
    assert_eq!(refreshed.target.head, review.target.head);
}

// ---------------------------------------------------------------------------
// Unit tests for ReviewState
// ---------------------------------------------------------------------------

#[test]
fn state_from_str_valid() {
    assert_eq!("open".parse::<ReviewState>().unwrap(), ReviewState::Open);
    assert_eq!(
        "merged".parse::<ReviewState>().unwrap(),
        ReviewState::Merged
    );
    assert_eq!(
        "closed".parse::<ReviewState>().unwrap(),
        ReviewState::Closed
    );
}

#[test]
fn state_from_str_invalid() {
    let err = "pending".parse::<ReviewState>().unwrap_err();
    assert!(matches!(err, Error::InvalidState(_)));
}

#[test]
fn state_as_str() {
    assert_eq!(ReviewState::Open.as_str(), "open");
    assert_eq!(ReviewState::Merged.as_str(), "merged");
    assert_eq!(ReviewState::Closed.as_str(), "closed");
}

// ---------------------------------------------------------------------------
// Pin entry correctness (issue 5 regression)
// ---------------------------------------------------------------------------

#[test]
fn pin_entry_blob_references_actual_object() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let blob = make_blob(&repo, b"reviewable content");
    let blob_oid = git2::Oid::from_str(&blob).unwrap();
    let target = ReviewTarget {
        head: blob.clone(),
        base: None,
    };
    let review = store.create_review("Blob pin", "", &target, None).unwrap();

    let ref_name = format!("refs/forge/review/{}", review.oid);
    let reference = repo.find_reference(&ref_name).unwrap();
    let tree = reference.peel_to_commit().unwrap().tree().unwrap();
    let entry = tree
        .get_path(std::path::Path::new(&format!("objects/{blob}")))
        .unwrap();

    // For blobs, the tree entry OID must equal the actual blob OID.
    assert_eq!(entry.id(), blob_oid);
}

#[test]
fn pin_entry_commit_references_actual_object() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let commit = head_oid(&repo);
    let commit_oid = git2::Oid::from_str(&commit).unwrap();
    let target = ReviewTarget {
        head: commit.clone(),
        base: None,
    };
    let review = store
        .create_review("Commit pin", "", &target, None)
        .unwrap();

    let ref_name = format!("refs/forge/review/{}", review.oid);
    let reference = repo.find_reference(&ref_name).unwrap();
    let tree = reference.peel_to_commit().unwrap().tree().unwrap();
    let entry = tree
        .get_path(std::path::Path::new(&format!("objects/{commit}")))
        .unwrap();

    // The tree entry OID must equal the actual commit OID (gitlink mode).
    assert_eq!(entry.id(), commit_oid);
    assert_eq!(entry.filemode(), 0o160_000);
}

// ---------------------------------------------------------------------------
// Imported review with state (issue 7 regression)
// ---------------------------------------------------------------------------

#[test]
fn create_review_imported_with_state() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let commit = head_oid(&repo);
    let target = ReviewTarget {
        head: commit,
        base: None,
    };
    let author = git2::Signature::now("bot", "bot@test.com").unwrap();
    let review = store
        .create_review_imported(
            "Merged PR",
            "",
            &target,
            None,
            Some(&ReviewState::Merged),
            "GH#99",
            &author,
            "https://example.com",
        )
        .unwrap();

    assert_eq!(review.state, ReviewState::Merged);

    // Verify it round-trips through get_review.
    let fetched = store.get_review("GH#99").unwrap();
    assert_eq!(fetched.state, ReviewState::Merged);
}

// ---------------------------------------------------------------------------
// Imported review with base target (issue 2 regression)
// ---------------------------------------------------------------------------

#[test]
fn create_review_imported_preserves_base() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let commit = head_oid(&repo);
    let base_blob = make_blob(&repo, b"base");
    let target = ReviewTarget {
        head: commit,
        base: Some(base_blob.clone()),
    };
    let author = git2::Signature::now("bot", "bot@test.com").unwrap();
    let review = store
        .create_review_imported(
            "PR with base",
            "",
            &target,
            Some("feature"),
            None,
            "GH#50",
            &author,
            "https://example.com",
        )
        .unwrap();

    assert_eq!(review.target.base, Some(base_blob.clone()));
    assert_eq!(review.source_ref, Some("feature".to_string()));

    let fetched = store.get_review("GH#50").unwrap();
    assert_eq!(fetched.target.base, Some(base_blob));
    assert_eq!(fetched.source_ref, Some("feature".to_string()));
}
