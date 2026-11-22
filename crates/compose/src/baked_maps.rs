use std::borrow::Cow;
use std::collections::BTreeMap;

use rayon::prelude::*;
use texturecraft_core::{BakedMap, BlockScales};

/// A baked map's texels as they are held: floats, or 16-bit steps (`q / 65535`) when every value is exactly one, which
/// halves the map and reads back the very same floats (most baked maps are written from 16-bit images; tangent-space
/// normal maps usually are not, and stay floats).
///
/// A map opened from a project file can be [`Texels::lazy`]: read from the file when first needed, and let go again
/// ([`Texels::release`]) while it is unchanged.
#[derive(Clone, Debug)]
pub enum Texels {
    F32(Vec<f32>),
    U16(Vec<u16>),
    Lazy(LazyTexels),
}

/// Reads a baked map's values again (from the file it was opened from); `None` when they cannot be read any more.
pub type TexelSource = std::sync::Arc<dyn Fn() -> Option<Vec<f32>> + Send + Sync>;

/// See [`Texels::lazy`].
#[derive(Clone)]
pub struct LazyTexels {
    len: usize,
    held: std::sync::OnceLock<std::sync::Arc<Texels>>,
    source: TexelSource,
}

impl std::fmt::Debug for LazyTexels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazyTexels").field("len", &self.len).field("held", &self.held.get().is_some()).finish()
    }
}

impl LazyTexels {
    /// The values read now (16-bit steps where exact); zeros when they cannot be read or are not `len` values.
    fn read(&self) -> Texels {
        match (self.source)() {
            Some(values) if values.len() == self.len => Texels::new(values),
            _ => Texels::F32(vec![0.0; self.len]),
        }
    }

    fn held(&self) -> &Texels {
        self.held.get_or_init(|| std::sync::Arc::new(self.read()))
    }
}

/// The 16-bit step `q` as a float (the one way it is read back, so the exactness check and the reads agree bit for bit).
#[inline]
fn step(q: u16) -> f32 {
    f32::from(q) / 65535.0
}

/// `v` as a 16-bit step when it is exactly one.
#[inline]
fn exact_step(v: f32) -> Option<u16> {
    let q = (v * 65535.0).round();
    if !(0.0..=65535.0).contains(&q) {
        return None;
    }
    let q = q as u16;
    (step(q).to_bits() == v.to_bits()).then_some(q)
}

impl Texels {
    /// `data`, held as 16-bit steps when every value is exactly one, else as it is.
    pub fn new(data: Vec<f32>) -> Texels {
        const CHUNK: usize = 1 << 16;
        let exact = data.par_chunks(CHUNK).all(|c| c.iter().all(|v| exact_step(*v).is_some()));
        if !exact || data.is_empty() {
            return Texels::F32(data);
        }
        let mut steps = vec![0u16; data.len()];
        steps.par_chunks_mut(CHUNK).zip(data.par_chunks(CHUNK)).for_each(|(out, c)| {
            for (o, v) in out.iter_mut().zip(c) {
                *o = exact_step(*v).unwrap_or_default();
            }
        });
        Texels::U16(steps)
    }

    /// `len` values that `source` reads when they are first needed.
    pub fn lazy(len: usize, source: TexelSource) -> Texels {
        Texels::Lazy(LazyTexels { len, held: std::sync::OnceLock::new(), source })
    }

