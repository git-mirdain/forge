//! `git2::Repository` implementation of [`Reviews`].

use git2::Repository;

use crate::indices::reviews::{NewReview, Review, ReviewState, ReviewUpdate, Reviews};

impl Reviews for Repository {
    fn list_reviews(&self) -> Result<Vec<Review>, git2::Error> {
        todo!()
    }

    fn list_reviews_by_state(&self, _state: ReviewState) -> Result<Vec<Review>, git2::Error> {
        todo!()
    }

    fn find_review(&self, _id: u64) -> Result<Option<Review>, git2::Error> {
        todo!()
    }

    fn create_review(&self, _review: &NewReview) -> Result<u64, git2::Error> {
        todo!()
    }

    fn update_review(&self, _id: u64, _update: &ReviewUpdate) -> Result<(), git2::Error> {
        todo!()
    }

    fn add_revision(&self, _id: u64, _head_commit: git2::Oid) -> Result<(), git2::Error> {
        todo!()
    }

    fn revision_range(
        &self,
        _review: &Review,
        _revision_index: usize,
    ) -> Result<(git2::Oid, git2::Oid), git2::Error> {
        todo!()
    }
}
