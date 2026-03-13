//! `git2::Repository` implementation of [`Queue`].

use git2::Repository;

use crate::enforcement::queue::{NewQueueEntry, Queue, QueueEntry};

impl Queue for Repository {
    fn list_queue(&self, _name: &str) -> Result<Vec<QueueEntry>, git2::Error> {
        todo!()
    }

    fn peek_queue(&self, _name: &str) -> Result<Option<QueueEntry>, git2::Error> {
        todo!()
    }

    fn push_queue(&self, _name: &str, _entry: &NewQueueEntry) -> Result<String, git2::Error> {
        todo!()
    }

    fn pop_queue(&self, _name: &str) -> Result<Option<QueueEntry>, git2::Error> {
        todo!()
    }

    fn remove_queue_entry(&self, _name: &str, _key: &str) -> Result<(), git2::Error> {
        todo!()
    }
}
