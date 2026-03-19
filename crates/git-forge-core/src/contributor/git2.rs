//! `git2::Repository` implementation of [`Contributors`].

use git2::Repository;

use crate::contributor::{CONTRIBUTORS_REF, Contributor, Contributors};

fn blob_str<'repo>(
    repo: &'repo Repository,
    tree: &git2::Tree<'repo>,
    name: &str,
) -> Result<Option<String>, git2::Error> {
    let Some(entry) = tree.get_name(name) else {
        return Ok(None);
    };
    let obj = entry.to_object(repo)?;
    let blob = obj
        .as_blob()
        .ok_or_else(|| git2::Error::from_str(&format!("'{name}' is not a blob")))?;
    Ok(Some(
        std::str::from_utf8(blob.content())
            .unwrap_or("")
            .trim_end()
            .to_string(),
    ))
}

fn contributor_from_tree(
    repo: &Repository,
    root: &git2::Tree<'_>,
    id: &str,
) -> Result<Option<Contributor>, git2::Error> {
    let Some(entry) = root.get_name(id) else {
        return Ok(None);
    };
    let obj = entry.to_object(repo)?;
    let subtree = obj
        .as_tree()
        .ok_or_else(|| git2::Error::from_str(&format!("contributor entry '{id}' is not a tree")))?;
    let Some(name) = blob_str(repo, subtree, "name")? else {
        return Ok(None);
    };
    let emails = blob_str(repo, subtree, "emails")?
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    Ok(Some(Contributor {
        id: id.to_string(),
        name,
        emails,
    }))
}

impl Contributors for Repository {
    fn list_contributors(&self) -> Result<Vec<Contributor>, git2::Error> {
        let reference = match self.find_reference(CONTRIBUTORS_REF) {
            Ok(r) => r,
            Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let tree = reference.peel_to_commit()?.tree()?;
        let mut contributors = Vec::new();
        for entry in tree.iter() {
            let Some(id) = entry.name() else { continue };
            if let Some(c) = contributor_from_tree(self, &tree, id)? {
                contributors.push(c);
            }
        }
        contributors.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(contributors)
    }

    fn find_contributor(&self, id: &str) -> Result<Option<Contributor>, git2::Error> {
        let reference = match self.find_reference(CONTRIBUTORS_REF) {
            Ok(r) => r,
            Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let tree = reference.peel_to_commit()?.tree()?;
        contributor_from_tree(self, &tree, id)
    }

    fn find_contributor_by_email(&self, email: &str) -> Result<Option<Contributor>, git2::Error> {
        Ok(self
            .list_contributors()?
            .into_iter()
            .find(|c| c.emails.iter().any(|e| e == email)))
    }

    fn add_contributor(&self, id: &str, name: &str, emails: &[String]) -> Result<(), git2::Error> {
        let existing_commit = match self.find_reference(CONTRIBUTORS_REF) {
            Ok(r) => Some(r.peel_to_commit()?),
            Err(e) if e.code() == git2::ErrorCode::NotFound => None,
            Err(e) => return Err(e),
        };

        let existing_tree = existing_commit.as_ref().map(|c| c.tree()).transpose()?;

        if let Some(ref tree) = existing_tree {
            if tree.get_name(id).is_some() {
                return Err(git2::Error::from_str(&format!(
                    "contributor '{id}' already exists"
                )));
            }
        }

        let name_blob = self.blob(name.as_bytes())?;
        let emails_blob = self.blob(emails.join("\n").as_bytes())?;

        let contributor_tree_oid = {
            let mut tb = self.treebuilder(None)?;
            tb.insert("name", name_blob, 0o100_644)?;
            tb.insert("emails", emails_blob, 0o100_644)?;
            tb.write()?
        };

        let root_tree_oid = {
            let mut tb = self.treebuilder(existing_tree.as_ref())?;
            tb.insert(id, contributor_tree_oid, 0o040_000)?;
            tb.write()?
        };

        let tree = self.find_tree(root_tree_oid)?;
        let sig = self.signature()?;
        let message = format!("add contributor {id}");
        let parents: &[&git2::Commit<'_>] = match existing_commit.as_ref() {
            Some(c) => &[c],
            None => &[],
        };
        self.commit(Some(CONTRIBUTORS_REF), &sig, &sig, &message, &tree, parents)?;

        Ok(())
    }

    fn update_contributor(
        &self,
        id: &str,
        name: Option<&str>,
        add_emails: &[String],
        remove_emails: &[String],
    ) -> Result<(), git2::Error> {
        let reference = match self.find_reference(CONTRIBUTORS_REF) {
            Ok(r) => r,
            Err(e) if e.code() == git2::ErrorCode::NotFound => {
                return Err(git2::Error::from_str(&format!(
                    "contributor '{id}' not found"
                )));
            }
            Err(e) => return Err(e),
        };
        let existing_commit = reference.peel_to_commit()?;
        let existing_tree = existing_commit.tree()?;

        let mut contributor = contributor_from_tree(self, &existing_tree, id)?.ok_or_else(
            || git2::Error::from_str(&format!("contributor '{id}' not found")),
        )?;

        if let Some(n) = name {
            contributor.name = n.to_string();
        }
        for e in add_emails {
            if !contributor.emails.contains(e) {
                contributor.emails.push(e.clone());
            }
        }
        contributor.emails.retain(|e| !remove_emails.contains(e));

        let name_blob = self.blob(contributor.name.as_bytes())?;
        let emails_blob = self.blob(contributor.emails.join("\n").as_bytes())?;

        let contributor_tree_oid = {
            let mut tb = self.treebuilder(None)?;
            tb.insert("name", name_blob, 0o100_644)?;
            tb.insert("emails", emails_blob, 0o100_644)?;
            tb.write()?
        };

        let root_tree_oid = {
            let mut tb = self.treebuilder(Some(&existing_tree))?;
            tb.insert(id, contributor_tree_oid, 0o040_000)?;
            tb.write()?
        };

        let tree = self.find_tree(root_tree_oid)?;
        let sig = self.signature()?;
        let message = format!("update contributor {id}");
        self.commit(
            Some(CONTRIBUTORS_REF),
            &sig,
            &sig,
            &message,
            &tree,
            &[&existing_commit],
        )?;

        Ok(())
    }

    fn remove_contributor(&self, id: &str) -> Result<(), git2::Error> {
        let reference = match self.find_reference(CONTRIBUTORS_REF) {
            Ok(r) => r,
            Err(e) if e.code() == git2::ErrorCode::NotFound => {
                return Err(git2::Error::from_str(&format!(
                    "contributor '{id}' not found"
                )));
            }
            Err(e) => return Err(e),
        };
        let existing_commit = reference.peel_to_commit()?;
        let existing_tree = existing_commit.tree()?;

        if existing_tree.get_name(id).is_none() {
            return Err(git2::Error::from_str(&format!(
                "contributor '{id}' not found"
            )));
        }

        let root_tree_oid = {
            let mut tb = self.treebuilder(Some(&existing_tree))?;
            tb.remove(id)?;
            tb.write()?
        };

        let tree = self.find_tree(root_tree_oid)?;
        let sig = self.signature()?;
        let message = format!("remove contributor {id}");
        self.commit(
            Some(CONTRIBUTORS_REF),
            &sig,
            &sig,
            &message,
            &tree,
            &[&existing_commit],
        )?;

        Ok(())
    }
}
