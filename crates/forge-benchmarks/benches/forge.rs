//! Forge benchmark suite — measures Git ref/object performance for Forge ops.
//! All repos are bare (no remotes), created fresh per benchmark group.
#![allow(missing_docs)]

use std::collections::HashMap;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use git2::{Oid, Repository, Signature};
use tempfile::TempDir;

// --- helpers -----------------------------------------------------------------

fn bare_repo() -> (TempDir, Repository) {
    let dir = TempDir::new().unwrap();
    let repo = Repository::init_bare(dir.path()).unwrap();
    (dir, repo)
}

fn sig() -> Signature<'static> {
    Signature::now("Bench Bot", "bench@example.com").unwrap()
}

fn write_blob(repo: &Repository, data: &[u8]) -> Oid {
    repo.blob(data).unwrap()
}

fn build_tree(repo: &Repository, entries: &[(&str, Oid, i32)]) -> Oid {
    let mut tb = repo.treebuilder(None).unwrap();
    for &(name, oid, mode) in entries {
        tb.insert(name, oid, mode).unwrap();
    }
    tb.write().unwrap()
}

fn write_commit(repo: &Repository, tree_oid: Oid, parent: Option<Oid>, msg: &str) -> Oid {
    let s = sig();
    let tree = repo.find_tree(tree_oid).unwrap();
    match parent {
        Some(p) => {
            let pc = repo.find_commit(p).unwrap();
            repo.commit(None, &s, &s, msg, &tree, &[&pc]).unwrap()
        }
        None => repo.commit(None, &s, &s, msg, &tree, &[]).unwrap(),
    }
}

fn set_ref(repo: &Repository, refname: &str, oid: Oid) {
    repo.reference(refname, oid, true, "bench").unwrap();
}

/// CAS via libgit2 reference_matching (git_reference_create_matching).
fn cas_ref(repo: &Repository, refname: &str, new_oid: Oid, expected: Oid) -> bool {
    repo.reference_matching(refname, new_oid, true, expected, "bench CAS")
        .is_ok()
}

// --- bench 1 & 2: issues -----------------------------------------------------

fn issue_meta_blob(id: u64, state: &str) -> Vec<u8> {
    format!(
        "id = {id}\nauthor = \"alice\"\ntitle = \"Issue {id}: fix the thing\"\n\
         state = \"{state}\"\nlabels = [\"bug\"]\nassignees = []\n\
         created = \"2024-01-01T00:00:00Z\"\n"
    )
    .into_bytes()
}

fn issue_body_blob(id: u64) -> Vec<u8> {
    // ~500 bytes
    format!(
        "# Issue {id}\n\nThis issue was filed to track a problem with component {id}.\n\n\
         ## Steps to reproduce\n\n1. Open the application.\n2. Navigate to section {id}.\n\
         3. Click the button.\n4. Observe incorrect behaviour.\n\n\
         ## Expected\n\nNothing should crash.\n\n## Actual\n\nIt crashes.\n"
    )
    .into_bytes()
}

/// Seed `n` issues and write `refs/meta/counters`. Half open, half closed.
/// Returns the counter commit OID.
fn seed_issues(repo: &Repository, n: u64) -> Oid {
    for id in 1..=n {
        let state = if id % 2 == 0 { "closed" } else { "open" };
        let meta = write_blob(repo, &issue_meta_blob(id, state));
        let body = write_blob(repo, &issue_body_blob(id));
        let tree = build_tree(repo, &[("meta", meta, 0o100644), ("body", body, 0o100644)]);
        let commit = write_commit(repo, tree, None, &format!("issue {id}"));
        set_ref(repo, &format!("refs/meta/issues/{id}"), commit);
    }
    let cnt_blob = write_blob(repo, n.to_string().as_bytes());
    let cnt_tree = build_tree(repo, &[("issues", cnt_blob, 0o100644)]);
    let cnt_commit = write_commit(repo, cnt_tree, None, &format!("counter {n}"));
    set_ref(repo, "refs/meta/counters", cnt_commit);
    cnt_commit
}