    /// Lets go of the values of a lazy map; they are read again when next needed. Returns the bytes let go.
    pub fn release(&mut self) -> usize {
        match self {
            Texels::Lazy(l) => {
                let bytes = l.held.get().map_or(0, |h| h.bytes());
                l.held = std::sync::OnceLock::new();
                bytes
            }
            _ => 0,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Texels::F32(d) => d.len(),
            Texels::U16(d) => d.len(),
            Texels::Lazy(l) => l.len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Bytes held.
    pub fn bytes(&self) -> usize {
        match self {
            Texels::F32(d) => d.len() * 4,
            Texels::U16(d) => d.len() * 2,
            Texels::Lazy(l) => l.held.get().map_or(0, |h| h.bytes()),
        }
    }

    /// Value `i`.
    #[inline]
    pub fn get(&self, i: usize) -> Option<f32> {
        match self {
            Texels::F32(d) => d.get(i).copied(),
            Texels::U16(d) => d.get(i).map(|q| step(*q)),
            Texels::Lazy(l) => l.held().get(i),
        }
    }

    /// The values as floats (borrowed when they are held as floats).
    pub fn to_f32(&self) -> Cow<'_, [f32]> {
        match self {
            Texels::F32(d) => Cow::Borrowed(d),
            Texels::U16(d) => Cow::Owned(d.iter().map(|q| step(*q)).collect()),
            Texels::Lazy(l) => l.held().to_f32(),
        }
    }

    /// The values as floats, `n` at a time (the last run shorter), handed to `f` until it fails.
    pub fn try_for_chunks<E>(&self, n: usize, mut f: impl FnMut(&[f32]) -> Result<(), E>) -> Result<(), E> {
        match self {
            Texels::F32(d) => d.chunks(n.max(1)).try_for_each(f),
            // read for the call and let go after when not held (what saving does)
            Texels::Lazy(l) => match l.held.get() {
                Some(h) => h.try_for_chunks(n, f),
                None => l.read().try_for_chunks(n, f),
            },
            Texels::U16(d) => {
                let mut buf = Vec::with_capacity(n.max(1));
                for c in d.chunks(n.max(1)) {
                    buf.clear();
                    buf.extend(c.iter().map(|q| step(*q)));
                    f(&buf)?;
                }
                Ok(())
            }
        }
    }

    /// The values as floats, in order.
    pub fn iter(&self) -> impl Iterator<Item = f32> + '_ {
        let (f, u, l) = match self {
            Texels::F32(d) => (Some(d.iter().copied()), None, None),
            Texels::U16(d) => (None, Some(d.iter().map(|q| step(*q))), None),
            Texels::Lazy(l) => (None, None, Some(l.held())),
        };
        let inner: Box<dyn Iterator<Item = f32> + '_> = match l {
            Some(h) => Box::new(h.iter()),
            None => Box::new(std::iter::empty()),
        };
        f.into_iter().flatten().chain(u.into_iter().flatten()).chain(inner)
    }
}

impl From<Vec<f32>> for Texels {
    fn from(data: Vec<f32>) -> Texels {
        Texels::new(data)
    }
}

/// The same values, however they are held.
impl PartialEq for Texels {
    fn eq(&self, other: &Texels) -> bool {
        match (self, other) {
            (Texels::F32(a), Texels::F32(b)) => a == b,
            (Texels::U16(a), Texels::U16(b)) => a == b,
            _ => self.len() == other.len() && self.iter().zip(other.iter()).all(|(a, b)| a == b),
        }
    }
}

/// Dense mesh maps with `components()` values per texel in row-major order
/// Each UV tile fills its block independently of the working resolution
/// The width is `resolution`; height follows the output size and UV tile grid
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakedMaps {
    /// Width in texels
    pub resolution: u32,
    pub maps: BTreeMap<BakedMap, std::sync::Arc<Texels>>,
}

impl BakedMaps {
    /// Sample a working atlas texel without resampling the stored bake
    pub fn sample_working(&self, map: BakedMap, x: u32, y: u32, size: (u32, u32), scales: &BlockScales) -> Option<[f32; 3]> {
        let uv = [(x as f32 + 0.5) / size.0.max(1) as f32, (y as f32 + 0.5) / size.1.max(1) as f32];
        self.sample_uv(map, scales.bake_uv(uv, size)?)
    }

    /// Nearest sample in the uniform bake atlas
    pub fn sample_uv(&self, map: BakedMap, uv: [f32; 2]) -> Option<[f32; 3]> {
        if uv.iter().any(|value| !value.is_finite()) {
            return None;
        }

        let size = (self.resolution, self.height(map));
        self.sample(map, (uv[0] * size.0 as f32) as u32, (uv[1] * size.1 as f32) as u32, size)
    }

    /// Height in texels of map `m`, from its length
    pub fn height(&self, m: BakedMap) -> u32 {
        self.maps.get(&m).map_or(0, |d| (d.len() / (self.resolution.max(1) as usize * m.components())) as u32)
    }

    /// Sample the corresponding texel of an image with the supplied output size
    pub fn sample(&self, map: BakedMap, x: u32, y: u32, output_size: (u32, u32)) -> Option<[f32; 3]> {
        let data = self.maps.get(&map)?;
        let width = u64::from(self.resolution.max(1));
        let height = u64::from(self.height(map).max(1));
        let sample_x = (u64::from(x) * width / u64::from(output_size.0.max(1))).min(width - 1);
        let sample_y = (u64::from(y) * height / u64::from(output_size.1.max(1))).min(height - 1);
        let component_count = map.components();
        let offset = ((sample_y * width + sample_x) as usize).checked_mul(component_count)?;
        if component_count == 3 { Some([data.get(offset)?, data.get(offset + 1)?, data.get(offset + 2)?]) } else { Some([data.get(offset)?; 3]) }
    }

