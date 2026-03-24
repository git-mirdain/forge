use std::path::Path;

use tempfile::TempDir;

use crate::{
    env::{
        ToolchainsConfig, load_config, load_toolchains, merge_trees, resolve_env, resolve_trees,
    },
    import::{import_dir, import_oci, import_tarball},
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

#[test]
fn store_gc_removes_unreferenced_store_entries() {
    let (_dir, store) = temp_store();
    let repo = store.repo();

    // Create and materialize a tree, then remove the ref so it becomes unreferenced.
    let blob_oid = repo.blob(b"data").unwrap();
    let mut builder = repo.treebuilder(None).unwrap();
    builder.insert("f.txt", blob_oid, 0o100_644).unwrap();
    let tree_oid = builder.write().unwrap();

    store.materialize(tree_oid).unwrap();
    assert!(
        store
            .root()
            .join("store")
            .join(tree_oid.to_string())
            .exists()
    );

    // No ref → gc should remove it.
    let stats = store.gc().unwrap();
    assert_eq!(stats.store_entries, 1);
    assert!(
        !store
            .root()
            .join("store")
            .join(tree_oid.to_string())
            .exists()
    );
}

#[test]
fn store_gc_keeps_referenced_store_entries() {
    let (_dir, store) = temp_store();

    let src = TempDir::new().unwrap();
    std::fs::write(src.path().join("f"), b"keep").unwrap();
    let oid = import_dir(&store, src.path()).unwrap();
    store.materialize(oid).unwrap();

    let stats = store.gc().unwrap();
    assert_eq!(stats.store_entries, 0);
    assert!(store.root().join("store").join(oid.to_string()).exists());
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
// Import: OCI layout
// ---------------------------------------------------------------------------

/// Build a tar layer (uncompressed) from a list of entries.
fn make_oci_layer(files: &[(&str, &[u8])]) -> Vec<u8> {
    make_tar(files)
}

/// Build a tar layer that contains a whiteout entry.
fn make_whiteout_layer(whiteouts: &[&str], files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut all: Vec<(&str, &[u8])> = whiteouts.iter().map(|w| (*w, &b""[..])).collect();
    all.extend_from_slice(files);
    make_tar(&all)
}

/// Compute SHA-256 digest of data (using the same algorithm OCI uses).
fn sha256_hex(data: &[u8]) -> String {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), data).unwrap();
    let output = std::process::Command::new("shasum")
        .args(["-a", "256"])
        .arg(tmp.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    stdout.split_whitespace().next().unwrap().to_string()
}

/// Create a minimal OCI image layout directory with the given layers.
fn make_oci_layout(dir: &Path, layers: &[Vec<u8>]) {
    std::fs::write(dir.join("oci-layout"), r#"{"imageLayoutVersion":"1.0.0"}"#).unwrap();

    let blobs = dir.join("blobs").join("sha256");
    std::fs::create_dir_all(&blobs).unwrap();

    let mut layer_descs = Vec::new();
    for layer in layers {
        let hash = sha256_hex(layer);
        std::fs::write(blobs.join(&hash), layer).unwrap();
        layer_descs.push(format!(
            r#"{{"mediaType":"application/vnd.oci.image.layer.v1.tar","digest":"sha256:{hash}","size":{}}}"#,
            layer.len()
        ));
    }

    // Write a minimal config blob.
    let config = b"{}";
    let config_hash = sha256_hex(config);
    std::fs::write(blobs.join(&config_hash), config).unwrap();

    let manifest = format!(
        r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{{"mediaType":"application/vnd.oci.image.config.v1+json","digest":"sha256:{config_hash}","size":{}}},"layers":[{}]}}"#,
        config.len(),
        layer_descs.join(",")
    );
    let manifest_hash = sha256_hex(manifest.as_bytes());
    std::fs::write(blobs.join(&manifest_hash), &manifest).unwrap();

    let index = format!(
        r#"{{"schemaVersion":2,"manifests":[{{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"sha256:{manifest_hash}","size":{}}}]}}"#,
        manifest.len()
    );
    std::fs::write(dir.join("index.json"), index).unwrap();
}

