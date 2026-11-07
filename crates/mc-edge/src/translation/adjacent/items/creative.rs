//! Creative outer framing and provenance of a lossy client projection.
use super::*;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

#[derive(Default)]
pub struct Images {
    slots: HashMap<i16, Image>,
    carried: Option<Image>,
}

pub enum Update<'a> {
    Echo,
    Restore(std::borrow::Cow<'a, [u8]>),
    Translate,
}

impl Images {
    pub fn clear_slots(&mut self) {
        self.slots.clear();
    }

    pub fn close(&mut self) {
        self.clear_slots();
        self.carried = None;
    }

    pub fn observe(&mut self, slot: i16, image: Option<Image>) {
        if let Some(image) = image {
            self.slots.insert(slot, image);
        } else {
            self.slots.remove(&slot);
        }
    }

    pub fn observe_cursor(&mut self, image: Option<Image>) {
        self.carried = image;
    }

    pub fn update<'a>(&'a mut self, t: &Tables, slot: i16, wire: &[u8]) -> Result<Update<'a>> {
        if slot < 0 {
            if let Some(image) = self.carried.take()
                && let Some(count) = image.matching_count(t, wire)?
            {
                let canonical = if count == image.count {
                    image.canonical
                } else {
                    image.restored_count(count)?.into_owned()
                };
                return Ok(Update::Restore(std::borrow::Cow::Owned(canonical)));
            }
            return Ok(Update::Translate);
        }
        if !(5..=45).contains(&slot) {
            return Ok(Update::Translate);
        }
        if let Some(image) = self.carried.as_ref()
            && let Some(count) = image.matching_count(t, wire)?
        {
            let image = self.carried.take().expect("matching carried image");
            self.carried = self.slots.insert(slot, image);
            return Ok(Update::Restore(self.slots[&slot].restored_count(count)?));
        }
        if let Some(image) = self.slots.get(&slot)
            && let Some(count) = image.matching_count(t, wire)?
        {
            return Ok(if count == image.count {
                Update::Echo
            } else {
                Update::Restore(std::borrow::Cow::Owned(
                    image.restored_count(count)?.into_owned(),
                ))
            });
        }
        let mut count = wire;
        if count.read_varint()? == 0 {
            // An empty creative write moves the old peer image to the client's
            // cursor. Later authoritative empty-slot refreshes must not erase it.
            if let Some(image) = self.slots.remove(&slot) {
                self.carried = Some(image);
            }
        } else {
            self.slots.remove(&slot);
            self.carried = None;
        }
        Ok(Update::Translate)
    }
}

pub struct Image {
    canonical: Vec<u8>,
    peer: Vec<u8>,
    count: i32,
    item: i32,
    added: HashMap<i32, Range<usize>>,
    removed: HashSet<i32>,
}

impl Image {
    pub fn new(t: &Tables, canonical: &[u8], peer: &[u8]) -> Result<Self> {
        let peer = delimit(t, To::Canonical, peer)?;
        let mut r = peer.as_slice();
        let count = r.read_varint()?;
        let item = r.read_varint()?;
        let additions = r.read_varint()?;
        let removals = r.read_varint()?;
        let mut added = HashMap::with_capacity(additions as usize);
        for _ in 0..additions {
            let id = r.read_varint()?;
            let value = component(&mut r)?;
            if !default_pot(t, item, id, value)? {
                let start = peer.len() - r.len() - value.len();
                added.insert(id, start..start + value.len());
            }
        }
        let removed = (0..removals)
            .map(|_| r.read_varint())
            .collect::<std::result::Result<_, _>>()?;
        ensure!(r.is_empty(), "creative image trailing bytes");
        Ok(Self {
            canonical: delimit(t, To::Legacy, canonical)?,
            peer,
            count,
            item,
            added,
            removed,
        })
    }

