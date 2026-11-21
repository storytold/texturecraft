use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use texturecraft_geom::{Mesh, NO_TRI, UvGBuffer};

const CELL: f32 = 8.0;
const MAX_LINKS: usize = 4_000_000;
const MAX_CROSSINGS: usize = 8;
const MIN_EDGE: f32 = 1e-6;
const SAME_UV: f32 = 1e-3;

#[derive(Clone, Copy, Debug)]
struct Link {
    a: [f32; 2],
    b: [f32; 2],
    a2: [f32; 2],
    b2: [f32; 2],
    n: [f32; 2],
    m: [f32; 4],
}

#[derive(Debug, Default)]
pub struct SeamLinks {
    links: Vec<Link>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

fn sub(p: [f32; 2], q: [f32; 2]) -> [f32; 2] {
    [p[0] - q[0], p[1] - q[1]]
}

fn dot(p: [f32; 2], q: [f32; 2]) -> f32 {
    p[0] * q[0] + p[1] * q[1]
}

fn cell_of(p: [f32; 2]) -> (i32, i32) {
    ((p[0] / CELL).floor() as i32, (p[1] / CELL).floor() as i32)
}

type PosKey = [u32; 3];

fn pos_key(mesh: &Mesh, vertex: usize) -> Option<PosKey> {
    let p = mesh.positions.get(vertex)?;
    Some([p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
}

impl SeamLinks {
    pub fn build(mesh: &Mesh, gbuf: &UvGBuffer) -> SeamLinks {
        let size = [gbuf.width as f32, gbuf.height as f32];
        let mut present = vec![false; mesh.triangle_count()];
        for &t in &gbuf.tri {
            if t != NO_TRI
                && let Some(slot) = present.get_mut(t as usize)
            {
                *slot = true;
            }
        }
        let corner = |t: usize, c: usize| -> Option<([f32; 2], PosKey)> {
            let tri = mesh.tri(t)?;
            let v = *tri.get(c)?;
            let uv = mesh.uvs.get(v)?;
            Some(([uv.x * size[0] - 0.5, uv.y * size[1] - 0.5], pos_key(mesh, v)?))
        };
        let mut edges: HashMap<(PosKey, PosKey), Vec<(usize, usize)>> = HashMap::new();
        for t in (0..present.len()).filter(|t| present[*t]) {
            for e in 0..3 {
                let (Some((_, k0)), Some((_, k1))) = (corner(t, e), corner(t, (e + 1) % 3)) else { continue };
                if k0 == k1 {
                    continue;
                }
                edges.entry((k0.min(k1), k0.max(k1))).or_default().push((t, e));
            }
        }
        let outward = |t: usize, e: usize, a: [f32; 2], b: [f32; 2]| -> Option<([f32; 2], [f32; 2], f32)> {
            let len = dot(sub(b, a), sub(b, a)).sqrt();
            if len.is_nan() || len <= MIN_EDGE {
                return None;
            }
            let tg = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
            let third = corner(t, (e + 2) % 3)?.0;
            let mut n = [tg[1], -tg[0]];
            if dot(n, sub(third, a)) > 0.0 {
                n = [-n[0], -n[1]];
            }
            Some((tg, n, len))
        };
        let mut out = SeamLinks::default();
        for pair in edges.values().filter(|v| v.len() == 2) {
            for (&(t, e), &(t2, e2)) in [(&pair[0], &pair[1]), (&pair[1], &pair[0])] {
                if out.links.len() >= MAX_LINKS {
                    return out.indexed();
                }
                let (Some((a, ka)), Some((b, _)), Some((c, kc)), Some((d, _))) =
                    (corner(t, e), corner(t, (e + 1) % 3), corner(t2, e2), corner(t2, (e2 + 1) % 3))
                else {
                    continue;
                };
                let (a2, b2) = if ka == kc { (c, d) } else { (d, c) };
                let da = sub(a, a2);
                let db = sub(b, b2);
                if dot(da, da).sqrt() < SAME_UV && dot(db, db).sqrt() < SAME_UV {
                    continue;
                }
                let (Some((tg, n, _)), Some((tg2, n2, _))) = (outward(t, e, a, b), outward(t2, e2, a2, b2)) else { continue };
                let m = [tg2[0] * tg[0] - n2[0] * n[0], tg2[0] * tg[1] - n2[0] * n[1], tg2[1] * tg[0] - n2[1] * n[0], tg2[1] * tg[1] - n2[1] * n[1]];
                out.links.push(Link { a, b, a2, b2, n, m });
            }
        }
        out.indexed()
    }

    fn indexed(mut self) -> SeamLinks {
        for (i, l) in self.links.iter().enumerate() {
            let v = sub(l.b, l.a);
            let len = dot(v, v).sqrt();
            let steps = (len / (CELL * 0.5)).ceil().max(1.0) as usize;
            for s in 0..=steps {
                let k = s as f32 / steps as f32;
                let p = [l.a[0] + v[0] * k, l.a[1] + v[1] * k];
                if !(p[0].is_finite() && p[1].is_finite()) {
                    continue;
                }
                let (cx, cy) = cell_of(p);
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        let list = self.cells.entry((cx + dx, cy + dy)).or_default();
                        if list.last() != Some(&(i as u32)) {
                            list.push(i as u32);
                        }
                    }
                }
            }
        }
        for list in self.cells.values_mut() {
            list.sort_unstable();
            list.dedup();
        }
        self
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    fn first_hit(&self, from: [f32; 2], to: [f32; 2]) -> Option<(f32, [f32; 2], usize)> {
        let v = sub(to, from);
        let len = dot(v, v).sqrt();
        let steps = (len / (CELL * 0.5)).ceil().max(1.0) as usize;
        let mut best: Option<(f32, [f32; 2], usize)> = None;
        let mut last = None;
        for s in 0..=steps {
            let k = s as f32 / steps as f32;
            let c = cell_of([from[0] + v[0] * k, from[1] + v[1] * k]);
            if last == Some(c) {
                continue;
            }
            last = Some(c);
            let Some(list) = self.cells.get(&c) else { continue };
            for &i in list {
                let Some(l) = self.links.get(i as usize) else { continue };
                let (s0, s1) = (dot(sub(from, l.a), l.n), dot(sub(to, l.a), l.n));
                if !(s0 <= 0.0 && s1 > 0.0) {
                    continue;
                }
                let u = s0 / (s0 - s1);
                let q = [from[0] + v[0] * u, from[1] + v[1] * u];
                let ab = sub(l.b, l.a);
                let along = dot(sub(q, l.a), ab) / dot(ab, ab).max(f32::MIN_POSITIVE);
                if !(0.0..=1.0).contains(&along) || best.is_some_and(|b| b.0 <= u) {
                    continue;
                }
                let q2 = [l.a2[0] + (l.b2[0] - l.a2[0]) * along, l.a2[1] + (l.b2[1] - l.a2[1]) * along];
                best = Some((u, q2, i as usize));
            }
        }
        best
    }

    pub fn walker(&self, origin: [f32; 2], dir: [f32; 2]) -> Walker<'_> {
        Walker { links: self, origin, dir, base: 0.0, at: origin, at_t: 0.0 }
    }
}

pub struct Walker<'a> {
    links: &'a SeamLinks,
    origin: [f32; 2],
    dir: [f32; 2],
    base: f32,
    at: [f32; 2],
    at_t: f32,
}