fn bench_issue_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("issue_creation");
    for n in [100u64, 1_000, 10_000] {
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let (dir, repo) = bare_repo();
                    seed_issues(&repo, n);
                    (dir, repo)
                },
                |(_dir, repo)| {
                    // 1. Read counter
                    let cref = repo.find_reference("refs/meta/counters").unwrap();
                    let old_oid = cref.target().unwrap();
                    let cnt_tree = repo.find_commit(old_oid).unwrap().tree().unwrap();
                    let cnt_val: u64 = {
                        let b = repo
                            .find_blob(cnt_tree.get_name("issues").unwrap().id())
                            .unwrap();
                        std::str::from_utf8(b.content())
                            .unwrap()
                            .trim()
                            .parse()
                            .unwrap()
                    };
                    let new_id = cnt_val + 1;

                    // 2. Write issue commit
                    let meta = write_blob(&repo, &issue_meta_blob(new_id, "open"));
                    let body = write_blob(&repo, &issue_body_blob(new_id));
                    let tree =
                        build_tree(&repo, &[("meta", meta, 0o100644), ("body", body, 0o100644)]);
                    let issue_commit = write_commit(&repo, tree, None, &format!("issue {new_id}"));
                    set_ref(&repo, &format!("refs/meta/issues/{new_id}"), issue_commit);

                    // 3. Build and CAS-update counter
                    let new_cnt_blob = write_blob(&repo, new_id.to_string().as_bytes());
                    let new_cnt_tree = build_tree(&repo, &[("issues", new_cnt_blob, 0o100644)]);
                    let new_cnt_commit = write_commit(
                        &repo,
                        new_cnt_tree,
                        Some(old_oid),
                        &format!("counter {new_id}"),
                    );
                    let _ok = cas_ref(&repo, "refs/meta/counters", new_cnt_commit, old_oid);
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_issue_listing(c: &mut Criterion) {
    let mut group = c.benchmark_group("issue_listing");
    for n in [100u64, 1_000, 10_000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let (_dir, repo) = bare_repo();
            seed_issues(&repo, n);
            b.iter(|| {
                let mut open: Vec<u64> = Vec::new();
                for r in repo.references_glob("refs/meta/issues/*").unwrap() {
                    let r = r.unwrap();
                    let name = r.name().unwrap();
                    let id: u64 = name
                        .trim_start_matches("refs/meta/issues/")
                        .parse()
                        .unwrap();
                    let commit = repo.find_commit(r.target().unwrap()).unwrap();
                    let tree = commit.tree().unwrap();
                    let blob = repo.find_blob(tree.get_name("meta").unwrap().id()).unwrap();
                    if std::str::from_utf8(blob.content())
                        .unwrap()
                        .contains("state = \"open\"")
                    {
                        open.push(id);
                    }
                }
                open
            });
        });
    }
    group.finish();
}

// --- bench 3: comment lookup -------------------------------------------------

/// Seed `refs/metadata/comments` with `total` comments spread across `k` blobs.
/// Returns the k blob OIDs.
fn seed_comments(repo: &Repository, k: usize, total: usize) -> Vec<Oid> {
    let per_file = (total / k).max(1);
    let blob_oids: Vec<Oid> = (0..k)
        .map(|i| {
            write_blob(
                repo,
                format!("// file {i}\nfn main() {{}}\n")
                    .repeat(20)
                    .as_bytes(),
            )
        })
        .collect();

    let mut root_tb = repo.treebuilder(None).unwrap();
    for &boid in &blob_oids {
        let hex = boid.to_string();
        let mut dir_tb = repo.treebuilder(None).unwrap();
        for ci in 0..per_file {
            let cid = format!("{ci:08x}");
            let meta = write_blob(
                repo,
                format!(
                    "author = \"alice\"\nstart_line = {}\nend_line = {}\n",
                    ci + 1,
                    ci + 3
                )
                .as_bytes(),
            );
            let body = write_blob(repo, format!("Comment {ci} body.\n").as_bytes());
            let ct = build_tree(repo, &[("meta", meta, 0o100644), ("body", body, 0o100644)]);
            dir_tb.insert(&cid, ct, 0o040000).unwrap();
        }
        let dir_tree = dir_tb.write().unwrap();
        root_tb.insert(&hex, dir_tree, 0o040000).unwrap();
    }
    let root_tree = root_tb.write().unwrap();
    let commit = write_commit(repo, root_tree, None, "seed comments");
    set_ref(repo, "refs/metadata/comments", commit);
    blob_oids
}

