use std::path::Path;

use tempfile::TempDir;

use crate::{
    env::{load_config, merge_trees, resolve_env, resolve_trees},
    import::{import_dir, import_tarball},
    store::Store,
};

fn temp_store() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let store = Store::open_or_init(dir.path()).unwrap();
    (dir, store)
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

#[test]
fn store_creates_dirs() {
    let (dir, _store) = temp_store();
    assert!(dir.path().join("blobs").exists());
    assert!(dir.path().join("store").exists());
    assert!(dir.path().join("runs").exists());
}

#[test]
fn store_write_blob_is_idempotent() {
    let (_dir, store) = temp_store();
    let oid1 = store.write_blob(b"hello").unwrap();
    let oid2 = store.write_blob(b"hello").unwrap();
    assert_eq!(oid1, oid2);
}

#[test]
fn store_list_trees_roundtrip() {
    let (_dir, store) = temp_store();
    let oid = store.write_blob(b"data").unwrap();
    // build a minimal tree containing the blob
    let repo = store.repo();
    let mut builder = repo.treebuilder(None).unwrap();
    builder.insert("file.txt", oid, 0o100_644).unwrap();
    let tree_oid = builder.write().unwrap();
    store.create_tree_ref(tree_oid).unwrap();

    let trees = store.list_trees().unwrap();
    assert!(trees.contains(&tree_oid));
}

#[test]
fn store_materialize_creates_hardlink_farm() {
    let (dir, store) = temp_store();

    let repo = store.repo();
    let blob_oid = repo.blob(b"content").unwrap();
    let mut builder = repo.treebuilder(None).unwrap();
    builder.insert("hello.txt", blob_oid, 0o100_644).unwrap();
    let tree_oid = builder.write().unwrap();

    let mat = store.materialize(tree_oid).unwrap();
    assert!(mat.starts_with(dir.path().join("store")));
    assert!(mat.join("hello.txt").exists());
    let got = std::fs::read_to_string(mat.join("hello.txt")).unwrap();
    assert_eq!(got, "content");
}

#[cfg(unix)]
#[test]
fn store_gc_removes_unreferenced_blobs() {

    let (_dir, store) = temp_store();
    let repo = store.repo();
    let blob_oid = repo.blob(b"orphan").unwrap();

    // Manually write blob to cache without hardlinks so nlink == 1.
    let cache = _dir.path().join("blobs").join(blob_oid.to_string());
    std::fs::write(&cache, b"orphan").unwrap();

    let removed = store.gc_blobs().unwrap();
    assert_eq!(removed, 1);
    assert!(!cache.exists());
}

// ---------------------------------------------------------------------------
// Import: dir
// ---------------------------------------------------------------------------

#[test]
fn import_dir_roundtrip() {
    let (_store_dir, store) = temp_store();
    let src = TempDir::new().unwrap();

    std::fs::write(src.path().join("a.txt"), b"alpha").unwrap();
    std::fs::write(src.path().join("b.txt"), b"beta").unwrap();

    let oid = import_dir(&store, src.path()).unwrap();

    let mat = store.materialize(oid).unwrap();
    assert_eq!(std::fs::read(mat.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(std::fs::read(mat.join("b.txt")).unwrap(), b"beta");
}

#[test]
fn import_dir_same_content_same_hash() {
    let (_dir, store) = temp_store();
    let src1 = TempDir::new().unwrap();
    let src2 = TempDir::new().unwrap();

    std::fs::write(src1.path().join("x"), b"data").unwrap();
    std::fs::write(src2.path().join("x"), b"data").unwrap();

    let oid1 = import_dir(&store, src1.path()).unwrap();
    let oid2 = import_dir(&store, src2.path()).unwrap();
    assert_eq!(oid1, oid2);
}

#[test]
fn import_dir_nested() {
    let (_dir, store) = temp_store();
    let src = TempDir::new().unwrap();

    std::fs::create_dir(src.path().join("sub")).unwrap();
    std::fs::write(src.path().join("sub").join("f"), b"nested").unwrap();

    let oid = import_dir(&store, src.path()).unwrap();
    let mat = store.materialize(oid).unwrap();
    assert_eq!(std::fs::read(mat.join("sub").join("f")).unwrap(), b"nested");
}

// ---------------------------------------------------------------------------
// Import: tarball
// ---------------------------------------------------------------------------

fn make_tar(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut ar = tar::Builder::new(&mut buf);
        for (name, content) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_mtime(0);
            header.set_cksum();
            ar.append_data(&mut header, name, *content).unwrap();
        }
        ar.finish().unwrap();
    }
    buf
}

