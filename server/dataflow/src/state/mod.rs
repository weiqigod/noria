mod keyed_state;
mod memory_state;
mod mk_key;
mod persistent_state;
mod single_state;

use std::borrow::Cow;
use std::ops::Deref;
use std::rc::Rc;
use std::vec;

use crate::prelude::*;
use ahash::RandomState;
use common::SizeOf;
use hashbag::HashBag;

pub(crate) use self::memory_state::MemoryState;
pub(crate) use self::persistent_state::PersistentState;

pub(crate) trait State: SizeOf + Send {
    /// Add an index keyed by the given columns and replayed to by the given partial tags.
    fn add_key(&mut self, columns: &[usize], partial: Option<Vec<Tag>>);

    /// Returns whether this state is currently keyed on anything. If not, then it cannot store any
    /// infromation and is thus "not useful".
    fn is_useful(&self) -> bool;

    fn is_partial(&self) -> bool;

    // Inserts or removes each record into State. Records that miss all indices in partial state
    // are removed from `records` (thus the mutable reference).
    fn process_records(&mut self, records: &mut Records, partial_tag: Option<Tag>);

    fn mark_hole(&mut self, key: &[DataType], tag: Tag);

    fn mark_filled(&mut self, key: Vec<DataType>, tag: Tag);

    fn lookup<'a>(&'a self, columns: &[usize], key: &KeyType) -> LookupResult<'a>;

    /// The number of rows stored in this state.
    ///
    /// Note that this is not necessarily the number of _keys_, and may bear little or no
    /// connection to the number of entries in the state's in-memory storage. For example, if the
    /// state's index is over a column that all rows share a single value for, the number of
    /// entries in the state may be 1 even though it holds a thousand rows. In this case, `len`
    /// would return `1000`.
    fn len(&self) -> usize;

    fn keys(&self) -> Vec<Vec<usize>>;

    /// Return a copy of all records. Panics if the state is only partially materialized.
    fn cloned_records(&self) -> Vec<Vec<DataType>>;

    /// Evict `bytes` bytes of state by randomly evicting keys, returning key colunms of the index
    /// chosen to evict from along with the keys evicted and the number of bytes evicted.
    ///
    /// The `fraction` argument allows the implementation of `evict_random_keys` to evict evenly
    /// from all underlying indices.
    ///
    /// The `spread` argument is used to spread multiple calls to eviction across different indices
    /// if possible. If you increment `spread` before each call to `evict_random_keys`, successive
    /// calls will go to distinct indices.
    fn evict_random_keys(
        &mut self,
        bytes: usize,
        fraction: &mut f64,
        spread: usize,
    ) -> (&[usize], Vec<Vec<DataType>>, u64);

    /// Evict the listed keys from the materialization targeted by `tag`, returning the key columns
    /// of the index that was evicted from and the number of bytes evicted.
    fn evict_keys(&mut self, tag: Tag, keys: &[Vec<DataType>]) -> Option<(&[usize], u64)>;

    fn clear(&mut self);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct Row(Rc<Vec<DataType>>);

pub(crate) type Rows = HashBag<Row, RandomState>;

unsafe impl Send for Row {}

impl From<Rc<Vec<DataType>>> for Row {
    fn from(r: Rc<Vec<DataType>>) -> Self {
        Self(r)
    }
}

impl AsRef<[DataType]> for Row {
    fn as_ref(&self) -> &[DataType] {
        &**self.0
    }
}

impl std::borrow::Borrow<[DataType]> for Row {
    fn borrow(&self) -> &[DataType] {
        &**self.0
    }
}

impl Deref for Row {
    type Target = Vec<DataType>;
    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}
impl SizeOf for Row {
    fn size_of(&self) -> u64 {
        use std::mem::size_of;
        size_of::<Self>() as u64
    }
    fn deep_size_of(&self) -> u64 {
        (*self.0).deep_size_of()
    }
    fn is_empty(&self) -> bool {
        false
    }
}

/// An std::borrow::Cow-like wrapper around a collection of rows.
pub(crate) enum RecordResult<'a> {
    Borrowed(&'a HashBag<Row, RandomState>),
    Owned(Vec<Vec<DataType>>),
}

impl<'a> RecordResult<'a> {
    pub(crate) fn len(&self) -> usize {
        match *self {
            RecordResult::Borrowed(rs) => rs.len(),
            RecordResult::Owned(ref rs) => rs.len(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        match *self {
            RecordResult::Borrowed(rs) => rs.is_empty(),
            RecordResult::Owned(ref rs) => rs.is_empty(),
        }
    }
}

impl<'a> IntoIterator for RecordResult<'a> {
    type Item = Cow<'a, [DataType]>;
    type IntoIter = RecordResultIterator<'a>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            RecordResult::Borrowed(rs) => RecordResultIterator::Borrowed(rs.iter()),
            RecordResult::Owned(rs) => RecordResultIterator::Owned(rs.into_iter()),
        }
    }
}

pub(crate) enum RecordResultIterator<'a> {
    Owned(vec::IntoIter<Vec<DataType>>),
    Borrowed(hashbag::Iter<'a, Row>),
}

impl<'a> Iterator for RecordResultIterator<'a> {
    type Item = Cow<'a, [DataType]>;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            RecordResultIterator::Borrowed(iter) => iter.next().map(|r| Cow::from(&r[..])),
            RecordResultIterator::Owned(iter) => iter.next().map(Cow::from),
        }
    }
}

pub(crate) enum LookupResult<'a> {
    Some(RecordResult<'a>),
    Missing,
}