#[test]
fn import_oci_basic() {
    let (_store_dir, store) = temp_store();
    let layout = TempDir::new().unwrap();

    let layer = make_oci_layer(&[("bin/hello", b"#!/bin/sh\necho hi"), ("lib/foo.so", b"ELF")]);
    make_oci_layout(layout.path(), &[layer]);

    let oid = import_oci(&store, layout.path().to_str().unwrap()).unwrap();
    let mat = store.materialize(oid).unwrap();
    assert_eq!(
        std::fs::read(mat.join("bin/hello")).unwrap(),
        b"#!/bin/sh\necho hi"
    );
    assert_eq!(std::fs::read(mat.join("lib/foo.so")).unwrap(), b"ELF");
}

#[test]
fn import_oci_whiteout_deletes_file() {
    let (_store_dir, store) = temp_store();
    let layout = TempDir::new().unwrap();

    let layer1 = make_oci_layer(&[("a.txt", b"alpha"), ("b.txt", b"beta")]);
    let layer2 = make_whiteout_layer(&[".wh.a.txt"], &[]);
    make_oci_layout(layout.path(), &[layer1, layer2]);

    let oid = import_oci(&store, layout.path().to_str().unwrap()).unwrap();
    let mat = store.materialize(oid).unwrap();
    assert!(!mat.join("a.txt").exists());
    assert_eq!(std::fs::read(mat.join("b.txt")).unwrap(), b"beta");
}

#[test]
fn import_oci_opaque_whiteout() {
    let (_store_dir, store) = temp_store();
    let layout = TempDir::new().unwrap();

    let layer1 = make_oci_layer(&[("dir/old.txt", b"old"), ("dir/keep.txt", b"keep")]);
    let layer2 = make_whiteout_layer(&["dir/.wh..wh..opq"], &[("dir/new.txt", b"new")]);
    make_oci_layout(layout.path(), &[layer1, layer2]);

    let oid = import_oci(&store, layout.path().to_str().unwrap()).unwrap();
    let mat = store.materialize(oid).unwrap();
    assert!(!mat.join("dir/old.txt").exists());
    assert!(!mat.join("dir/keep.txt").exists());
    assert_eq!(std::fs::read(mat.join("dir/new.txt")).unwrap(), b"new");
}

#[test]
fn import_oci_multi_layer_overlay() {
    let (_store_dir, store) = temp_store();
    let layout = TempDir::new().unwrap();

    let layer1 = make_oci_layer(&[("f.txt", b"v1")]);
    let layer2 = make_oci_layer(&[("f.txt", b"v2"), ("g.txt", b"new")]);
    make_oci_layout(layout.path(), &[layer1, layer2]);

    let oid = import_oci(&store, layout.path().to_str().unwrap()).unwrap();
    let mat = store.materialize(oid).unwrap();
    assert_eq!(std::fs::read(mat.join("f.txt")).unwrap(), b"v2");
    assert_eq!(std::fs::read(mat.join("g.txt")).unwrap(), b"new");
}

#[test]
fn import_oci_not_a_layout() {
    let (_store_dir, store) = temp_store();
    let dir = TempDir::new().unwrap();
    assert!(import_oci(&store, dir.path().to_str().unwrap()).is_err());
}

// ---------------------------------------------------------------------------
// Env: config and composition
// ---------------------------------------------------------------------------

fn write_config(dir: &Path, content: &str) -> std::path::PathBuf {
    let path = dir.join("environment.toml");
    std::fs::write(&path, content).unwrap();
    path
}

fn write_toolchains(dir: &Path, content: &str) -> std::path::PathBuf {
    let path = dir.join("toolchains.toml");
    std::fs::write(&path, content).unwrap();
    path
}

