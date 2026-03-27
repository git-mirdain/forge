use anyhow::Result;
use git2::{Oid, Repository, TreeBuilder};

/// Recursively insert a blob at an arbitrary depth inside a tree builder.
pub(crate) fn insert_nested(
    repo: &Repository,
    builder: &mut TreeBuilder<'_>,
    components: &[&str],
    blob_oid: Oid,
) -> Result<()> {
    match components {
        [] => {}
        [leaf] => {
            builder.insert(leaf, blob_oid, 0o100_644)?;
        }
        [head, rest @ ..] => {
            let mut sub_builder = if let Some(existing) = builder.get(head)? {
                let existing_tree = repo.find_tree(existing.id())?;
                repo.treebuilder(Some(&existing_tree))?
            } else {
                repo.treebuilder(None)?
            };
            insert_nested(repo, &mut sub_builder, rest, blob_oid)?;
            let sub_oid = sub_builder.write()?;
            builder.insert(head, sub_oid, 0o040_000)?;
        }
    }
    Ok(())
}

/// Build a tree from `(path, content)` pairs, supporting `/`-separated nested paths.
pub(crate) fn build_fields_tree(repo: &Repository, fields: &[(&str, &[u8])]) -> Result<Oid> {
    let mut builder = repo.treebuilder(None)?;
    for (name, value) in fields {
        let blob_oid = repo.blob(value)?;
        let parts: Vec<&str> = name.split('/').collect();
        insert_nested(repo, &mut builder, &parts, blob_oid)?;
    }
    Ok(builder.write()?)
}
