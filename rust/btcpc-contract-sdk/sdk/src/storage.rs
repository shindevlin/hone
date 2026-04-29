//! Typed persistent storage collections backed by the host KV store.

use borsh::{BorshDeserialize, BorshSerialize};
use borsh::io::{self, Read, Write};
use crate::env::{storage_read, storage_write, storage_remove, storage_has_key};

pub use crate::types::StoragePrefix;

/// Trait for types that can be used as storage keys.
pub trait StorageKey: BorshSerialize {}
impl<T: BorshSerialize> StorageKey for T {}

// ── LookupMap ────────────────────────────────────────────────────────────────

/// An ordered key-value store. Keys are not enumerable.
/// Lowest gas: use when you only need get/set/remove.
pub struct LookupMap<K, V> {
    prefix: Vec<u8>,
    _phantom: core::marker::PhantomData<(K, V)>,
}

impl<K, V> BorshSerialize for LookupMap<K, V> {
    fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.prefix.serialize(writer)
    }
}

impl<K, V> BorshDeserialize for LookupMap<K, V> {
    fn deserialize_reader<R: Read>(reader: &mut R) -> io::Result<Self> {
        let prefix = Vec::<u8>::deserialize_reader(reader)?;
        Ok(Self { prefix, _phantom: core::marker::PhantomData })
    }
}

impl<K: BorshSerialize, V: BorshSerialize + BorshDeserialize> LookupMap<K, V> {
    pub fn new(prefix: impl BorshSerialize) -> Self {
        let mut buf = Vec::new();
        prefix.serialize(&mut buf).unwrap();
        Self { prefix: buf, _phantom: core::marker::PhantomData }
    }

    fn make_key(&self, key: &K) -> Vec<u8> {
        let mut k = self.prefix.clone();
        key.serialize(&mut k).unwrap();
        k
    }

    pub fn get(&self, key: &K) -> Option<V> {
        let raw_key = self.make_key(key);
        storage_read(&raw_key).and_then(|bytes| V::try_from_slice(&bytes).ok())
    }

    pub fn insert(&mut self, key: &K, value: &V) {
        let raw_key = self.make_key(key);
        let mut val_buf = Vec::new();
        value.serialize(&mut val_buf).unwrap();
        storage_write(&raw_key, &val_buf);
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let raw_key = self.make_key(key);
        let existing = storage_read(&raw_key).and_then(|b| V::try_from_slice(&b).ok());
        if existing.is_some() {
            storage_remove(&raw_key);
        }
        existing
    }

    pub fn contains_key(&self, key: &K) -> bool {
        storage_has_key(&self.make_key(key))
    }
}

// ── LazyOption ───────────────────────────────────────────────────────────────

/// A single lazily-loaded optional value.
pub struct LazyOption<V> {
    key: Vec<u8>,
    _phantom: core::marker::PhantomData<V>,
}

impl<V> BorshSerialize for LazyOption<V> {
    fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.key.serialize(writer)
    }
}

impl<V> BorshDeserialize for LazyOption<V> {
    fn deserialize_reader<R: Read>(reader: &mut R) -> io::Result<Self> {
        let key = Vec::<u8>::deserialize_reader(reader)?;
        Ok(Self { key, _phantom: core::marker::PhantomData })
    }
}

impl<V: BorshSerialize + BorshDeserialize> LazyOption<V> {
    pub fn new(key: impl BorshSerialize, default: Option<V>) -> Self {
        let mut k = Vec::new();
        key.serialize(&mut k).unwrap();
        let opt = Self { key: k, _phantom: core::marker::PhantomData };
        if let Some(val) = default {
            let mut buf = Vec::new();
            val.serialize(&mut buf).unwrap();
            storage_write(&opt.key, &buf);
        }
        opt
    }

    pub fn get(&self) -> Option<V> {
        storage_read(&self.key).and_then(|b| V::try_from_slice(&b).ok())
    }

    pub fn set(&mut self, value: &V) {
        let mut buf = Vec::new();
        value.serialize(&mut buf).unwrap();
        storage_write(&self.key, &buf);
    }

    pub fn take(&mut self) -> Option<V> {
        let existing = self.get();
        if existing.is_some() {
            storage_remove(&self.key);
        }
        existing
    }
}

