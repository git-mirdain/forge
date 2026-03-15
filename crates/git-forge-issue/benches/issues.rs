#![allow(missing_docs)]

use criterion::{criterion_group, criterion_main, Criterion};
use git_forge_issue::issue::{Issue, IssueMeta, IssueState, ISSUES_REF_PREFIX};

fn bench_issue_state_as_str(c: &mut Criterion) {
    c.bench_function("IssueState::as_str/open", |b| {
        b.iter(|| criterion::black_box(IssueState::Open.as_str()));
    });

    c.bench_function("IssueState::as_str/closed", |b| {
        b.iter(|| criterion::black_box(IssueState::Closed.as_str()));
    });
}

#[allow(clippy::eq_op)]
fn bench_issue_state_equality(c: &mut Criterion) {
    c.bench_function("IssueState::equality", |b| {
        b.iter(|| {
            criterion::black_box(IssueState::Open == IssueState::Open);
            criterion::black_box(IssueState::Open == IssueState::Closed);
        });
    });
}

fn bench_issue_ref(c: &mut Criterion) {
    c.bench_function("Issues::issue_ref/small_id", |b| {
        b.iter(|| {
            let id: u64 = criterion::black_box(1);
            criterion::black_box(format!("{ISSUES_REF_PREFIX}{id}"))
        });
    });

    c.bench_function("Issues::issue_ref/large_id", |b| {
        b.iter(|| {
            let id: u64 = criterion::black_box(99_999);
            criterion::black_box(format!("{ISSUES_REF_PREFIX}{id}"))
        });
    });
}

fn bench_issue_meta_construction(c: &mut Criterion) {
    c.bench_function("IssueMeta::construct", |b| {
        b.iter(|| IssueMeta {
            author: criterion::black_box("fingerprint-0011".to_owned()),
            title: criterion::black_box("Add benchmarks".to_owned()),
            state: criterion::black_box(IssueState::Open),
            labels: criterion::black_box(vec!["perf".to_owned()]),
        });
    });
}

fn bench_issue_construction(c: &mut Criterion) {
    c.bench_function("Issue::construct/no_comments", |b| {
        b.iter(|| Issue {
            id: criterion::black_box(1),
            meta: IssueMeta {
                author: criterion::black_box("fingerprint-0011".to_owned()),
                title: criterion::black_box("Add benchmarks".to_owned()),
                state: criterion::black_box(IssueState::Open),
                labels: criterion::black_box(vec![]),
            },
            body: criterion::black_box("Please add criterion benches.".to_owned()),
            comments: criterion::black_box(vec![]),
        });
    });

    c.bench_function("Issue::construct/with_comments", |b| {
        b.iter(|| Issue {
            id: criterion::black_box(42),
            meta: IssueMeta {
                author: criterion::black_box("fingerprint-0011".to_owned()),
                title: criterion::black_box("Discuss approach".to_owned()),
                state: criterion::black_box(IssueState::Closed),
                labels: criterion::black_box(vec!["discussion".to_owned()]),
            },
            body: criterion::black_box("What approach should we take?".to_owned()),
            comments: criterion::black_box(vec![
                (
                    "001-2024-03-15T10:00:00Z-fingerprint-aabb".to_owned(),
                    "I think we should go with option A.".to_owned(),
                ),
                (
                    "002-2024-03-15T11:00:00Z-fingerprint-0011".to_owned(),
                    "Agreed, let's proceed.".to_owned(),
                ),
            ]),
        });
    });
}

criterion_group!(
    benches,
    bench_issue_state_as_str,
    bench_issue_state_equality,
    bench_issue_ref,
    bench_issue_meta_construction,
    bench_issue_construction,
);
criterion_main!(benches);
