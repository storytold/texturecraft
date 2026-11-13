//! Alembic objects and properties in the HDF5 container, as Alembic's documentation and files
//! lay them out:
//!
//! - the root group holds the top object as the group `ABC` (and version attributes);
//! - an object is a group: its child objects are its other links, in creation order; its own
//!   metadata is the string attribute `.prop.meta` on its group; its properties are in the
//!   compound property group `.prop`;
//! - a compound property group lists its properties as attributes `<name>.info` (one to five
//!   u32: the first packs the property type in bits 0-1, the POD type in bits 2-5 and the extent
//!   in bits 8-15; a lone 0 marks a compound), with metadata in the string attribute
//!   `<name>.meta`; a child compound property is the group `<name>`;
//! - the first sample of an array property is the dataset `<name>.smp0` (items and extent in one
//!   dimension), of a scalar property the attribute `<name>.smp0`. Later samples (in the group
//!   `<name>.smpi`) are not read: like the Ogawa reader, this reads the first sample.

use std::collections::HashMap;
use std::rc::Rc;

use super::hdf5::{Attr, H5, Link, numbers, text};
use super::{Archive, Prop, PropKind};
use crate::{Result, malformed};

const KIND: &str = "Alembic (HDF5)";
/// Groups whose links and attributes are kept for reuse (objects and compounds shared many
/// times over are parsed once).
const MAX_CACHED: usize = 65_536;

/// Where an object or property lives: the object's group (`name` empty), or the compound
/// property group holding property `name`.
#[derive(Clone)]
pub(super) struct Loc {
    group: u64,
    name: String,
}

pub(super) struct H5Archive<'a> {
    h: H5<'a>,
    links: HashMap<u64, Rc<Vec<Link>>>,
    attrs: HashMap<u64, Rc<Vec<Attr<'a>>>>,
    is_group: HashMap<u64, bool>,
}

impl<'a> H5Archive<'a> {
    pub(super) fn open(bytes: &'a [u8]) -> Result<H5Archive<'a>> {
        Ok(H5Archive { h: H5::open(bytes)?, links: HashMap::new(), attrs: HashMap::new(), is_group: HashMap::new() })
    }

    /// The top object: the root group's `ABC` group.
    pub(super) fn top(&mut self) -> Result<Loc> {
        let root = self.h.root;
        let top = self.link(root, "ABC")?.ok_or_else(|| malformed(KIND, "the HDF5 file holds no Alembic archive (no ABC group)"))?;
        Ok(Loc { group: top, name: String::new() })
    }

    fn links(&mut self, group: u64) -> Result<Rc<Vec<Link>>> {
        if let Some(l) = self.links.get(&group) {
            return Ok(l.clone());
        }
        let l = Rc::new(self.h.links(group)?);
        if self.links.len() >= MAX_CACHED {
            self.links.clear();
        }
        self.links.insert(group, l.clone());
        Ok(l)
    }

    fn attrs(&mut self, group: u64) -> Result<Rc<Vec<Attr<'a>>>> {
        if let Some(a) = self.attrs.get(&group) {
            return Ok(a.clone());
        }
        let a = Rc::new(self.h.attributes(group)?);
        if self.attrs.len() >= MAX_CACHED {
            self.attrs.clear();
        }
        self.attrs.insert(group, a.clone());
        Ok(a)
    }

    /// Address of `group`'s link `name`.
    fn link(&mut self, group: u64, name: &str) -> Result<Option<u64>> {
        Ok(self.links(group)?.iter().find(|l| l.name == name).map(|l| l.addr))
    }

    /// Whether the object at `addr` is a group (datasets are not child objects).
    fn is_group(&mut self, addr: u64) -> Result<bool> {
        if let Some(g) = self.is_group.get(&addr) {
            return Ok(*g);
        }
        let g = !self.h.is_dataset(addr)?;
        if self.is_group.len() >= MAX_CACHED {
            self.is_group.clear();
        }
        self.is_group.insert(addr, g);
        Ok(g)
    }

    /// The properties of the compound property group at `group`.
    fn props_in(&mut self, group: u64) -> Result<Vec<Prop<Loc>>> {
        let attrs = self.attrs(group)?;
        let mut out = Vec::new();
        for a in attrs.iter() {
            let Some(name) = a.name.strip_suffix(".info").filter(|n| !n.is_empty()) else { continue };
            let info = numbers(a.dtype, a.data)?;
            let Some(&word) = info.first() else { continue };
            let word = word as u32;
            let meta_name = format!("{name}.meta");
            let meta = attrs.iter().find(|m| m.name == meta_name).map(text).unwrap_or_default();
            let kind = match word & 0x03 {
                0 => PropKind::Compound,
                1 => PropKind::Scalar,
                // 3: an array property flagged scalar-like.
                _ => PropKind::Array,
            };
            let (pod, extent) = ((word >> 2) & 0x0f, ((word >> 8) & 0xff) as usize);
            out.push(Prop { name: name.to_string(), meta, kind, pod, extent, at: Loc { group, name: name.to_string() } });
        }
        Ok(out)
    }
}

impl Archive for H5Archive<'_> {
    type Handle = Loc;

    fn children(&mut self, object: &Loc) -> Result<Vec<(String, String, Loc)>> {
        let links = self.links(object.group)?;
        let mut out = Vec::new();
        for l in links.iter().filter(|l| l.name != ".prop") {
            if !self.is_group(l.addr)? {
                continue;
            }
            let attrs = self.attrs(l.addr)?;
            let meta = attrs.iter().find(|a| a.name == ".prop.meta").map(text).unwrap_or_default();
            out.push((l.name.clone(), meta, Loc { group: l.addr, name: String::new() }));
        }
        Ok(out)
    }

    fn properties(&mut self, object: &Loc) -> Result<Vec<Prop<Loc>>> {
        match self.link(object.group, ".prop")? {
            Some(g) => self.props_in(g),
            None => Ok(Vec::new()),
        }
    }

    fn compound(&mut self, property: &Prop<Loc>) -> Result<Vec<Prop<Loc>>> {
        match self.link(property.at.group, &property.at.name)? {
            Some(g) => self.props_in(g),
            None => Ok(Vec::new()),
        }
    }

    fn values(&mut self, property: &Prop<Loc>) -> Result<Vec<f64>> {
        let sample = format!("{}.smp0", property.at.name);
        let values = match property.kind {
            PropKind::Compound => return Ok(Vec::new()),
            PropKind::Scalar => {
                let attrs = self.attrs(property.at.group)?;
                match attrs.iter().find(|a| a.name == sample) {
                    Some(a) => numbers(a.dtype, a.data),
                    None => return Ok(Vec::new()),
                }
            }
            PropKind::Array => match self.link(property.at.group, &sample)? {
                Some(addr) => {
                    let d = self.h.dataset(addr)?;
                    numbers(d.dtype, &d.data)
                }
                None => return Ok(Vec::new()),
            },
        };
        let values = values.map_err(|e| malformed(KIND, format!("property \"{}\": {e}", property.name)))?;
        self.h.charge(values.len().saturating_mul(8))?;
        Ok(values)
    }
}