fn bench_comment_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("comment_lookup");
    for total in [100usize, 500, 1_000] {
        group.bench_with_input(BenchmarkId::from_parameter(total), &total, |b, &total| {
            let (_dir, repo) = bare_repo();
            let blobs = seed_comments(&repo, 50, total);
            let target = blobs[blobs.len() / 2];
            b.iter(|| {
                let cref = repo.find_reference("refs/metadata/comments").unwrap();
                let root = repo
                    .find_commit(cref.target().unwrap())
                    .unwrap()
                    .tree()
                    .unwrap();
                let hex = target.to_string();
                let dir_entry = match root.get_name(&hex) {
                    Some(e) => e,
                    None => return vec![],
                };
                let dir = repo.find_tree(dir_entry.id()).unwrap();
                let mut out = Vec::new();
                for entry in dir.iter() {
                    let ct = repo.find_tree(entry.id()).unwrap();
                    let meta = repo.find_blob(ct.get_name("meta").unwrap().id()).unwrap();
                    let body = repo.find_blob(ct.get_name("body").unwrap().id()).unwrap();
                    out.push((meta.content().to_vec(), body.content().to_vec()));
                }
                out
            });
        });
    }
    group.finish();
}

// --- bench 4: link traversal -------------------------------------------------

fn seed_links(repo: &Repository, n: usize) {
    let mut root_tb = repo.treebuilder(None).unwrap();
    let mut issues_tb = repo.treebuilder(None).unwrap();
    let mut i42_tb = repo.treebuilder(None).unwrap();
    for i in 0..n {
        let name = format!("comment:{i:016x}");
        let payload = format!("type = \"comment\"\ntarget = \"{i:016x}\"\n");
        let blob = write_blob(repo, payload.as_bytes());
        i42_tb.insert(&name, blob, 0o100644).unwrap();
    }
    let i42 = i42_tb.write().unwrap();
    issues_tb.insert("42", i42, 0o040000).unwrap();
    let issues = issues_tb.write().unwrap();
    root_tb.insert("issues", issues, 0o040000).unwrap();
    let root = root_tb.write().unwrap();
    let commit = write_commit(repo, root, None, "seed links");
    set_ref(repo, "refs/metadata/links", commit);
}

fn bench_link_traversal(c: &mut Criterion) {
    let mut group = c.benchmark_group("link_traversal");
    for n in [10usize, 100, 500] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let (_dir, repo) = bare_repo();
            seed_links(&repo, n);
            b.iter(|| {
                let r = repo.find_reference("refs/metadata/links").unwrap();
                let root = repo
                    .find_commit(r.target().unwrap())
                    .unwrap()
                    .tree()
                    .unwrap();
                let issues = repo
                    .find_tree(root.get_name("issues").unwrap().id())
                    .unwrap();
                let i42 = repo.find_tree(issues.get_name("42").unwrap().id()).unwrap();
                i42.iter()
                    .map(|e| e.name().unwrap().to_owned())
                    .collect::<Vec<_>>()
            });
        });
    }
    group.finish();
}

// --- bench 5: approval lookup ------------------------------------------------

/// Seed `refs/metadata/approvals` with `p` patch-IDs, one approver each.
fn seed_approvals(repo: &Repository, p: usize) -> Vec<String> {
    let patch_ids: Vec<String> = (0..p).map(|i| format!("{i:040x}")).collect();
    let mut root_tb = repo.treebuilder(None).unwrap();
    for pid in &patch_ids {
        let fp = format!("fp:{pid}");
        let payload = format!(
            "approver = \"{fp}\"\ntimestamp = \"2024-01-01T00:00:00Z\"\nkind = \"patch\"\n"
        );
        let blob = write_blob(repo, payload.as_bytes());
        let mut ptb = repo.treebuilder(None).unwrap();
        ptb.insert(&fp, blob, 0o100644).unwrap();
        let pt = ptb.write().unwrap();
        root_tb.insert(pid, pt, 0o040000).unwrap();
    }
    let root = root_tb.write().unwrap();
    let commit = write_commit(repo, root, None, "seed approvals");
    set_ref(repo, "refs/metadata/approvals", commit);
    patch_ids
}