    fn matching_count(&self, t: &Tables, mut r: &[u8]) -> Result<Option<i32>> {
        let count = r.read_varint()?;
        if count <= 0 || r.read_varint()? != self.item {
            return Ok(None);
        }
        let added = r.read_varint()?;
        let removed = r.read_varint()?;
        ensure!(
            (0..=MAX_LIST).contains(&added) && (0..=MAX_LIST).contains(&removed),
            "component count"
        );
        let mut matched = 0;
        let mut seen = [0; MAX_LIST as usize * 2];
        let mut seen_count = 0;
        for _ in 0..added {
            let id = r.read_varint()?;
            ensure!(
                !seen[..seen_count].contains(&id),
                "duplicate creative component"
            );
            seen[seen_count] = id;
            seen_count += 1;
            let value = component(&mut r)?;
            if default_pot(t, self.item, id, value)? {
                continue;
            }
            if !self
                .added
                .get(&id)
                .is_some_and(|range| &self.peer[range.clone()] == value)
            {
                return Ok(None);
            }
            matched += 1;
        }
        if matched != self.added.len() || removed as usize != self.removed.len() {
            return Ok(None);
        }
        for _ in 0..removed {
            let id = r.read_varint()?;
            ensure!(
                !seen[..seen_count].contains(&id),
                "duplicate creative component"
            );
            seen[seen_count] = id;
            seen_count += 1;
            if !self.removed.contains(&id) {
                return Ok(None);
            }
        }
        ensure!(r.is_empty(), "creative echo trailing bytes");
        Ok(Some(count))
    }

    fn restored_count(&self, count: i32) -> Result<std::borrow::Cow<'_, [u8]>> {
        if count == self.count {
            return Ok(std::borrow::Cow::Borrowed(&self.canonical));
        }
        let mut body = self.canonical.as_slice();
        body.read_varint()?;
        let mut restored = Vec::with_capacity(body.len() + 5);
        restored.write_varint(count);
        restored.extend_from_slice(body);
        Ok(std::borrow::Cow::Owned(restored))
    }
}

fn component<'a>(r: &mut &'a [u8]) -> Result<&'a [u8]> {
    let length = usize::try_from(r.read_varint()?).context("negative component length")?;
    ensure!(length <= r.len(), "truncated creative component");
    let (value, rest) = r.split_at(length);
    *r = rest;
    Ok(value)
}

fn default_pot(t: &Tables, item: i32, id: i32, mut value: &[u8]) -> Result<bool> {
    // The <777 PotDecorations value of four bricks equals the item's default.
    // PatchedDataComponentMap discards it when the real client reserializes.
    if t.formats.lower >= 777
        || t.legacy_pot_item != Some(item)
        || t.component_shapes.shape(To::Canonical, id) != Some(Payload::PotDecorations)
        || value.read_varint()? != 4
    {
        return Ok(false);
    }
    for _ in 0..4 {
        if value.read_varint()? != t.legacy_brick {
            return Ok(false);
        }
    }
    Ok(value.is_empty())
}

/// Reframe a trusted stack without remapping its ids or component payloads.
/// `to` identifies the source endpoint for the existing bounded shape walker.
pub fn delimit(t: &Tables, to: To, mut r: &[u8]) -> Result<Vec<u8>> {
    let mut w = Vec::new();
    let count = r.read_varint()?;
    w.write_varint(count);
    if count > 0 {
        w.write_varint(r.read_varint()?);
        let added = r.read_varint()?;
        let removed = r.read_varint()?;
        ensure!(
            (0..=MAX_LIST).contains(&added) && (0..=MAX_LIST).contains(&removed),
            "component count"
        );
        w.write_varint(added);
        w.write_varint(removed);
        let mut scratch = Vec::new();
        for _ in 0..added {
            let id = r.read_varint()?;
            let shape = t
                .component_shapes
                .shape(to, id)
                .context("unsupported creative image component")?;
            let before = r;
            scratch.clear();
            copy_payload(t, to, shape, &mut r, &mut scratch, 0, &mut false)?;
            let value = &before[..before.len() - r.len()];
            w.write_varint(id);
            w.write_varint(value.len() as i32);
            w.extend_from_slice(value);
        }
        for _ in 0..removed {
            w.write_varint(r.read_varint()?);
        }
    }
    ensure!(r.is_empty(), "creative image trailing bytes");
    Ok(w)
}