    pub fn scalar(&self, m: BakedMap, x: u32, y: u32, size: (u32, u32)) -> Option<f32> {
        self.sample(m, x, y, size).map(|v| v[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_16_bit_steps_are_held_as_steps_and_read_back_bit_for_bit() {
        let values: Vec<f32> = (0..=u16::MAX).map(|q| f32::from(q) / 65535.0).collect();
        let held = Texels::new(values.clone());
        assert!(matches!(held, Texels::U16(_)), "every value is a step");
        assert_eq!(held.bytes(), values.len() * 2);
        assert!(held.iter().zip(&values).all(|(a, b)| a.to_bits() == b.to_bits()));
        assert!(held.to_f32().iter().zip(&values).all(|(a, b)| a.to_bits() == b.to_bits()));
        assert_eq!((0..values.len()).filter_map(|i| held.get(i)).map(f32::to_bits).collect::<Vec<_>>(), values.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
        let mut chunked = Vec::new();
        held.try_for_chunks::<()>(1000, |c| {
            chunked.extend_from_slice(c);
            Ok(())
        })
        .unwrap();
        assert_eq!(chunked, values);
        assert_eq!(held, Texels::F32(values));
    }

    #[test]
    fn a_map_with_any_value_off_the_steps_stays_floats() {
        for odd in [0.5, -0.0, 1.5, -0.25, f32::NAN, f32::INFINITY, 1.0 / 65535.0 + 1e-9] {
            let held = Texels::new(vec![0.0, 1.0, odd]);
            assert!(matches!(held, Texels::F32(_)), "{odd} is not a step");
            assert_eq!(held.get(2).map(f32::to_bits), Some(odd.to_bits()));
        }
        assert!(matches!(Texels::new(Vec::new()), Texels::F32(_)));
    }

    #[test]
    fn a_map_held_as_steps_samples_as_its_floats() {
        let values: Vec<f32> = (0..16 * 8 * 3).map(|i| f32::from((i * 997 % 65536) as u16) / 65535.0).collect();
        let steps = BakedMaps { resolution: 16, maps: [(BakedMap::Position, std::sync::Arc::new(Texels::new(values.clone())))].into() };
        let floats = BakedMaps { resolution: 16, maps: [(BakedMap::Position, std::sync::Arc::new(Texels::F32(values)))].into() };
        assert!(matches!(*steps.maps[&BakedMap::Position], Texels::U16(_)));
        assert_eq!(steps.height(BakedMap::Position), 8);
        for (x, y) in [(0, 0), (5, 3), (15, 7), (16, 8)] {
            assert_eq!(steps.sample(BakedMap::Position, x, y, (16, 8)), floats.sample(BakedMap::Position, x, y, (16, 8)));
        }
        assert_eq!(steps, floats);
    }
}

#[cfg(test)]
mod lazy_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn a_lazy_map_reads_when_needed_and_again_after_it_lets_go() {
        let values: Vec<f32> = (0..300).map(|i| f32::from(i as u16 * 200) / 65535.0).collect();
        let reads = Arc::new(AtomicUsize::new(0));
        let (v, r) = (values.clone(), reads.clone());
        let mut lazy = Texels::lazy(
            values.len(),
            Arc::new(move || {
                r.fetch_add(1, Ordering::Relaxed);
                Some(v.clone())
            }),
        );
        assert_eq!((lazy.len(), lazy.bytes(), reads.load(Ordering::Relaxed)), (300, 0, 0));
        let mut chunked = Vec::new();
        lazy.try_for_chunks::<()>(64, |c| {
            chunked.extend_from_slice(c);
            Ok(())
        })
        .unwrap();
        assert_eq!(chunked, values);
        assert_eq!(lazy.bytes(), 0, "saving does not keep it");
        assert_eq!(lazy.get(7).map(f32::to_bits), Some(values[7].to_bits()));
        assert_eq!(lazy.bytes(), 600, "held as 16-bit steps");
        assert_eq!(lazy, Texels::F32(values.clone()));
        assert_eq!(lazy.release(), 600);
        assert_eq!(lazy.to_f32().as_ref(), values.as_slice());
        assert_eq!(reads.load(Ordering::Relaxed), 3);
        assert_eq!(Texels::lazy(4, Arc::new(|| None)).get(3), Some(0.0), "unreadable: zeros");
    }
}