fn bench_approval_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("approval_lookup");
    for p in [10usize, 100, 1_000] {
        // hit: patch ID present
        group.bench_with_input(BenchmarkId::new("hit", p), &p, |b, &p| {
            let (_dir, repo) = bare_repo();
            let ids = seed_approvals(&repo, p);
            let target = ids[p / 2].clone();
            let policy = vec![format!("fp:{target}")];
            b.iter(|| {
                let r = repo.find_reference("refs/metadata/approvals").unwrap();
                let root = repo
                    .find_commit(r.target().unwrap())
                    .unwrap()
                    .tree()
                    .unwrap();
                match root.get_name(&target) {
                    None => false,
                    Some(e) => {
                        let pt = repo.find_tree(e.id()).unwrap();
                        pt.iter()
                            .any(|e| policy.contains(&e.name().unwrap().to_owned()))
                    }
                }
            });
        });
        // miss: patch ID absent
        group.bench_with_input(BenchmarkId::new("miss", p), &p, |b, &p| {
            let (_dir, repo) = bare_repo();
            seed_approvals(&repo, p);
            let absent = "ffffffffffffffffffffffffffffffffffffffff00".to_owned();
            b.iter(|| {
                let r = repo.find_reference("refs/metadata/approvals").unwrap();
                let root = repo
                    .find_commit(r.target().unwrap())
                    .unwrap()
                    .tree()
                    .unwrap();
                root.get_name(&absent).is_some()
            });
        });
    }
    group.finish();
}

// --- bench 6: metadata auto-merge --------------------------------------------

/// Build a base commit with `count` comment subtrees under refs/metadata/comments.
fn seed_merge_base(repo: &Repository, count: usize) -> Oid {
    let mut root_tb = repo.treebuilder(None).unwrap();
    for i in 0..count {
        let name = format!("comment-{i:04}");
        let meta = write_blob(
            repo,
            format!("author = \"alice\"\nbody = \"c{i}\"\n").as_bytes(),
        );
        let body = write_blob(repo, format!("Comment {i} body.\n").as_bytes());
        let ct = build_tree(repo, &[("meta", meta, 0o100644), ("body", body, 0o100644)]);
        root_tb.insert(&name, ct, 0o040000).unwrap();
    }
    let root = root_tb.write().unwrap();
    let commit = write_commit(repo, root, None, "base");
    set_ref(repo, "refs/metadata/comments", commit);
    commit
}

