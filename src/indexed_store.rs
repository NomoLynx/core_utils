//! Generic indexed collection with O(1)-average key lookups.
//!
//! Stores items in insertion order and supports:
//! - index access via the backing `Vec`
//! - iteration over all items
//! - key-based lookup via `HashMap`
//!
//! Key uniqueness is enforced.

use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;
use std::ops::{Deref, Index};

#[derive(Debug)]
pub struct IndexedStore<T, K, F>
where
    K: Eq + Hash + Clone,
    F: Fn(&T) -> K,
{
    items: Vec<T>,
    by_key: HashMap<K, usize>,
    key_of: F,
}

/// String-key specialization of `IndexedStore`.
pub type StringIndexedStore<T, F> = IndexedStore<T, String, F>;

/// Creates a string-keyed indexed store without spelling out the key type.
pub fn new_string_indexed_store<T, F>(key_of: F) -> StringIndexedStore<T, F>
where
    F: Fn(&T) -> String,
{
    IndexedStore::new(key_of)
}

#[derive(Debug, PartialEq, Eq)]
pub enum InsertError<T> {
    DuplicateKey(T),
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReplaceError<T> {
    OutOfBounds(T),
    DuplicateKey(T),
}

impl<T, K, F> IndexedStore<T, K, F>
where
    K: Eq + Hash + Clone,
    F: Fn(&T) -> K,
{
    pub fn new(key_of: F) -> Self {
        Self {
            items: Vec::new(),
            by_key: HashMap::new(),
            key_of,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn as_slice(&self) -> &[T] {
        &self.items
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.items.get(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter()
    }

    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.by_key.contains_key(key)
    }

    pub fn get_by_key<Q>(&self, key: &Q) -> Option<&T>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.by_key
            .get(key)
            .and_then(|&index| self.items.get(index))
    }

    pub fn try_push(&mut self, item: T) -> Result<(), InsertError<T>> {
        let key = (self.key_of)(&item);
        if self.by_key.contains_key(&key) {
            return Err(InsertError::DuplicateKey(item));
        }
        let index = self.items.len();
        self.items.push(item);
        self.by_key.insert(key, index);
        Ok(())
    }

    /// Vec-like push that panics on duplicate keys.
    pub fn push(&mut self, item: T) {
        if let Err(InsertError::DuplicateKey(_)) = self.try_push(item) {
            panic!("duplicate key inserted into IndexedStore, key");
        }
    }

    pub fn replace_at(&mut self, index: usize, new_item: T) -> Result<T, ReplaceError<T>> {
        if index >= self.items.len() {
            return Err(ReplaceError::OutOfBounds(new_item));
        }

        let new_key = (self.key_of)(&new_item);
        if let Some(existing_index) = self.by_key.get(&new_key).copied() {
            if existing_index != index {
                return Err(ReplaceError::DuplicateKey(new_item));
            }
        }

        let old_key = (self.key_of)(&self.items[index]);
        let old_item = std::mem::replace(&mut self.items[index], new_item);

        if old_key != new_key {
            self.by_key.remove(&old_key);
            self.by_key.insert(new_key, index);
        }

        Ok(old_item)
    }

    pub fn remove_at(&mut self, index: usize) -> Option<T> {
        let len = self.items.len();
        if index >= len {
            return None;
        }

        let removed_key = (self.key_of)(&self.items[index]);
        let last_index = len - 1;
        let moved_key = if index != last_index {
            Some((self.key_of)(&self.items[last_index]))
        } else {
            None
        };

        let removed = self.items.swap_remove(index);
        self.by_key.remove(&removed_key);

        if let Some(moved_key) = moved_key {
            self.by_key.insert(moved_key, index);
        }

        Some(removed)
    }

    pub fn remove_by_key<Q>(&mut self, key: &Q) -> Option<T>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        let index = self.by_key.get(key).copied()?;
        self.remove_at(index)
    }

    pub fn into_vec(self) -> Vec<T> {
        self.items
    }
}

impl<T, K, F> Deref for IndexedStore<T, K, F>
where
    K: Eq + Hash + Clone,
    F: Fn(&T) -> K,
{
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.items
    }
}

impl<T, K, F> AsRef<[T]> for IndexedStore<T, K, F>
where
    K: Eq + Hash + Clone,
    F: Fn(&T) -> K,
{
    fn as_ref(&self) -> &[T] {
        &self.items
    }
}

impl<T, K, F> Index<usize> for IndexedStore<T, K, F>
where
    K: Eq + Hash + Clone,
    F: Fn(&T) -> K,
{
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        &self.items[index]
    }
}

impl<T, K, F, Q> Index<&Q> for IndexedStore<T, K, F>
where
    K: Eq + Hash + Clone + Borrow<Q>,
    F: Fn(&T) -> K,
    Q: Eq + Hash + ?Sized,
{
    type Output = T;

    fn index(&self, key: &Q) -> &Self::Output {
        self.get_by_key(key).expect("key not found in IndexedStore")
    }
}

impl<T, K, F> IntoIterator for IndexedStore<T, K, F>
where
    K: Eq + Hash + Clone,
    F: Fn(&T) -> K,
{
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<'a, T, K, F> IntoIterator for &'a IndexedStore<T, K, F>
where
    K: Eq + Hash + Clone,
    F: Fn(&T) -> K,
{
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}
