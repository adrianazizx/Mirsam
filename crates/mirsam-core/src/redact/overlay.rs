//! The objects an update is about to write, readable as the file would show
//! them after it: one place that every pass (redaction, scrubbing,
//! sanitizing) reads from and writes to, so no object is written twice.

use std::collections::BTreeMap;

use crate::pdf::write::Update;
use crate::pdf::{Dict, Entry, File, Object};

pub(crate) enum Held {
    Object(Object),
    /// A stream whose data is written as given.
    Stream(Dict, Vec<u8>),
}

pub(crate) struct Overlay<'f, 'b> {
    pub(crate) file: &'f File<'b>,
    held: BTreeMap<u32, (u16, Held)>,
}

impl<'f, 'b> Overlay<'f, 'b> {
    pub(crate) fn new(file: &'f File<'b>) -> Self {
        Self { file, held: BTreeMap::new() }
    }

    /// The object as the update will leave it, and its raw stream data when
    /// it is a stream. For a stream held here the data is as it will be
    /// written.
    pub(crate) fn get(&self, number: u32) -> Option<(Object, Option<Vec<u8>>)> {
        if let Some((_, held)) = self.held.get(&number) {
            return Some(match held {
                Held::Object(object) => (object.clone(), None),
                Held::Stream(dict, data) => (Object::Dict(dict.clone()), Some(data.clone())),
            });
        }
        match *self.file.entries.get(&number)? {
            Entry::Offset(offset) => {
                let (object, data) = self.file.indirect_object_at(offset)?;
                Some((object, data.map(<[u8]>::to_vec)))
            }
            Entry::InStream { stream, index } => {
                Some((self.file.in_object_stream(stream, index, number)?, None))
            }
        }
    }

    pub(crate) fn resolve(&self, object: &Object) -> Option<Object> {
        let mut current = object.clone();
        for _ in 0..crate::pdf::MAX_DEPTH {
            let Object::Ref(number, _) = current else { return Some(current) };
            current = self.get(number)?.0;
        }
        None
    }

    pub(crate) fn put(&mut self, number: u32, generation: u16, object: Object) {
        self.held.insert(number, (generation, Held::Object(object)));
    }

    pub(crate) fn put_stream(&mut self, number: u32, generation: u16, dict: Dict, data: Vec<u8>) {
        self.held.insert(number, (generation, Held::Stream(dict, data)));
    }

    pub(crate) fn numbers(&self) -> Vec<u32> {
        let mut numbers: Vec<u32> = self.file.entries.keys().copied().collect();
        numbers.extend(self.held.keys().copied());
        numbers.sort_unstable();
        numbers.dedup();
        numbers
    }

    /// The generation number of object `number` (0 where it cannot be read).
    pub(crate) fn generation(&self, number: u32) -> u16 {
        if let Some((generation, _)) = self.held.get(&number) {
            return *generation;
        }
        let Some(Entry::Offset(offset)) = self.file.entries.get(&number).copied() else { return 0 };
        let mut parser = crate::pdf::Parser::new(self.file.bytes, offset);
        parser.unsigned();
        parser.unsigned().and_then(|g| u16::try_from(g).ok()).unwrap_or(0)
    }

    /// Writes every held object into `update`, in number order.
    pub(crate) fn write(self, update: &mut Update) {
        for (number, (generation, held)) in self.held {
            match held {
                Held::Object(object) => update.object(number, generation, &object),
                Held::Stream(dict, data) => update.stream(number, generation, dict, &data),
            }
        }
    }
}