#[test]
fn env_load_config_basic() {
    let dir = TempDir::new().unwrap();
    let path = write_config(
        dir.path(),
        r#"
[project]
default = "default"

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
[project]
default = "base"

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
        &format!("[project]\ndefault = \"myenv\"\n\n[env.myenv]\ntrees = [\"{tree_oid}\"]\n"),
    );
    let cfg = load_config(&cfg_path).unwrap();
    let oid = resolve_env(&store, &cfg, None, "myenv").unwrap();

    let envs = store.list_envs().unwrap();
    assert!(envs.contains(&oid));
}

#[test]
fn env_circular_extends_is_error() {
    let dir = TempDir::new().unwrap();
    let path = write_config(
        dir.path(),
        r#"
[project]
default = "a"

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
    let result = resolve_trees(&cfg, None, "a");
    assert!(result.is_err());
    let _ = store; // keep alive
}

#[test]
fn env_unknown_name_is_error() {
    let dir = TempDir::new().unwrap();
    let path = write_config(
        dir.path(),
        "[project]\ndefault = \"real\"\n\n[env.real]\ntrees = []\n",
    );
    let cfg = load_config(&path).unwrap();
    assert!(resolve_trees(&cfg, None, "missing").is_err());
}

#[test]
fn toolchains_load_basic() {
    let dir = TempDir::new().unwrap();
    let path = write_toolchains(
        dir.path(),
        r#"
[rust]
source = "git://kiln-packages/rust@1.82.0"
oid = "a3f1c9d2b8e64f5a1c0d9e2f3b4a5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b"

[python]
source = "git://kiln-packages/cpython@3.12.0"
"#,
    );
    let tc = load_toolchains(&path).unwrap();
    assert_eq!(tc.toolchains.len(), 2);
    assert_eq!(
        tc.toolchains["rust"].source,
        "git://kiln-packages/rust@1.82.0"
    );
    assert!(tc.toolchains["rust"].oid.is_some());
    assert!(tc.toolchains["python"].oid.is_none());
}

#[test]
fn toolchains_resolve_trees_includes_toolchain_oids() {
    let (_dir, store) = temp_store();
    let repo = store.repo();

    let blob = repo.blob(b"rustc").unwrap();
    let mut tb = repo.treebuilder(None).unwrap();
    tb.insert("rustc", blob, 0o100_755).unwrap();
    let tc_tree = tb.write().unwrap();

    let cfg_dir = TempDir::new().unwrap();
    let cfg_path = write_config(
        cfg_dir.path(),
        r#"
[project]
default = "dev"

[env.dev]
toolchains = ["rust"]
"#,
    );
    let cfg = load_config(&cfg_path).unwrap();

    let tc = ToolchainsConfig {
        toolchains: [(
            "rust".into(),
            crate::env::ToolchainDef {
                source: "git://kiln-packages/rust@1.82.0".into(),
                oid: Some(tc_tree.to_string()),
                strip_prefix: 0,
            },
        )]
        .into_iter()
        .collect(),
    };

    let oids = resolve_trees(&cfg, Some(&tc), "dev").unwrap();
    assert_eq!(oids, vec![tc_tree]);
}

#[test]
fn toolchains_missing_oid_is_error() {
    let cfg_dir = TempDir::new().unwrap();
    let cfg_path = write_config(
        cfg_dir.path(),
        r#"
[project]
default = "dev"

[env.dev]
toolchains = ["rust"]
"#,
    );
    let cfg = load_config(&cfg_path).unwrap();

    let tc = ToolchainsConfig {
        toolchains: [(
            "rust".into(),
            crate::env::ToolchainDef {
                source: "git://kiln-packages/rust@1.82.0".into(),
                oid: None,
                strip_prefix: 0,
            },
        )]
        .into_iter()
        .collect(),
    };

    assert!(resolve_trees(&cfg, Some(&tc), "dev").is_err());
}

#[test]
fn toolchains_missing_name_is_error() {
    let cfg_dir = TempDir::new().unwrap();
    let cfg_path = write_config(
        cfg_dir.path(),
        r#"
[project]
default = "dev"

[env.dev]
toolchains = ["go"]
"#,
    );
    let cfg = load_config(&cfg_path).unwrap();

    let tc = ToolchainsConfig {
        toolchains: [(
            "rust".into(),
            crate::env::ToolchainDef {
                source: "git://kiln-packages/rust@1.82.0".into(),
                oid: Some(
                    "a3f1c9d2b8e64f5a1c0d9e2f3b4a5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b".into(),
                ),
                strip_prefix: 0,
            },
        )]
        .into_iter()
        .collect(),
    };

    assert!(resolve_trees(&cfg, Some(&tc), "dev").is_err());
}

#[test]
fn toolchains_no_config_with_toolchain_ref_is_error() {
    let cfg_dir = TempDir::new().unwrap();
    let cfg_path = write_config(
        cfg_dir.path(),
        r#"
[project]
default = "dev"

[env.dev]
toolchains = ["rust"]
"#,
    );
    let cfg = load_config(&cfg_path).unwrap();
    assert!(resolve_trees(&cfg, None, "dev").is_err());
}

#[test]
fn toolchains_inherited_through_extends() {
    let (_dir, store) = temp_store();
    let repo = store.repo();

    let blob_r = repo.blob(b"rustc").unwrap();
    let mut tb1 = repo.treebuilder(None).unwrap();
    tb1.insert("rustc", blob_r, 0o100_755).unwrap();
    let rust_tree = tb1.write().unwrap();

    let blob_p = repo.blob(b"python3").unwrap();
    let mut tb2 = repo.treebuilder(None).unwrap();
    tb2.insert("python3", blob_p, 0o100_755).unwrap();
    let python_tree = tb2.write().unwrap();

    let cfg_dir = TempDir::new().unwrap();
    let cfg_path = write_config(
        cfg_dir.path(),
        r#"
[project]
default = "dev"

[env.base]
toolchains = ["rust"]

[env.dev]
extends = "base"
toolchains = ["python"]
"#,
    );
    let cfg = load_config(&cfg_path).unwrap();

    let tc = ToolchainsConfig {
        toolchains: [
            (
                "rust".into(),
                crate::env::ToolchainDef {
                    source: "git://kiln-packages/rust@1.82.0".into(),
                    oid: Some(rust_tree.to_string()),
                    strip_prefix: 0,
                },
            ),
            (
                "python".into(),
                crate::env::ToolchainDef {
                    source: "git://kiln-packages/cpython@3.12.0".into(),
                    oid: Some(python_tree.to_string()),
                    strip_prefix: 0,
                },
            ),
        ]
        .into_iter()
        .collect(),
    };

    let oids = resolve_trees(&cfg, Some(&tc), "dev").unwrap();
    assert_eq!(oids, vec![rust_tree, python_tree]);
}

#[test]
fn toolchains_before_raw_trees() {
    let (_dir, store) = temp_store();
    let repo = store.repo();

    let blob_tc = repo.blob(b"rustc").unwrap();
    let mut tb1 = repo.treebuilder(None).unwrap();
    tb1.insert("rustc", blob_tc, 0o100_755).unwrap();
    let tc_tree = tb1.write().unwrap();

    let blob_raw = repo.blob(b"extra").unwrap();
    let mut tb2 = repo.treebuilder(None).unwrap();
    tb2.insert("extra", blob_raw, 0o100_644).unwrap();
    let raw_tree = tb2.write().unwrap();

    let cfg_dir = TempDir::new().unwrap();
    let cfg_path = write_config(
        cfg_dir.path(),
        &format!(
            r#"
[project]
default = "dev"

[env.dev]
toolchains = ["rust"]
trees = ["{raw_tree}"]
"#
        ),
    );
    let cfg = load_config(&cfg_path).unwrap();

    let tc = ToolchainsConfig {
        toolchains: [(
            "rust".into(),
            crate::env::ToolchainDef {
                source: "git://kiln-packages/rust@1.82.0".into(),
                oid: Some(tc_tree.to_string()),
                strip_prefix: 0,
            },
        )]
        .into_iter()
        .collect(),
    };

    let oids = resolve_trees(&cfg, Some(&tc), "dev").unwrap();
    assert_eq!(oids, vec![tc_tree, raw_tree]);
}