impl Walker<'_> {
    pub fn advance(&mut self, t: f32) -> [f32; 2] {
        for _ in 0..MAX_CROSSINGS {
            let reach = t - self.base;
            let target = [self.origin[0] + self.dir[0] * reach, self.origin[1] + self.dir[1] * reach];
            if !(target[0].is_finite() && target[1].is_finite()) {
                break;
            }
            let Some((u, q2, i)) = self.links.first_hit(self.at, target) else {
                self.at = target;
                self.at_t = t;
                return target;
            };
            let Some(l) = self.links.links.get(i) else { break };
            let tc = self.at_t + u * (t - self.at_t);
            let d = [l.m[0] * self.dir[0] + l.m[1] * self.dir[1], l.m[2] * self.dir[0] + l.m[3] * self.dir[1]];
            let norm = dot(d, d).sqrt();
            if norm.is_nan() || norm <= 0.0 {
                break;
            }
            self.dir = [d[0] / norm, d[1] / norm];
            self.origin = q2;
            self.base = tc;
            self.at = q2;
            self.at_t = tc;
        }
        let reach = t - self.base;
        let p = [self.origin[0] + self.dir[0] * reach, self.origin[1] + self.dir[1] * reach];
        self.at = p;
        self.at_t = t;
        p
    }
}

type Cache = Mutex<Option<(u64, Arc<SeamLinks>)>>;

fn mix(h: &mut u64, v: u64) {
    *h = (*h ^ v).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(23);
}

fn fingerprint(mesh: &Mesh, gbuf: &UvGBuffer) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    mix(&mut h, u64::from(gbuf.width) << 32 | u64::from(gbuf.height));
    for p in &mesh.positions {
        mix(&mut h, u64::from(p.x.to_bits()) << 32 | u64::from(p.y.to_bits()));
        mix(&mut h, u64::from(p.z.to_bits()));
    }
    for uv in &mesh.uvs {
        mix(&mut h, u64::from(uv.x.to_bits()) << 32 | u64::from(uv.y.to_bits()));
    }
    for i in &mesh.indices {
        mix(&mut h, u64::from(*i));
    }
    for t in &gbuf.tri {
        mix(&mut h, u64::from(*t));
    }
    h
}

pub fn links_for(mesh: &Mesh, gbuf: &UvGBuffer) -> Option<Arc<SeamLinks>> {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let key = fingerprint(mesh, gbuf);
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock()
        && let Some((k, links)) = guard.as_ref()
        && *k == key
    {
        return (!links.is_empty()).then(|| links.clone());
    }
    let built = Arc::new(SeamLinks::build(mesh, gbuf));
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((key, built.clone()));
    }
    (!built.is_empty()).then_some(built)
}
