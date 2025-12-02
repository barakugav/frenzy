use std::mem::MaybeUninit;

pub(crate) struct SimpleHashMap<K, V, S = std::hash::RandomState> {
    hasher: S,
    table_mask: u64,
    table: Box<[Entry<K, V>]>,
    fallback: Box<std::collections::HashMap<K, V, S>>,
}
struct Entry<K, V> {
    // Vacant,
    // Occupied(NonZero<u64>, K, V),
    hash: u64,
    kv: MaybeUninit<EntryKV<K, V>>,
}
struct EntryKV<K, V> {
    key: K,
    value: V,
}
impl<K, V, S> SimpleHashMap<K, V, S> {
    pub fn new(max_capacity: usize, safety_factor: f32) -> Self
    where
        S: Default,
    {
        let max_capacity = ((max_capacity as f32 * safety_factor) as usize).next_power_of_two();
        let mut table = (0..max_capacity)
            .map(|_| Entry {
                hash: 0,
                kv: MaybeUninit::zeroed(),
            })
            .collect::<Box<[_]>>();
        table[0].hash = 1;
        let table_mask = (max_capacity - 1) as u64;

        let fallback_capacity = (max_capacity / 64).min(64);
        Self {
            table_mask,
            table,
            hasher: S::default(),
            fallback: Box::new(std::collections::HashMap::with_capacity_and_hasher(
                fallback_capacity,
                S::default(),
            )),
        }
    }

    #[inline(always)]
    pub fn get_or_default<'a>(&'a mut self, key: impl Key<K>) -> &'a mut V
    where
        K: std::hash::Hash + Eq,
        V: Default,
        S: std::hash::BuildHasher,
    {
        let pair = key.into_key_and_hash(&self.hasher);
        let (key, hash) = (pair.key, pair.hash);

        let bucket = (hash & self.table_mask) as usize;
        if std::hint::likely(self.table[bucket].hash == hash) {
            if std::hint::likely(key == unsafe { self.table[bucket].kv.assume_init_ref() }.key) {
                return &mut unsafe { self.table[bucket].kv.assume_init_mut() }.value;
            }
        }

        if std::hint::unlikely(self.table[bucket].hash != 0) {
            // - if bucket is occupied by a different key
            // - if we hit bucket 0, possibly because the key hash is 0
            return self.get_or_default_fallback(key);
        }

        self.table[bucket] = Entry {
            hash,
            kv: MaybeUninit::new(EntryKV {
                key,
                value: V::default(),
            }),
        };
        return &mut unsafe { self.table[bucket].kv.assume_init_mut() }.value;
    }

    #[inline(never)]
    #[cold]
    fn get_or_default_fallback<'a>(&'a mut self, key: K) -> &'a mut V
    where
        K: std::hash::Hash + Eq,
        V: Default,
        S: std::hash::BuildHasher,
    {
        self.fallback.entry(key).or_default()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        struct Iter<'a, K, V> {
            table: &'a [Entry<K, V>],
            idx: usize,
        }
        impl<'a, K, V> Iterator for Iter<'a, K, V> {
            type Item = (&'a K, &'a V);

            fn next(&mut self) -> Option<Self::Item> {
                while self.idx < self.table.len() {
                    let entry = &self.table[self.idx];
                    self.idx += 1;
                    if entry.hash != 0 {
                        let kv = &unsafe { entry.kv.assume_init_ref() };
                        return Some((&kv.key, &kv.value));
                    }
                }
                None
            }
        }
        let table_iter = Iter {
            table: &self.table,
            idx: 1, // we dont use bucket 0
        };
        table_iter.chain(self.fallback.iter())
    }

    pub(crate) fn hasher(&self) -> S::Hasher
    where
        S: std::hash::BuildHasher,
    {
        self.hasher.build_hasher()
    }

    pub(crate) fn fallback_size(&self) -> usize {
        self.fallback.len()
    }
}

pub(crate) struct KeyHashPair<K> {
    key: K,
    hash: u64,
}
impl<K> KeyHashPair<K> {
    pub unsafe fn new_unchecked(key: K, hash: u64) -> Self {
        Self { key, hash }
    }
}

pub(crate) trait Key<K> {
    fn into_key_and_hash<S>(self, hasher: &S) -> KeyHashPair<K>
    where
        S: std::hash::BuildHasher;
}
impl<K> Key<K> for K
where
    K: std::hash::Hash,
{
    fn into_key_and_hash<S>(self, hasher: &S) -> KeyHashPair<K>
    where
        S: std::hash::BuildHasher,
    {
        let hash = hasher.hash_one(&self);
        KeyHashPair { key: self, hash }
    }
}
impl<K> Key<K> for KeyHashPair<K>
where
    K: std::hash::Hash,
{
    fn into_key_and_hash<S>(self, hasher: &S) -> KeyHashPair<K>
    where
        S: std::hash::BuildHasher,
    {
        debug_assert_eq!(self.hash, hasher.hash_one(&self.key));
        self
    }
}
