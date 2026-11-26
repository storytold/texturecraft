/// What a cached map holds, to keep the cache within [`CACHE_BYTES`](super::CACHE_BYTES)
pub(super) trait CachedBytes {
    fn bytes(&self) -> usize;
}
