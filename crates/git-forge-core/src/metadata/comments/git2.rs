//! `git2::Repository` implementation of [`Comments`].

use git2::Repository;

use crate::metadata::comments::{Comment, Comments, NewComment, Reanchor};

impl Comments for Repository {
    fn comments_on_blob(&self, _blob_oid: git2::Oid) -> Result<Vec<Comment>, git2::Error> {
        todo!()
    }

    fn find_comment(
        &self,
        _blob_oid: git2::Oid,
        _id: &str,
    ) -> Result<Option<Comment>, git2::Error> {
        todo!()
    }

    fn add_comment(&self, _comment: &NewComment) -> Result<String, git2::Error> {
        todo!()
    }

    fn reply_to_comment(
        &self,
        _blob_oid: git2::Oid,
        _comment_id: &str,
        _author: &str,
        _body: &str,
    ) -> Result<(), git2::Error> {
        todo!()
    }

    fn resolve_comment(
        &self,
        _blob_oid: git2::Oid,
        _comment_id: &str,
        _resolver: &str,
    ) -> Result<(), git2::Error> {
        todo!()
    }

    fn reanchor_comment(&self, _reanchor: &Reanchor) -> Result<(), git2::Error> {
        todo!()
    }

    fn orphaned_comments(&self) -> Result<Vec<Comment>, git2::Error> {
        todo!()
    }
}