fn bench_auto_merge(c: &mut Criterion) {
    let mut group = c.benchmark_group("auto_merge");
    for count in [10usize, 100, 500] {
        // clean: Alice and Bob add different new entries
        group.bench_with_input(BenchmarkId::new("clean", count), &count, |b, &count| {
            b.iter_batched(
                || {
                    let (dir, repo) = bare_repo();
                    let base = seed_merge_base(&repo, count);
                    let (a_commit, b_commit) = {
                        let base_tree = repo.find_commit(base).unwrap().tree().unwrap();

                        let a_meta = write_blob(&repo, b"author = \"alice\"\n");
                        let a_body = write_blob(&repo, b"Alice comment.\n");
                        let a_sub = build_tree(
                            &repo,
                            &[("meta", a_meta, 0o100644), ("body", a_body, 0o100644)],
                        );
                        let mut a_tb = repo.treebuilder(Some(&base_tree)).unwrap();
                        a_tb.insert("alice-comment", a_sub, 0o040000).unwrap();
                        let a_tree = a_tb.write().unwrap();
                        let a_commit = write_commit(&repo, a_tree, Some(base), "alice");

                        let b_meta = write_blob(&repo, b"author = \"bob\"\n");
                        let b_body = write_blob(&repo, b"Bob comment.\n");
                        let b_sub = build_tree(
                            &repo,
                            &[("meta", b_meta, 0o100644), ("body", b_body, 0o100644)],
                        );
                        let mut b_tb = repo.treebuilder(Some(&base_tree)).unwrap();
                        b_tb.insert("bob-comment", b_sub, 0o040000).unwrap();
                        let b_tree = b_tb.write().unwrap();
                        let b_commit = write_commit(&repo, b_tree, Some(base), "bob");
                        (a_commit, b_commit)
                    };

                    (dir, repo, base, a_commit, b_commit)
                },
                |(_dir, repo, base, a_commit, b_commit)| {
                    let anc = repo.find_commit(base).unwrap().tree().unwrap();
                    let our = repo.find_commit(a_commit).unwrap().tree().unwrap();
                    let their = repo.find_commit(b_commit).unwrap().tree().unwrap();
                    let opts = git2::MergeOptions::new();
                    let mut idx = repo.merge_trees(&anc, &our, &their, Some(&opts)).unwrap();
                    let conflicts = idx.has_conflicts();
                    if !conflicts {
                        let merged = idx.write_tree_to(&repo).unwrap();
                        let merged_tree = repo.find_tree(merged).unwrap();
                        let s = sig();
                        let pa = repo.find_commit(a_commit).unwrap();
                        let pb = repo.find_commit(b_commit).unwrap();
                        let mc = repo
                            .commit(None, &s, &s, "merge", &merged_tree, &[&pa, &pb])
                            .unwrap();
                        set_ref(&repo, "refs/metadata/comments", mc);
                    }
                    conflicts
                },
                criterion::BatchSize::SmallInput,
            );
        });

        // conflict: Alice and Bob both resolve the same comment
        group.bench_with_input(BenchmarkId::new("conflict", count), &count, |b, &count| {
            b.iter_batched(
                || {
                    let (dir, repo) = bare_repo();
                    let base = seed_merge_base(&repo, count);
                    let (a_commit, b_commit) = {
                        let base_tree = repo.find_commit(base).unwrap().tree().unwrap();
                        let c0_oid = base_tree.get_name("comment-0000").unwrap().id();

                        let r_a = write_blob(&repo, b"by = \"alice\"\n");
                        let mut a_c0 = repo
                            .treebuilder(Some(&repo.find_tree(c0_oid).unwrap()))
                            .unwrap();
                        a_c0.insert("resolved", r_a, 0o100644).unwrap();
                        let a_c0t = a_c0.write().unwrap();
                        let mut a_tb = repo.treebuilder(Some(&base_tree)).unwrap();
                        a_tb.insert("comment-0000", a_c0t, 0o040000).unwrap();
                        let a_tree = a_tb.write().unwrap();
                        let a_commit = write_commit(&repo, a_tree, Some(base), "alice resolves");

                        let r_b = write_blob(&repo, b"by = \"bob\"\n");
                        let mut b_c0 = repo
                            .treebuilder(Some(&repo.find_tree(c0_oid).unwrap()))
                            .unwrap();
                        b_c0.insert("resolved", r_b, 0o100644).unwrap();
                        let b_c0t = b_c0.write().unwrap();
                        let mut b_tb = repo.treebuilder(Some(&base_tree)).unwrap();
                        b_tb.insert("comment-0000", b_c0t, 0o040000).unwrap();
                        let b_tree = b_tb.write().unwrap();
                        let b_commit = write_commit(&repo, b_tree, Some(base), "bob resolves");
                        (a_commit, b_commit)
                    };

                    (dir, repo, base, a_commit, b_commit)
                },
                |(_dir, repo, base, a_commit, b_commit)| {
                    let anc = repo.find_commit(base).unwrap().tree().unwrap();
                    let our = repo.find_commit(a_commit).unwrap().tree().unwrap();
                    let their = repo.find_commit(b_commit).unwrap().tree().unwrap();
                    let opts = git2::MergeOptions::new();
                    let idx = repo.merge_trees(&anc, &our, &their, Some(&opts)).unwrap();
                    idx.has_conflicts()
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

// --- bench 7: reanchoring ----------------------------------------------------

/// Seed n comments on `old_blob` under refs/metadata/comments.
/// Returns the root commit OID and the comment root tree.
fn seed_reanchor_comments(repo: &Repository, old_blob: Oid, n: usize) -> Oid {
    let hex = old_blob.to_string();
    let mut dir_tb = repo.treebuilder(None).unwrap();
    for i in 0..n {
        let cid = format!("{i:08x}");
        let meta = write_blob(
            repo,
            format!(
                "author = \"alice\"\nstart_line = {}\nend_line = {}\n",
                i * 5 + 1,
                i * 5 + 3
            )
            .as_bytes(),
        );
        let body = write_blob(repo, format!("Comment {i}.\n").as_bytes());
        let ct = build_tree(repo, &[("meta", meta, 0o100644), ("body", body, 0o100644)]);
        dir_tb.insert(&cid, ct, 0o040000).unwrap();
    }
    let dir = dir_tb.write().unwrap();
    let mut root_tb = repo.treebuilder(None).unwrap();
    root_tb.insert(&hex, dir, 0o040000).unwrap();
    let root = root_tb.write().unwrap();
    let commit = write_commit(repo, root, None, "seed reanchor comments");
    set_ref(repo, "refs/metadata/comments", commit);
    commit
}

fn bench_reanchoring(c: &mut Criterion) {
    let mut group = c.benchmark_group("reanchoring");
    for n in [1usize, 10, 50] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let (dir, repo) = bare_repo();
                    // Create old file blob
                    let old_content: Vec<u8> = (0..100u32)
                        .map(|i| format!("line {i}\n"))
                        .collect::<String>()
                        .into_bytes();
                    let old_blob = write_blob(&repo, &old_content);
                    let base_commit = seed_reanchor_comments(&repo, old_blob, n);

                    // Create new file blob (each line shifted by adding a prefix)
                    let new_content: Vec<u8> = (0..100u32)
                        .map(|i| format!("// line {i}\n"))
                        .collect::<String>()
                        .into_bytes();
                    let new_blob = write_blob(&repo, &new_content);

                    // Blame map: every old start_line maps to start_line + 2
                    let blame_map: HashMap<u32, u32> = (0..n)
                        .map(|i| {
                            let old_start = (i * 5 + 1) as u32;
                            (old_start, old_start + 2)
                        })
                        .collect();

                    (dir, repo, old_blob, new_blob, base_commit, blame_map)
                },
                |(_dir, repo, old_blob, new_blob, _base_commit, blame_map)| {
                    let old_hex = old_blob.to_string();
                    let new_hex = new_blob.to_string();

                    // Read existing comments on old_blob
                    let cref = repo.find_reference("refs/metadata/comments").unwrap();
                    let old_root_oid = cref.target().unwrap();
                    let old_root = repo.find_commit(old_root_oid).unwrap().tree().unwrap();

                    let dir_entry = match old_root.get_name(&old_hex) {
                        Some(e) => e,
                        None => return old_root_oid,
                    };
                    let dir = repo.find_tree(dir_entry.id()).unwrap();

                    // Build new dir under new_blob_hex with updated line ranges
                    let mut new_dir_tb = repo.treebuilder(None).unwrap();
                    for entry in dir.iter() {
                        let ct = repo.find_tree(entry.id()).unwrap();
                        let meta_entry = ct.get_name("meta").unwrap();
                        let meta_blob = repo.find_blob(meta_entry.id()).unwrap();
                        let meta_str = std::str::from_utf8(meta_blob.content()).unwrap();

                        // Parse start_line
                        let start_line: u32 = meta_str
                            .lines()
                            .find(|l| l.starts_with("start_line"))
                            .and_then(|l| l.split('=').nth(1))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(1);

                        let new_start = blame_map.get(&start_line).copied().unwrap_or(start_line);
                        let new_end = new_start + 2;

                        let new_meta_content = format!(
                            "author = \"alice\"\nstart_line = {new_start}\nend_line = {new_end}\n"
                        );
                        let new_meta = write_blob(&repo, new_meta_content.as_bytes());
                        let body_oid = ct.get_name("body").unwrap().id();
                        let new_ct = build_tree(
                            &repo,
                            &[("meta", new_meta, 0o100644), ("body", body_oid, 0o100644)],
                        );
                        new_dir_tb
                            .insert(entry.name().unwrap(), new_ct, 0o040000)
                            .unwrap();
                    }
                    let new_dir = new_dir_tb.write().unwrap();

                    // Build new root: remove old_hex entry, add new_hex entry
                    let mut new_root_tb = repo.treebuilder(Some(&old_root)).unwrap();
                    new_root_tb.remove(&old_hex).unwrap();
                    new_root_tb.insert(&new_hex, new_dir, 0o040000).unwrap();
                    let new_root = new_root_tb.write().unwrap();

                    let new_commit = write_commit(&repo, new_root, Some(old_root_oid), "reanchor");
                    set_ref(&repo, "refs/metadata/comments", new_commit);
                    new_commit
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

// --- criterion wiring --------------------------------------------------------

criterion_group!(
    benches,
    bench_issue_creation,
    bench_issue_listing,
    bench_comment_lookup,
    bench_link_traversal,
    bench_approval_lookup,
    bench_auto_merge,
    bench_reanchoring,
);
criterion_main!(benches);