// ── Vector ───────────────────────────────────────────────────────────────────

/// An append-only indexed sequence backed by storage.
pub struct Vector<V> {
    prefix: Vec<u8>,
    len: u64,
    _phantom: core::marker::PhantomData<V>,
}

impl<V> BorshSerialize for Vector<V> {
    fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.prefix.serialize(writer)?;
        self.len.serialize(writer)
    }
}

impl<V> BorshDeserialize for Vector<V> {
    fn deserialize_reader<R: Read>(reader: &mut R) -> io::Result<Self> {
        let prefix = Vec::<u8>::deserialize_reader(reader)?;
        let len = u64::deserialize_reader(reader)?;
        Ok(Self { prefix, len, _phantom: core::marker::PhantomData })
    }
}

impl<V: BorshSerialize + BorshDeserialize> Vector<V> {
    pub fn new(prefix: impl BorshSerialize) -> Self {
        let mut p = Vec::new();
        prefix.serialize(&mut p).unwrap();
        // Read length from storage if it exists
        let len_key = [p.as_slice(), b"__len"].concat();
        let len = storage_read(&len_key)
            .and_then(|b| u64::try_from_slice(&b).ok())
            .unwrap_or(0);
        Self { prefix: p, len, _phantom: core::marker::PhantomData }
    }

    fn index_key(&self, index: u64) -> Vec<u8> {
        let mut k = self.prefix.clone();
        index.serialize(&mut k).unwrap();
        k
    }

    pub fn push(&mut self, value: &V) {
        let key = self.index_key(self.len);
        let mut buf = Vec::new();
        value.serialize(&mut buf).unwrap();
        storage_write(&key, &buf);
        self.len += 1;
        let len_key = [self.prefix.as_slice(), b"__len"].concat();
        let mut len_buf = Vec::new();
        self.len.serialize(&mut len_buf).unwrap();
        storage_write(&len_key, &len_buf);
    }

    pub fn get(&self, index: u64) -> Option<V> {
        if index >= self.len { return None; }
        storage_read(&self.index_key(index)).and_then(|b| V::try_from_slice(&b).ok())
    }

    pub fn len(&self) -> u64 { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
}

// ── UnorderedMap ─────────────────────────────────────────────────────────────

/// Key-value store with iteration support. Higher gas than LookupMap.
/// Maintains a separate index for enumeration.
pub struct UnorderedMap<K, V> {
    prefix: Vec<u8>,
    keys: Vector<K>,
    values: LookupMap<K, V>,
}

impl<K, V> BorshSerialize for UnorderedMap<K, V> {
    fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.prefix.serialize(writer)
    }
}

impl<K, V> BorshDeserialize for UnorderedMap<K, V>
where
    K: BorshSerialize + BorshDeserialize + Clone,
    V: BorshSerialize + BorshDeserialize,
{
    fn deserialize_reader<R: Read>(reader: &mut R) -> io::Result<Self> {
        let prefix = Vec::<u8>::deserialize_reader(reader)?;
        Ok(Self::new(prefix))
    }
}

impl<K: BorshSerialize + BorshDeserialize + Clone, V: BorshSerialize + BorshDeserialize>
    UnorderedMap<K, V>
{
    pub fn new(prefix: impl BorshSerialize) -> Self {
        let mut p = Vec::new();
        prefix.serialize(&mut p).unwrap();
        let keys: Vector<K> = Vector::new([p.as_slice(), b"k"].concat());
        let values: LookupMap<K, V> = LookupMap::new([p.as_slice(), b"v"].concat());
        Self { prefix: p, keys, values }
    }

    pub fn insert(&mut self, key: &K, value: &V) {
        if !self.values.contains_key(key) {
            self.keys.push(key);
        }
        self.values.insert(key, value);
    }

    pub fn get(&self, key: &K) -> Option<V> {
        self.values.get(key)
    }

    pub fn len(&self) -> u64 { self.keys.len() }
    pub fn is_empty(&self) -> bool { self.keys.is_empty() }

    pub fn keys_as_vec(&self) -> Vec<K> {
        (0..self.keys.len())
            .filter_map(|i| self.keys.get(i))
            .collect()
    }
}