#[test]
fn import_tarball_basic() {
    let (_dir, store) = temp_store();
    let tarball_dir = TempDir::new().unwrap();
    let tar_path = tarball_dir.path().join("test.tar");

    let data = make_tar(&[("root/a.txt", b"alpha"), ("root/b.txt", b"beta")]);
    std::fs::write(&tar_path, &data).unwrap();

    let oid = import_tarball(&store, &tar_path, 1).unwrap();
    let mat = store.materialize(oid).unwrap();
    assert_eq!(std::fs::read(mat.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(std::fs::read(mat.join("b.txt")).unwrap(), b"beta");
}

#[test]
fn import_tarball_strip_zero() {
    let (_dir, store) = temp_store();
    let tarball_dir = TempDir::new().unwrap();
    let tar_path = tarball_dir.path().join("test.tar");

    let data = make_tar(&[("file.txt", b"content")]);
    std::fs::write(&tar_path, &data).unwrap();

    let oid = import_tarball(&store, &tar_path, 0).unwrap();
    let mat = store.materialize(oid).unwrap();
    assert_eq!(std::fs::read(mat.join("file.txt")).unwrap(), b"content");
}

#[test]
fn import_tarball_deterministic() {
    let (_dir, store) = temp_store();
    let tarball_dir = TempDir::new().unwrap();
    let tar_path = tarball_dir.path().join("test.tar");

    let data = make_tar(&[("f", b"x")]);
    std::fs::write(&tar_path, &data).unwrap();

    let oid1 = import_tarball(&store, &tar_path, 0).unwrap();
    let oid2 = import_tarball(&store, &tar_path, 0).unwrap();
    assert_eq!(oid1, oid2);
}

// ---------------------------------------------------------------------------
// Env: config and composition
// ---------------------------------------------------------------------------

fn write_config(dir: &Path, content: &str) -> std::path::PathBuf {
    let path = dir.join("env.toml");
    std::fs::write(&path, content).unwrap();
    path
}

#[test]
fn env_load_config_basic() {
    let dir = TempDir::new().unwrap();
    let path = write_config(
        dir.path(),
        r#"
[env.default]
trees = ["a3f1c9d2b8e64f5a1c0d9e2f3b4a5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b"]
"#,
    );
    let cfg = load_config(&path).unwrap();
    assert!(cfg.env.contains_key("default"));
    assert_eq!(cfg.env["default"].trees.len(), 1);
}

#[test]
fn env_load_config_extends() {
    let dir = TempDir::new().unwrap();
    let path = write_config(
        dir.path(),
        r#"
[env.base]
trees = []

[env.derived]
extends = "base"
trees = []
"#,
    );
    let cfg = load_config(&path).unwrap();
    assert_eq!(cfg.env["derived"].extends.as_deref(), Some("base"));
}

#[test]
fn env_load_config_missing_file() {
    let dir = TempDir::new().unwrap();
    let result = load_config(&dir.path().join("missing.toml"));
    assert!(result.is_err());
}

#[test]
fn env_merge_trees_last_wins() {
    let (_dir, store) = temp_store();
    let repo = store.repo();

    // tree_a has a.txt = "a"
    let blob_a = repo.blob(b"a").unwrap();
    let mut b1 = repo.treebuilder(None).unwrap();
    b1.insert("a.txt", blob_a, 0o100_644).unwrap();
    let tree_a = b1.write().unwrap();

    // tree_b has a.txt = "b" (conflict: b wins)
    let blob_b = repo.blob(b"b").unwrap();
    let mut b2 = repo.treebuilder(None).unwrap();
    b2.insert("a.txt", blob_b, 0o100_644).unwrap();
    let tree_b = b2.write().unwrap();

    let merged = merge_trees(&store, &[tree_a, tree_b]).unwrap();

    let mat = store.materialize(merged).unwrap();
    assert_eq!(std::fs::read(mat.join("a.txt")).unwrap(), b"b");
}

#[test]
fn env_merge_trees_additive() {
    let (_dir, store) = temp_store();
    let repo = store.repo();

    let blob_a = repo.blob(b"alpha").unwrap();
    let mut b1 = repo.treebuilder(None).unwrap();
    b1.insert("a.txt", blob_a, 0o100_644).unwrap();
    let tree_a = b1.write().unwrap();

    let blob_b = repo.blob(b"beta").unwrap();
    let mut b2 = repo.treebuilder(None).unwrap();
    b2.insert("b.txt", blob_b, 0o100_644).unwrap();
    let tree_b = b2.write().unwrap();

    let merged = merge_trees(&store, &[tree_a, tree_b]).unwrap();
    let mat = store.materialize(merged).unwrap();
    assert_eq!(std::fs::read(mat.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(std::fs::read(mat.join("b.txt")).unwrap(), b"beta");
}

#[test]
fn env_resolve_env_creates_ref() {
    let (_dir, store) = temp_store();

    // Import a real tree so the hash is valid.
    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("f"), b"x").unwrap();
    let tree_oid = import_dir(&store, src.path()).unwrap();

    let cfg_dir = TempDir::new().unwrap();
    let cfg_path = write_config(
        cfg_dir.path(),
        &format!(
            "[env.myenv]\ntrees = [\"{tree_oid}\"]\n"
        ),
    );
    let cfg = load_config(&cfg_path).unwrap();
    let oid = resolve_env(&store, &cfg, "myenv").unwrap();

    let envs = store.list_envs().unwrap();
    assert!(envs.contains(&oid));
}

#[test]
fn env_circular_extends_is_error() {
    let dir = TempDir::new().unwrap();
    let path = write_config(
        dir.path(),
        r#"
[env.a]
extends = "b"
trees = []

[env.b]
extends = "a"
trees = []
"#,
    );
    let cfg = load_config(&path).unwrap();
    let (_store_dir, store) = temp_store();
    let result = resolve_trees(&cfg, "a");
    assert!(result.is_err());
    let _ = store; // keep alive
}

#[test]
fn env_unknown_name_is_error() {
    let dir = TempDir::new().unwrap();
    let path = write_config(dir.path(), "[env.real]\ntrees = []\n");
    let cfg = load_config(&path).unwrap();
    assert!(resolve_trees(&cfg, "missing").is_err());
}
