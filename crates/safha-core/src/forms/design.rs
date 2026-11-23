//! Making forms (ADR 0047): fields added, changed, moved, and removed, and
//! the order Tab visits them in, each as an incremental update.
//!
//! A new field is a standard AcroForm field (ISO 32000-2, 12.7): one object
//! that is both the field and its widget, or, for a radio group, a parent
//! field whose kids are its buttons. Its appearance is drawn at once by the
//! same code that fills fields (ADR 0045), so an Arabic default value is
//! shaped, and every viewer shows what we drew.
//!
//! Its `/DA` names one of our two fonts (`/MirsamSans`, `/MirsamNaskh`).
//! That says which bundled family draws it here, and that its `/Q` is
//! logical: 2 is the end of the text's own direction (the left of Arabic).
//! `/DR` maps both names to standard fonts, for viewers that draw a field
//! themselves while it is typed into.
//!
//! Tab order is the order of a page's widgets in its `/Annots`, with
//! `/Tabs /W` (ISO 32000-2, 12.5.1) saying so.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use super::appearance::{ButtonStyle, Look, NASKH_FONT, SANS_FONT, da_font, parse_da};
use super::{
    COMBO, Changed, EDIT, FieldAlign, FieldKind, FieldOption, Form, FormField, Looks, MAX_FIELD_TEXT,
    MULTI_SELECT, MULTILINE, NO_TOGGLE_TO_OFF, Node, PUSH_BUTTON, RADIO, READ_ONLY, REQUIRED, Reader,
    refresh, value_appearance,
};
use crate::annotate::{Annots, MAX_COORDINATE, Writer, num, rect_of, resolve_dict, text_string};
use crate::edit::{Leaf, PageTree, Start, set};
use crate::error::{Error, Result};
use crate::fonts::FontFamily;
use crate::pdf::crypt::Crypt;
use crate::pdf::{Dict, File, Object};

/// The longest field name, in UTF-16 code units.
pub const MAX_FIELD_NAME: usize = 256;
/// The most choices a drop-down or list is given here.
const MAX_OPTIONS: usize = 1_000;
/// The smallest a field is made, in points.
const MIN_SIDE: f32 = 4.0;
/// A new field's border (`/MK /BC`): a mid grey.
const BORDER: f32 = 0.4;
/// A push button's face (`/MK /BG`).
const BUTTON_FACE: f32 = 0.85;

/// A field as the designer sets it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldSpec {
    pub kind: FieldKind,
    /// Its own part of the name (`/T`): no full stops, which separate parts.
    pub name: String,
    #[serde(default)]
    pub tooltip: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub read_only: bool,
    /// Text fields.
    #[serde(default)]
    pub multiline: bool,
    #[serde(default)]
    pub max_len: Option<usize>,
    /// The value it starts with and resets to, as [`FormField::values`]:
    /// a text; a choice's value; a list's values; `["Yes"]` for a ticked
    /// check box; a radio button's value.
    #[serde(default)]
    pub default_values: Vec<String>,
    #[serde(default)]
    pub align: FieldAlign,
    /// The bundled family; none keeps a field's own `/DA` font.
    #[serde(default)]
    pub font: Option<FontFamily>,
    /// 0 fits the text to the box.
    #[serde(default)]
    pub font_size: f32,
    /// A drop-down's or list's choices; a radio group's buttons, in widget
    /// order (their values).
    #[serde(default)]
    pub options: Vec<FieldOption>,
    #[serde(default)]
    pub multi_select: bool,
    #[serde(default)]
    pub editable: bool,
    #[serde(default = "yes")]
    pub bordered: bool,
    /// A push button's label.
    #[serde(default)]
    pub caption: String,
}

fn yes() -> bool {
    true
}

/// A radio button's export value: its label, else its state's name.
fn export_of(option: &FieldOption) -> &str {
    if option.label.is_empty() { &option.value } else { &option.label }
}

/// The state name of a radio button at `index` (ISO 32000-2, 12.7.5.2.4:
/// its position, with `/Opt` holding its export value). ASCII, so every
/// reader keeps it as it is.
fn position_name(index: usize) -> Vec<u8> {
    index.to_string().into_bytes()
}

fn opt_of<'a>(exports: impl Iterator<Item = &'a str>) -> Object {
    Object::Array(exports.map(|export| Object::Str(text_string(export))).collect())
}

/// One change to a document's form (ADR 0047). Boxes are in page space
/// (left, bottom, right, top).
#[derive(Debug, Clone, PartialEq)]
pub enum FieldDesign {
    /// A new field with one widget. A radio group starts with one button,
    /// whose value is its first option's.
    Add { page: usize, rect: [f32; 4], spec: FieldSpec },
    /// Another button for radio group `field`, its value `value`.
    AddButton { field: u32, page: usize, rect: [f32; 4], value: String },
    /// New properties for `field`; its kind stays.
    Update { field: u32, spec: FieldSpec },
    /// Moves or resizes widget `widget` of `field`.
    SetRect { field: u32, widget: usize, rect: [f32; 4] },
    /// Removes a radio group's button `widget` (a group keeps one at least).
    RemoveButton { field: u32, widget: usize },
    /// Removes the field and its widgets.
    Remove { field: u32 },
    /// The order Tab visits `page`'s fields in: these first, in this order,
    /// then the page's other fields as they were.
    TabOrder { page: usize, fields: Vec<u32> },
}

impl FieldSpec {
    fn check(&self) -> Result<()> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(Error::InvalidEdit("a field needs a name"));
        }
        if name.contains('.') {
            return Err(Error::InvalidEdit("a field's name has no full stops"));
        }
        if name.encode_utf16().count() > MAX_FIELD_NAME
            || self.caption.encode_utf16().count() > MAX_FIELD_NAME
        {
            return Err(Error::InvalidEdit("a field's name holds at most 256 characters"));
        }
        if self.tooltip.encode_utf16().count() > MAX_FIELD_TEXT {
            return Err(Error::InvalidEdit("a field holds at most 20,000 characters"));
        }
        if self.kind == FieldKind::Signature {
            return Err(Error::InvalidEdit("signature fields are made by signing"));
        }
        if !self.font_size.is_finite() || !(self.font_size == 0.0 || (4.0..=144.0).contains(&self.font_size))
        {
            return Err(Error::InvalidEdit("a field's text is 4 to 144 points, or fitted"));
        }
        if self.max_len.is_some_and(|n| n == 0 || n > MAX_FIELD_TEXT) {
            return Err(Error::InvalidEdit("a field's longest text is 1 to 20,000 characters"));
        }
        let choices = matches!(self.kind, FieldKind::Combo | FieldKind::List | FieldKind::Radio);
        if choices {
            if self.options.len() > MAX_OPTIONS {
                return Err(Error::InvalidEdit("a field has at most 1,000 choices"));
            }
            let mut seen = HashSet::new();
            for option in &self.options {
                // A radio button is known by its export value (its label), its
                // state name being its position (ADR 0047).
                let own =
                    if self.kind == FieldKind::Radio { export_of(option) } else { option.value.as_str() };
                if own.is_empty() || !seen.insert(own) {
                    return Err(Error::InvalidEdit("each choice needs a value of its own"));
                }
                if option.value.encode_utf16().count() > MAX_FIELD_NAME
                    || option.label.encode_utf16().count() > MAX_FIELD_NAME
                {
                    return Err(Error::InvalidEdit("a choice holds at most 256 characters"));
                }
            }
            if self.kind == FieldKind::Radio && self.options.is_empty() {
                return Err(Error::InvalidEdit("a radio group needs a button"));
            }
        }
        let known = |value: &String| self.options.iter().any(|option| &option.value == value);
        let values = &self.default_values;
        match self.kind {
            FieldKind::Text => {
                if values.len() > 1 {
                    return Err(Error::InvalidEdit("a text field has one value"));
                }
                let text = values.first().map(String::as_str).unwrap_or("");
                if text.encode_utf16().count() > MAX_FIELD_TEXT {
                    return Err(Error::InvalidEdit("a field holds at most 20,000 characters"));
                }
                if self.max_len.is_some_and(|max| text.chars().count() > max) {
                    return Err(Error::InvalidEdit("the text is longer than the field allows"));
                }
            }
            FieldKind::Combo => {
                if values.len() > 1
                    || values.first().is_some_and(|v| !v.is_empty() && !known(v) && !self.editable)
                {
                    return Err(Error::InvalidEdit("not one of the field's choices"));
                }
            }
            FieldKind::List => {
                if values.len() > 1 && !self.multi_select {
                    return Err(Error::InvalidEdit("this list takes one choice"));
                }
                if !values.iter().all(known) {
                    return Err(Error::InvalidEdit("not one of the field's choices"));
                }
            }
            FieldKind::Checkbox => {
                if values.len() > 1 || values.first().is_some_and(|v| v != "Yes") {
                    return Err(Error::InvalidEdit("a check box is ticked (Yes) or not"));
                }
            }
            FieldKind::Radio => {
                if values.len() > 1 || !values.iter().all(|v| self.radio_index(v).is_some()) {
                    return Err(Error::InvalidEdit("not one of the field's choices"));
                }
            }
            FieldKind::PushButton | FieldKind::Signature => {
                if !values.is_empty() {
                    return Err(Error::InvalidEdit("a push button has no value"));
                }
            }
        }
        Ok(())
    }

    /// The button a radio group's value names: by its state, or else by its
    /// export value.
    fn radio_index(&self, value: &str) -> Option<usize> {
        self.options
            .iter()
            .position(|option| option.value == value)
            .or_else(|| self.options.iter().position(|option| export_of(option) == value))
    }

    /// The field flags (`/Ff`) the designer sets.
    fn flags(&self) -> i64 {
        let mut flags = 0;
        if self.read_only {
            flags |= READ_ONLY;
        }
        if self.required {
            flags |= REQUIRED;
        }
        match self.kind {
            FieldKind::Text if self.multiline => flags |= MULTILINE,
            FieldKind::Combo => flags |= COMBO | if self.editable { EDIT } else { 0 },
            FieldKind::List if self.multi_select => flags |= MULTI_SELECT,
            FieldKind::Radio => flags |= RADIO | NO_TOGGLE_TO_OFF,
            FieldKind::PushButton => flags |= PUSH_BUTTON,
            _ => {}
        }
        flags
    }

    /// The `/DA` of a field set in `font` at `size`, in `color`.
    fn da(&self, color: [f32; 3]) -> Vec<u8> {
        let font = match self.kind {
            FieldKind::Checkbox | FieldKind::Radio => b"ZaDb".as_slice(),
            _ => match self.font.unwrap_or_default() {
                FontFamily::Sans => SANS_FONT,
                FontFamily::Naskh => NASKH_FONT,
            },
        };
        let [r, g, b] = color;
        let colour = if r == g && g == b {
            format!("{} g", num(r))
        } else {
            format!("{} {} {} rg", num(r), num(g), num(b))
        };
        format!("/{} {} Tf {colour}", String::from_utf8_lossy(font), num(self.font_size)).into_bytes()
    }

    /// `/MK` for a widget turned `rotation` degrees.
    fn mk(&self, rotation: i64) -> Dict {
        let mut mk = Dict::default();
        if rotation != 0 {
            mk.0.push((b"R".to_vec(), Object::Int(rotation)));
        }
        if self.bordered {
            mk.0.push((b"BC".to_vec(), Object::Array(vec![Object::Real(f64::from(BORDER))])));
        }
        match self.kind {
            FieldKind::PushButton => {
                mk.0.push((b"BG".to_vec(), Object::Array(vec![Object::Real(f64::from(BUTTON_FACE))])));
                mk.0.push((b"CA".to_vec(), Object::Str(text_string(&self.caption))));
            }
            // ZapfDingbats letters: a tick, and a dot.
            FieldKind::Checkbox => mk.0.push((b"CA".to_vec(), Object::Str(b"4".to_vec()))),
            FieldKind::Radio => mk.0.push((b"CA".to_vec(), Object::Str(b"l".to_vec()))),
            _ => {}
        }
        mk
    }

    /// `/Opt` for a drop-down or list: `[value label]` pairs, or the value
    /// alone where it is its own label.
    fn opt(&self) -> Object {
        Object::Array(
            self.options
                .iter()
                .map(|option| {
                    if option.label.is_empty() || option.label == option.value {
                        Object::Str(text_string(&option.value))
                    } else {
                        Object::Array(vec![
                            Object::Str(text_string(&option.value)),
                            Object::Str(text_string(&option.label)),
                        ])
                    }
                })
                .collect(),
        )
    }

    /// The options with labels filled in (a label left empty is its value).
    fn shown_options(&self) -> Vec<FieldOption> {
        self.options
            .iter()
            .map(|option| FieldOption {
                value: option.value.clone(),
                label: if option.label.is_empty() { option.value.clone() } else { option.label.clone() },
            })
            .collect()
    }

    /// The field as [`super::value_appearance`] draws it.
    fn as_field(&self, values: Vec<String>) -> FormField {
        FormField {
            id: 0,
            name: self.name.clone(),
            partial_name: self.name.clone(),
            kind: self.kind,
            tooltip: self.tooltip.clone(),
            default_values: self.default_values.clone(),
            align: self.align,
            font: self.font,
            font_size: self.font_size,
            bordered: self.bordered,
            caption: self.caption.clone(),
            values,
            options: self.shown_options(),
            read_only: self.read_only,
            required: self.required,
            multiline: self.kind == FieldKind::Text && self.multiline,
            password: false,
            comb: false,
            max_len: self.max_len,
            multi_select: self.multi_select,
            editable: self.editable,
            widgets: Vec::new(),
        }
    }

    /// `/V` and `/DV` for these values (none: no value).
    fn value_object(&self, values: &[String]) -> Option<Object> {
        match self.kind {
            FieldKind::Checkbox | FieldKind::Radio => {
                Some(Object::Name(values.first().map_or(b"Off".to_vec(), |v| v.as_bytes().to_vec())))
            }
            FieldKind::List if values.len() > 1 => {
                Some(Object::Array(values.iter().map(|v| Object::Str(text_string(v))).collect()))
            }
            FieldKind::PushButton | FieldKind::Signature => None,
            _ => values.first().filter(|v| !v.is_empty()).map(|v| Object::Str(text_string(v))),
        }
    }
}

/// A widget box: finite, at least a few points each way, normalized.
fn check_rect([x0, y0, x1, y1]: [f32; 4]) -> Result<[f32; 4]> {
    let rect = [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)];
    if rect.iter().any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE)
        || rect[2] - rect[0] < MIN_SIDE
        || rect[3] - rect[1] < MIN_SIDE
    {
        return Err(Error::InvalidEdit("a field needs a box on the page"));
    }
    Ok(rect)
}

fn reals(values: &[f32]) -> Object {
    Object::Array(values.iter().map(|v| Object::Real(f64::from(*v))).collect())
}

/// Everything an update changes in objects that exist already, each
/// written once at its end.
struct Edits<'r, 'f> {
    reader: &'r Reader<'r, 'f>,
    dicts: Changed,
    arrays: BTreeMap<(u32, u16), Vec<Object>>,
}

impl<'r, 'f> Edits<'r, 'f> {
    fn file(&self) -> &'r File<'f> {
        self.reader.file
    }

    /// The page's `/Annots`, to change: in its own array object or in the
    /// page dictionary.
    fn annots(&mut self, leaf: &Leaf) -> Result<&mut Vec<Object>> {
        let annots = Annots::read(self.file(), leaf)?;
        if let Some(own) = annots.own {
            return Ok(self.arrays.entry(own).or_insert(annots.items));
        }
        let page = page_node(leaf);
        let dict = self.dicts.entry(&page)?;
        if !matches!(dict.get(b"Annots"), Some(Object::Array(_))) {
            set(dict, b"Annots", Object::Array(annots.items));
        }
        match dict.0.iter_mut().find(|(key, _)| key == b"Annots") {
            Some((_, Object::Array(items))) => Ok(items),
            _ => Err(Error::EditUnsupported("the page's annotations cannot be read")),
        }
    }

    /// The AcroForm dictionary, to change.
    fn acro(&mut self, form: &Form) -> Result<&mut Dict> {
        match &form.catalog {
            Some(catalog) => {
                let dict = self.dicts.entry(catalog)?;
                match dict.0.iter_mut().find(|(key, _)| key == b"AcroForm") {
                    Some((_, Object::Dict(acro))) => Ok(acro),
                    _ => Err(Error::EditUnsupported("the form cannot be read")),
                }
            }
            None => self.dicts.entry(&form.acro),
        }
    }

    /// An array that is an entry of a dictionary, or an object of its own,
    /// to change: `/Fields`, `/Kids`, `/CO`.
    fn array_in(&mut self, holder: ArrayHolder<'_>, key: &[u8]) -> Result<Option<&mut Vec<Object>>> {
        let current = match &holder {
            ArrayHolder::Acro(form) => self.acro_view(form).get(key).cloned(),
            ArrayHolder::Node(node) => {
                self.dicts.0.get(&node.id.unwrap_or_default()).unwrap_or(&node.dict).get(key).cloned()
            }
        };
        match current {
            None => Ok(None),
            Some(Object::Ref(number, generation)) => {
                let items = match self.file().resolve(&Object::Ref(number, generation)) {
                    Some(Object::Array(items)) => items,
                    _ => Vec::new(),
                };
                Ok(Some(self.arrays.entry((number, generation)).or_insert(items)))
            }
            Some(Object::Array(_)) => {
                let dict = match holder {
                    ArrayHolder::Acro(form) => self.acro(form)?,
                    ArrayHolder::Node(node) => self.dicts.entry(node)?,
                };
                match dict.0.iter_mut().find(|(k, _)| k == key) {
                    Some((_, Object::Array(items))) => Ok(Some(items)),
                    _ => Ok(None),
                }
            }
            Some(_) => Ok(None),
        }
    }

    /// The AcroForm dictionary as it stands in this update.
    fn acro_view(&self, form: &Form) -> Dict {
        match &form.catalog {
            Some(catalog) => {
                let dict = catalog.id.and_then(|id| self.dicts.0.get(&id)).unwrap_or(&catalog.dict);
                dict.get(b"AcroForm").and_then(Object::as_dict).cloned().unwrap_or_default()
            }
            None => form.acro.id.and_then(|id| self.dicts.0.get(&id)).unwrap_or(&form.acro.dict).clone(),
        }
    }

    fn write(self, writer: &mut Writer<'_, '_>) {
        for ((number, generation), dict) in self.dicts.0 {
            writer.update.object(number, generation, &Object::Dict(dict));
        }
        for ((number, generation), items) in self.arrays {
            writer.update.object(number, generation, &Object::Array(items));
        }
    }
}

enum ArrayHolder<'a> {
    Acro(&'a Form),
    Node(&'a Node),
}

fn page_node(leaf: &Leaf) -> Node {
    let id = Some((leaf.number, leaf.generation));
    Node { id, owner: id, dict: leaf.dict.clone() }
}

/// The bytes to append to `bytes` to apply `edit` (ADR 0047): an
/// incremental update. `password` is the one the document was opened with.
pub fn design_update(bytes: &[u8], password: Option<&str>, edit: &FieldDesign) -> Result<Vec<u8>> {
    let Start { file, section, form: xref_form, mut next_number } = Start::read(bytes)?;
    let crypt = Crypt::of(&file, password)?;
    let tree = PageTree::read(&file)?;
    let reader = Reader { file: &file, crypt: crypt.as_ref() };
    let form = reader.form(&tree);
    let mut writer = Writer::new(&file, bytes, password, &mut next_number);
    let mut edits = Edits { reader: &reader, dicts: Changed::default(), arrays: BTreeMap::new() };
    let leaf = |page: usize| {
        tree.leaves.get(page).ok_or(Error::PageOutOfRange { index: page, count: tree.leaves.len() })
    };
    let field_of = |id: u32| -> Result<&super::Field> {
        form.as_ref()
            .and_then(|form| form.fields.iter().find(|field| field.info.id == id))
            .ok_or(Error::InvalidEdit("no such form field"))
    };

    // Fields already drawn by this edit, left alone by the NeedAppearances pass.
    let mut drawn = HashSet::new();
    match edit {
        FieldDesign::Add { page, rect, spec } => {
            spec.check()?;
            let rect = check_rect(*rect)?;
            let leaf = leaf(*page)?;
            let name = spec.name.trim();
            if form.as_ref().is_some_and(|form| taken(form, name, None)) {
                return Err(Error::InvalidEdit("a field with this name exists already"));
            }
            let spec = FieldSpec { name: name.to_string(), ..spec.clone() };
            let field = add_field(&mut writer, &mut edits, leaf, rect, &spec)?;
            match &form {
                Some(form) => {
                    ensure_fonts(&mut writer, &mut edits, form)?;
                    match edits.array_in(ArrayHolder::Acro(form), b"Fields")? {
                        Some(fields) => fields.push(Object::Ref(field, 0)),
                        None => {
                            set(edits.acro(form)?, b"Fields", Object::Array(vec![Object::Ref(field, 0)]));
                        }
                    }
                }
                None => new_acro_form(&mut writer, &mut edits, &file, field)?,
            }
        }
        FieldDesign::AddButton { field, page, rect, value } => {
            let target = field_of(*field)?;
            if target.info.kind != FieldKind::Radio {
                return Err(Error::InvalidEdit("buttons are added to radio groups"));
            }
            let mut exports = radio_exports(&target.info, target)?;
            if value.trim().is_empty() || exports.iter().any(|export| export == value) {
                return Err(Error::InvalidEdit("each choice needs a value of its own"));
            }
            let rect = check_rect(*rect)?;
            let leaf = leaf(*page)?;
            let parent = &target.chain[0];
            let parent_id =
                parent.id.ok_or(Error::EditUnsupported("this form field is not an object of its own"))?;
            if parent.dict.get(b"Kids").is_none() {
                // A field that is its own widget cannot take another here.
                return Err(Error::EditUnsupported("this radio group cannot take another button"));
            }
            let spec = spec_of(&target.info);
            let state = position_name(target.widgets.len());
            let widget =
                add_radio_button(&mut writer, &mut edits, leaf, rect, &spec, &state, parent_id, false)?;
            if let Some(kids) = edits.array_in(ArrayHolder::Node(parent), b"Kids")? {
                kids.push(Object::Ref(widget, 0));
            }
            exports.push(value.clone());
            let order: Vec<usize> = (0..target.widgets.len()).collect();
            renumber(&mut writer, &mut edits, target, &order, &exports)?;
            drawn.insert(*field);
        }
        FieldDesign::Update { field, spec } => {
            spec.check()?;
            let form = form.as_ref().ok_or(Error::InvalidEdit("this document has no form"))?;
            let target = field_of(*field)?;
            update_field(&mut writer, &mut edits, form, target, spec)?;
            drawn.insert(*field);
        }
        FieldDesign::SetRect { field, widget, rect } => {
            let form = form.as_ref().ok_or(Error::InvalidEdit("this document has no form"))?;
            let target = field_of(*field)?;
            let rect = check_rect(*rect)?;
            let node = &target.widgets.get(*widget).ok_or(Error::InvalidEdit("no such widget"))?.node;
            set(edits.dicts.entry(node)?, b"Rect", reals(&rect));
            redraw(&mut writer, &mut edits, form, target, &target.info, Some(*widget), true)?;
            drawn.insert(*field);
        }
        FieldDesign::RemoveButton { field, widget } => {
            let target = field_of(*field)?;
            if target.info.kind != FieldKind::Radio || target.widgets.len() < 2 {
                return Err(Error::InvalidEdit("a radio group keeps one button at least"));
            }
            let doomed = target.widgets.get(*widget).ok_or(Error::InvalidEdit("no such widget"))?;
            let reference = doomed.node.id.map(|(n, g)| Object::Ref(n, g));
            let Some(reference) = reference else {
                return Err(Error::EditUnsupported("this button is not an object of its own"));
            };
            let mut exports = radio_exports(&target.info, target)?;
            if let Some(kids) = edits.array_in(ArrayHolder::Node(&target.chain[0]), b"Kids")? {
                kids.retain(|kid| kid != &reference);
            }
            remove_from_pages(&mut edits, &tree, &[reference])?;
            exports.remove(*widget);
            let order: Vec<usize> = (0..target.widgets.len()).filter(|i| i != widget).collect();
            renumber(&mut writer, &mut edits, target, &order, &exports)?;
            drawn.insert(*field);
        }
        FieldDesign::Remove { field } => {
            let form = form.as_ref().ok_or(Error::InvalidEdit("this document has no form"))?;
            let target = field_of(*field)?;
            remove_field(&mut edits, &tree, form, target)?;
            drawn.insert(*field);
        }
        FieldDesign::TabOrder { page, fields } => {
            let form = form.as_ref().ok_or(Error::InvalidEdit("this document has no form"))?;
            let leaf = leaf(*page)?;
            tab_order(&mut edits, form, leaf, fields)?;
        }
    }

    // A form that asks viewers to draw its fields gets ours instead (ADR 0045).
    if let Some(form) = form.as_ref().filter(|form| form.need_appearances) {
        let looks = Looks { reader: &reader, form };
        let mut changed = std::mem::take(&mut edits.dicts);
        for field in form.fields.iter().filter(|field| !drawn.contains(&field.info.id)) {
            refresh(&looks, field, &mut writer, &mut changed)?;
        }
        edits.dicts = changed;
        set(edits.acro(form)?, b"NeedAppearances", Object::Bool(false));
    }
    edits.write(&mut writer);
    let update = writer.update;
    Ok(update.finish(xref_form, file.trailer.clone(), section, next_number))
}

/// Whether `full` (a fully qualified name) is taken by a field other than
/// `except`, or is a parent of one.
fn taken(form: &Form, full: &str, except: Option<u32>) -> bool {
    let prefix = format!("{full}.");
    form.fields
        .iter()
        .filter(|field| Some(field.info.id) != except)
        .any(|field| field.info.name == full || field.info.name.starts_with(&prefix))
}

/// The turn a widget takes on `leaf` to stand upright as the page is shown
/// (`/MK /R`, ISO 32000-2, 12.5.6.19).
fn upright(leaf: &Leaf) -> i64 {
    leaf.rotate.rem_euclid(360) / 90 * 90
}

/// Writes a new field (with its widget, or a radio group with its first
/// button) and adds its widget to the page; returns the field's number.
fn add_field(
    writer: &mut Writer<'_, '_>,
    edits: &mut Edits<'_, '_>,
    leaf: &Leaf,
    rect: [f32; 4],
    spec: &FieldSpec,
) -> Result<u32> {
    let values: Vec<String> = spec.default_values.iter().filter(|v| !v.is_empty()).cloned().collect();
    if spec.kind == FieldKind::Radio {
        let parent = writer.take_number()?;
        let chosen = spec.default_values.first().and_then(|v| spec.radio_index(v)) == Some(0);
        let widget =
            add_radio_button(writer, edits, leaf, rect, spec, &position_name(0), (parent, 0), chosen)?;
        let mut dict = vec![
            (b"FT".to_vec(), Object::Name(b"Btn".to_vec())),
            (b"Ff".to_vec(), Object::Int(spec.flags())),
            (b"T".to_vec(), Object::Str(text_string(&spec.name))),
        ];
        if !spec.tooltip.is_empty() {
            dict.push((b"TU".to_vec(), Object::Str(text_string(&spec.tooltip))));
        }
        dict.push((b"DA".to_vec(), Object::Str(spec.da([0.0; 3]))));
        dict.push((b"Opt".to_vec(), opt_of(spec.options.iter().take(1).map(export_of))));
        if chosen {
            dict.push((b"V".to_vec(), Object::Name(position_name(0))));
            dict.push((b"DV".to_vec(), Object::Name(position_name(0))));
        }
        dict.push((b"Kids".to_vec(), Object::Array(vec![Object::Ref(widget, 0)])));
        write_new(writer, parent, Dict(dict))?;
        return Ok(parent);
    }

    let number = writer.take_number()?;
    let rotation = upright(leaf);
    let mk = spec.mk(rotation);
    let bs = border_style();
    let da = spec.da([0.0; 3]);
    let ft: &[u8] = match spec.kind {
        FieldKind::Text => b"Tx",
        FieldKind::Combo | FieldKind::List => b"Ch",
        _ => b"Btn",
    };
    let mut dict = vec![
        (b"Type".to_vec(), Object::Name(b"Annot".to_vec())),
        (b"Subtype".to_vec(), Object::Name(b"Widget".to_vec())),
        (b"Rect".to_vec(), reals(&rect)),
        (b"F".to_vec(), Object::Int(4)),
        (b"P".to_vec(), Object::Ref(leaf.number, leaf.generation)),
        (b"FT".to_vec(), Object::Name(ft.to_vec())),
        (b"T".to_vec(), Object::Str(text_string(&spec.name))),
    ];
    if !spec.tooltip.is_empty() {
        dict.push((b"TU".to_vec(), Object::Str(text_string(&spec.tooltip))));
    }
    dict.push((b"Ff".to_vec(), Object::Int(spec.flags())));
    dict.push((b"DA".to_vec(), Object::Str(da.clone())));
    if spec.kind != FieldKind::Checkbox && spec.kind != FieldKind::PushButton {
        dict.push((b"Q".to_vec(), Object::Int(spec.align.quadding())));
    }
    if spec.kind == FieldKind::Text
        && let Some(max) = spec.max_len
    {
        dict.push((b"MaxLen".to_vec(), Object::Int(i64::try_from(max).unwrap_or(i64::MAX))));
    }
    if matches!(spec.kind, FieldKind::Combo | FieldKind::List) {
        dict.push((b"Opt".to_vec(), spec.opt()));
    }
    if let Some(value) = spec.value_object(&values) {
        dict.push((b"V".to_vec(), value.clone()));
        dict.push((b"DV".to_vec(), value));
    }
    if spec.kind == FieldKind::List && !values.is_empty() {
        let indices = spec
            .options
            .iter()
            .enumerate()
            .filter(|(_, option)| values.contains(&option.value))
            .map(|(i, _)| Object::Int(i as i64))
            .collect();
        dict.push((b"I".to_vec(), Object::Array(indices)));
    }
    dict.push((b"MK".to_vec(), Object::Dict(mk.clone())));
    dict.push((b"BS".to_vec(), Object::Dict(bs.clone())));

    let caption = (spec.kind == FieldKind::PushButton).then(|| spec.caption.as_bytes().to_vec());
    let look = Look::new(rect, &da, Some(spec.align.quadding()), &mk, Some(&bs), None, caption.as_deref());
    if spec.kind == FieldKind::Checkbox {
        let (on, off) = look.button(ButtonStyle::Check);
        let (on, off) = (writer.write_appearance(&on)?, writer.write_appearance(&off)?);
        let ticked = !values.is_empty();
        dict.push((b"AS".to_vec(), Object::Name(if ticked { b"Yes".to_vec() } else { b"Off".to_vec() })));
        dict.push((b"AP".to_vec(), states(b"Yes", on, off)));
    } else {
        let filled = spec.as_field(values);
        if let Some(appearance) = value_appearance(&look, &filled, 0)? {
            let stream = writer.write_appearance(&appearance)?;
            dict.push((b"AP".to_vec(), Object::Dict(Dict(vec![(b"N".to_vec(), Object::Ref(stream, 0))]))));
        }
    }
    write_new(writer, number, Dict(dict))?;
    edits.annots(leaf)?.push(Object::Ref(number, 0));
    Ok(number)
}

/// Writes a radio button of the group `parent`, its "on" state named
/// `state`, and adds it to the page; returns its number. `chosen`: it is
/// shown on.
#[allow(clippy::too_many_arguments)]
fn add_radio_button(
    writer: &mut Writer<'_, '_>,
    edits: &mut Edits<'_, '_>,
    leaf: &Leaf,
    rect: [f32; 4],
    spec: &FieldSpec,
    state: &[u8],
    (parent, parent_generation): (u32, u16),
    chosen: bool,
) -> Result<u32> {
    let number = writer.take_number()?;
    let mk = spec.mk(upright(leaf));
    let bs = border_style();
    let look = Look::new(rect, &spec.da([0.0; 3]), None, &mk, Some(&bs), None, Some(b"l"));
    let (on, off) = look.button(ButtonStyle::Radio);
    let (on, off) = (writer.write_appearance(&on)?, writer.write_appearance(&off)?);
    let dict = Dict(vec![
        (b"Type".to_vec(), Object::Name(b"Annot".to_vec())),
        (b"Subtype".to_vec(), Object::Name(b"Widget".to_vec())),
        (b"Rect".to_vec(), reals(&rect)),
        (b"F".to_vec(), Object::Int(4)),
        (b"P".to_vec(), Object::Ref(leaf.number, leaf.generation)),
        (b"Parent".to_vec(), Object::Ref(parent, parent_generation)),
        (b"MK".to_vec(), Object::Dict(mk)),
        (b"BS".to_vec(), Object::Dict(bs)),
        (b"AS".to_vec(), Object::Name(if chosen { state.to_vec() } else { b"Off".to_vec() })),
        (b"AP".to_vec(), states(state, on, off)),
    ]);
    write_new(writer, number, dict)?;
    edits.annots(leaf)?.push(Object::Ref(number, 0));
    Ok(number)
}

fn border_style() -> Dict {
    Dict(vec![(b"W".to_vec(), Object::Int(1)), (b"S".to_vec(), Object::Name(b"S".to_vec()))])
}

/// A button's normal appearances: `on`, and `Off`.
fn states(on_name: &[u8], on: u32, off: u32) -> Object {
    let states = Dict(vec![(on_name.to_vec(), Object::Ref(on, 0)), (b"Off".to_vec(), Object::Ref(off, 0))]);
    Object::Dict(Dict(vec![(b"N".to_vec(), Object::Dict(states))]))
}

/// Writes a new object, its strings encrypted under its number.
fn write_new(writer: &mut Writer<'_, '_>, number: u32, dict: Dict) -> Result<()> {
    let mut object = Object::Dict(dict);
    if let Some(crypt) = writer.crypt()? {
        crypt.encrypt_strings(number, 0, &mut object);
    }
    writer.update.object(number, 0, &object);
    Ok(())
}

/// The standard fonts `/DR` names for our `/DA`s.
fn standard_font(base: &[u8]) -> Dict {
    let mut dict = Dict(vec![
        (b"Type".to_vec(), Object::Name(b"Font".to_vec())),
        (b"Subtype".to_vec(), Object::Name(b"Type1".to_vec())),
        (b"BaseFont".to_vec(), Object::Name(base.to_vec())),
    ]);
    if base != b"ZapfDingbats" {
        dict.0.push((b"Encoding".to_vec(), Object::Name(b"WinAnsiEncoding".to_vec())));
    }
    dict
}

const DR_FONTS: [(&[u8], &[u8]); 3] =
    [(SANS_FONT, b"Helvetica"), (NASKH_FONT, b"Times-Roman"), (b"ZaDb", b"ZapfDingbats")];

/// A document without a form gets one: `/AcroForm` in the catalog, with
/// `field` its first field, and our fonts in `/DR`.
fn new_acro_form(
    writer: &mut Writer<'_, '_>,
    edits: &mut Edits<'_, '_>,
    file: &File<'_>,
    field: u32,
) -> Result<()> {
    let root = file.trailer.get(b"Root").ok_or(Error::EditUnsupported("it has no catalog"))?;
    let Object::Ref(number, generation) = *root else {
        return Err(Error::EditUnsupported("its catalog is not an object of its own"));
    };
    let catalog = resolve_dict(file, root).ok_or(Error::EditUnsupported("its catalog cannot be read"))?;
    let mut fonts = Dict::default();
    for (name, base) in DR_FONTS {
        let font = writer.take_number()?;
        writer.update.object(font, 0, &Object::Dict(standard_font(base)));
        fonts.0.push((name.to_vec(), Object::Ref(font, 0)));
    }
    let acro = writer.take_number()?;
    let dict = Dict(vec![
        (b"Fields".to_vec(), Object::Array(vec![Object::Ref(field, 0)])),
        (b"DA".to_vec(), Object::Str(b"/MirsamSans 0 Tf 0 g".to_vec())),
        (b"DR".to_vec(), Object::Dict(Dict(vec![(b"Font".to_vec(), Object::Dict(fonts))]))),
    ]);
    write_new(writer, acro, dict)?;
    let id = Some((number, generation));
    let node = Node { id, owner: id, dict: catalog };
    set(edits.dicts.entry(&node)?, b"AcroForm", Object::Ref(acro, 0));
    Ok(())
}

/// Adds our fonts to the form's `/DR /Font` where they are missing.
fn ensure_fonts(writer: &mut Writer<'_, '_>, edits: &mut Edits<'_, '_>, form: &Form) -> Result<()> {
    let file = edits.file();
    let acro = edits.acro_view(form);
    let dr_entry = acro.get(b"DR").cloned();
    let dr = dr_entry.as_ref().and_then(|dr| resolve_dict(file, dr)).unwrap_or_default();
    let font_entry = dr.get(b"Font").cloned();
    let fonts = font_entry.as_ref().and_then(|f| resolve_dict(file, f)).unwrap_or_default();
    let missing: Vec<_> = DR_FONTS.iter().filter(|(name, _)| fonts.get(name).is_none()).collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut new_fonts = fonts.clone();
    for (name, base) in missing {
        let font = writer.take_number()?;
        writer.update.object(font, 0, &Object::Dict(standard_font(base)));
        new_fonts.0.push((name.to_vec(), Object::Ref(font, 0)));
    }
    // Where the font dictionary lives decides what is written again.
    match (&dr_entry, &font_entry) {
        (_, Some(Object::Ref(number, generation))) => {
            let id = Some((*number, *generation));
            let node = Node { id, owner: id, dict: fonts };
            *edits.dicts.entry(&node)? = new_fonts;
        }
        (Some(Object::Ref(number, generation)), _) => {
            let id = Some((*number, *generation));
            let node = Node { id, owner: id, dict: dr };
            set(edits.dicts.entry(&node)?, b"Font", Object::Dict(new_fonts));
        }
        _ => {
            let mut dr = dr;
            set(&mut dr, b"Font", Object::Dict(new_fonts));
            set(edits.acro(form)?, b"DR", Object::Dict(dr));
        }
    }
    Ok(())
}

/// The spec a field has now (for a new radio button: its look).
fn spec_of(info: &FormField) -> FieldSpec {
    FieldSpec {
        kind: info.kind,
        name: info.partial_name.clone(),
        tooltip: info.tooltip.clone(),
        required: info.required,
        read_only: info.read_only,
        multiline: info.multiline,
        max_len: info.max_len,
        default_values: info.default_values.clone(),
        align: info.align,
        font: info.font,
        font_size: info.font_size,
        options: info.options.clone(),
        multi_select: info.multi_select,
        editable: info.editable,
        bordered: info.bordered,
        caption: info.caption.clone(),
    }
}

/// Encrypts `value`'s strings for object `owner`, where the file is encrypted.
fn sealed(writer: &mut Writer<'_, '_>, owner: Option<(u32, u16)>, mut value: Object) -> Result<Object> {
    if let (Some(crypt), Some((number, generation))) = (writer.crypt()?, owner) {
        crypt.encrypt_strings(number, generation, &mut value);
    }
    Ok(value)
}

/// Sets a field's properties, then draws its widgets again.
fn update_field(
    writer: &mut Writer<'_, '_>,
    edits: &mut Edits<'_, '_>,
    form: &Form,
    target: &super::Field,
    spec: &FieldSpec,
) -> Result<()> {
    let info = &target.info;
    if spec.kind != info.kind {
        return Err(Error::InvalidEdit("a field's kind cannot be changed"));
    }
    if info.kind == FieldKind::Signature {
        return Err(Error::InvalidEdit("signature fields are made by signing"));
    }
    let node = &target.chain[0];
    let owner = node.id;
    if owner.is_none() {
        return Err(Error::EditUnsupported("this form field is not an object of its own"));
    }
    let name = spec.name.trim();
    let prefix = info.name.strip_suffix(info.partial_name.as_str()).unwrap_or("");
    let full = format!("{prefix}{name}");
    if name != info.partial_name && taken(form, &full, Some(info.id)) {
        return Err(Error::InvalidEdit("a field with this name exists already"));
    }
    if info.kind == FieldKind::Radio && spec.options.len() != target.widgets.len() {
        return Err(Error::InvalidEdit("a radio group has one choice per button"));
    }

    let reader = edits.reader;
    let old_da = reader.da(&target.chain, &form.acro);
    let (_, color) = parse_da(&old_da);
    let font_changed = spec.font.is_some() && (spec.font != info.font || spec.font_size != info.font_size);
    let size_changed = spec.font.is_none() && spec.font_size != info.font_size;
    let new_da = if font_changed || (info.font.is_some() && spec.font.is_some()) {
        Some(spec.da(color))
    } else if size_changed {
        // The field's own font, at the new size.
        let font = da_font(&old_da).unwrap_or_else(|| b"Helv".to_vec());
        let [r, g, b] = color;
        Some(
            format!(
                "/{} {} Tf {} {} {} rg",
                String::from_utf8_lossy(&font),
                num(spec.font_size),
                num(r),
                num(g),
                num(b)
            )
            .into_bytes(),
        )
    } else {
        None
    };
    // Buttons keep their ZapfDingbats `/DA`.
    let new_da = new_da.filter(|_| !matches!(info.kind, FieldKind::Checkbox | FieldKind::Radio));

    let flags_mask = READ_ONLY | REQUIRED | MULTILINE | COMBO | EDIT | MULTI_SELECT | RADIO | PUSH_BUTTON;
    let old_flags = reader.inherited(&target.chain, b"Ff").and_then(|(v, _)| v.as_int()).unwrap_or(0);
    let flags = (old_flags & !flags_mask) | spec.flags();

    let values_changed = spec.default_values != info.default_values;
    let options_changed =
        matches!(info.kind, FieldKind::Combo | FieldKind::List) && spec.shown_options() != info.options;

    // The values shown from now on.
    let mut values = if values_changed { spec.default_values.clone() } else { info.values.clone() };
    values.retain(|v| !v.is_empty());
    if options_changed && !spec.editable {
        values.retain(|v| spec.options.iter().any(|o| &o.value == v));
    }
    if info.kind == FieldKind::Text
        && let (Some(max), Some(text)) = (spec.max_len, values.first_mut())
        && text.chars().count() > max
    {
        *text = text.chars().take(max).collect();
    }
    if info.kind == FieldKind::List && !spec.multi_select {
        values.truncate(1);
    }

    {
        let mut entries: Vec<(&[u8], Option<Object>)> = vec![
            (b"T", Some(sealed(writer, owner, Object::Str(text_string(name)))?)),
            (
                b"TU",
                if spec.tooltip.is_empty() {
                    None
                } else {
                    Some(sealed(writer, owner, Object::Str(text_string(&spec.tooltip)))?)
                },
            ),
            (b"Ff", Some(Object::Int(flags))),
        ];
        if let Some(da) = &new_da {
            entries.push((b"DA", Some(sealed(writer, owner, Object::Str(da.clone()))?)));
        }
        if !matches!(info.kind, FieldKind::Checkbox | FieldKind::Radio | FieldKind::PushButton) {
            entries.push((b"Q", Some(Object::Int(spec.align.quadding()))));
        }
        if info.kind == FieldKind::Text {
            entries
                .push((b"MaxLen", spec.max_len.map(|n| Object::Int(i64::try_from(n).unwrap_or(i64::MAX)))));
        }
        if options_changed {
            entries.push((b"Opt", Some(sealed(writer, owner, spec.opt())?)));
        }
        // A radio group's values are its buttons' positions, set below.
        if info.kind != FieldKind::Radio {
            let value_object = spec.value_object(&values);
            let default_object = spec.value_object(&spec.default_values);
            if values_changed || options_changed || info.kind == FieldKind::Text {
                entries.push((b"V", value_object.map(|v| sealed(writer, owner, v)).transpose()?));
            }
            if values_changed {
                entries.push((b"DV", default_object.map(|v| sealed(writer, owner, v)).transpose()?));
            }
        }
        if info.kind == FieldKind::List {
            let indices: Vec<Object> = spec
                .options
                .iter()
                .enumerate()
                .filter(|(_, option)| values.contains(&option.value))
                .map(|(i, _)| Object::Int(i as i64))
                .collect();
            entries.push((b"I", (!indices.is_empty()).then_some(Object::Array(indices))));
        }
        let dict = edits.dicts.entry(node)?;
        for (key, value) in entries {
            match value {
                Some(value) => set(dict, key, value),
                None => dict.0.retain(|(k, _)| k.as_slice() != key),
            }
        }
    }

    // Each widget: its border, a button's caption, and a check box's state shown.
    let shown_on = values.first().map(|v| v.as_bytes().to_vec());
    for widget in &target.widgets {
        let mk_owner = widget.node.owner;
        let mut mk =
            widget.node.dict.get(b"MK").and_then(|mk| resolve_dict(reader.file, mk)).unwrap_or_default();
        if spec.bordered {
            if mk.get(b"BC").is_none() || !info.bordered {
                set(&mut mk, b"BC", Object::Array(vec![Object::Real(f64::from(BORDER))]));
            }
        } else {
            mk.0.retain(|(key, _)| key != b"BC");
        }
        if info.kind == FieldKind::PushButton {
            set(&mut mk, b"CA", sealed(writer, mk_owner, Object::Str(text_string(&spec.caption)))?);
        }
        let dict = edits.dicts.entry(&widget.node)?;
        set(dict, b"MK", Object::Dict(mk));
        if info.kind == FieldKind::Checkbox {
            let shown = match (&widget.on, &shown_on) {
                (Some(on), Some(chosen)) if on == chosen => on.clone(),
                _ => b"Off".to_vec(),
            };
            set(dict, b"AS", Object::Name(shown));
        }
    }

    // A radio group: its buttons named by position, their export values
    // in `/Opt`, and the chosen one by its new name (ADR 0047).
    if info.kind == FieldKind::Radio {
        let exports: Vec<String> = spec.options.iter().map(|option| export_of(option).to_string()).collect();
        let order: Vec<usize> = (0..target.widgets.len()).collect();
        renumber(writer, edits, target, &order, &exports)?;
        if values_changed {
            let chosen = spec.default_values.first().and_then(|v| spec.radio_index(v));
            set_radio_choice(edits, target, chosen, true)?;
        }
    }

    let mut filled = spec.as_field(values);
    filled.comb = info.comb;
    filled.password = info.password;
    filled.options = if matches!(info.kind, FieldKind::Combo | FieldKind::List) {
        spec.shown_options()
    } else {
        info.options.clone()
    };
    let redraw_buttons = spec.bordered != info.bordered;
    redraw_with(writer, edits, form, target, &filled, None, redraw_buttons, new_da.as_deref())
}

/// Each radio button's export value, in widget order: its `/Opt` entry or
/// its state's name. Groups whose buttons share a state (buttons that turn
/// on together) are not changed here.
fn radio_exports(info: &FormField, target: &super::Field) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    target
        .widgets
        .iter()
        .map(|widget| {
            let state = widget.on.as_ref().ok_or(Error::EditUnsupported("this radio button has no state"))?;
            if !seen.insert(state.clone()) {
                return Err(Error::EditUnsupported("this radio group's buttons share a value"));
            }
            let name = String::from_utf8_lossy(state).into_owned();
            Ok(info
                .options
                .iter()
                .find(|option| option.value == name)
                .map_or(name, |option| option.label.clone()))
        })
        .collect()
}

/// Names a radio group's buttons by position (ISO 32000-2, 12.7.5.2.4):
/// the buttons at `order` (old widget indices, in their new order) take
/// the names 0, 1, …, with `exports` as `/Opt`; the value follows its
/// button, and is off when its button went.
fn renumber(
    writer: &mut Writer<'_, '_>,
    edits: &mut Edits<'_, '_>,
    target: &super::Field,
    order: &[usize],
    exports: &[String],
) -> Result<()> {
    let reader = edits.reader;
    let renamed = |state: &[u8]| -> Option<Vec<u8>> {
        let old = target.widgets.iter().position(|w| w.on.as_deref() == Some(state))?;
        order.iter().position(|i| *i == old).map(position_name)
    };
    for (new, old) in order.iter().enumerate() {
        let widget = &target.widgets[*old];
        let Some(state) = widget.on.clone() else { continue };
        let name = position_name(new);
        let current =
            widget.node.id.and_then(|id| edits.dicts.0.get(&id)).unwrap_or(&widget.node.dict).clone();
        let ap = current.get(b"AP").and_then(|ap| resolve_dict(reader.file, ap));
        let shown = current.get(b"AS").and_then(Object::as_name).map(<[u8]>::to_vec);
        let dict = edits.dicts.entry(&widget.node)?;
        if let Some(mut ap) = ap {
            for key in [&b"N"[..], b"D"] {
                let Some(states) = ap.get(key).and_then(|s| resolve_dict(reader.file, s)) else { continue };
                let states = Dict(
                    states
                        .0
                        .into_iter()
                        .map(|(k, v)| (if k == state { name.clone() } else { k }, v))
                        .collect(),
                );
                set(&mut ap, key, Object::Dict(states));
            }
            set(dict, b"AP", Object::Dict(ap));
        }
        if shown.as_deref() == Some(state.as_slice()) {
            set(dict, b"AS", Object::Name(name));
        }
    }
    let node = &target.chain[0];
    let opt = sealed(writer, node.id, opt_of(exports.iter().map(String::as_str)))?;
    let value = |key: &[u8]| -> Option<Object> {
        match node.dict.get(key) {
            Some(Object::Name(state)) if state != b"Off" => {
                Some(Object::Name(renamed(state).unwrap_or_else(|| b"Off".to_vec())))
            }
            _ => None,
        }
    };
    let (v, dv) = (value(b"V"), value(b"DV"));
    let dict = edits.dicts.entry(node)?;
    set(dict, b"Opt", opt);
    if let Some(v) = v {
        set(dict, b"V", v);
    }
    match dv {
        Some(Object::Name(off)) if off == b"Off" => dict.0.retain(|(key, _)| key != b"DV"),
        Some(dv) => set(dict, b"DV", dv),
        None => {}
    }
    Ok(())
}

/// Chooses radio button `chosen` (by position; none: no button), as the
/// group's value and, with `default`, its default.
fn set_radio_choice(
    edits: &mut Edits<'_, '_>,
    target: &super::Field,
    chosen: Option<usize>,
    default: bool,
) -> Result<()> {
    let name = chosen.map_or_else(|| b"Off".to_vec(), position_name);
    for (i, widget) in target.widgets.iter().enumerate() {
        let shown = if Some(i) == chosen { name.clone() } else { b"Off".to_vec() };
        set(edits.dicts.entry(&widget.node)?, b"AS", Object::Name(shown));
    }
    let dict = edits.dicts.entry(&target.chain[0])?;
    set(dict, b"V", Object::Name(name.clone()));
    if default {
        match chosen {
            Some(_) => set(dict, b"DV", Object::Name(name)),
            None => dict.0.retain(|(key, _)| key != b"DV"),
        }
    }
    Ok(())
}

/// Draws a field's widgets again (`only`: one of them), showing `filled`;
/// buttons' states too when `buttons`.
fn redraw(
    writer: &mut Writer<'_, '_>,
    edits: &mut Edits<'_, '_>,
    form: &Form,
    target: &super::Field,
    filled: &FormField,
    only: Option<usize>,
    buttons: bool,
) -> Result<()> {
    redraw_with(writer, edits, form, target, filled, only, buttons, None)
}

#[allow(clippy::too_many_arguments)]
fn redraw_with(
    writer: &mut Writer<'_, '_>,
    edits: &mut Edits<'_, '_>,
    form: &Form,
    target: &super::Field,
    filled: &FormField,
    only: Option<usize>,
    buttons: bool,
    da: Option<&[u8]>,
) -> Result<()> {
    let reader = edits.reader;
    let da = da.map_or_else(|| reader.da(&target.chain, &form.acro), <[u8]>::to_vec);
    let quadding = match filled.kind {
        FieldKind::Text | FieldKind::Combo | FieldKind::List => Some(filled.align.quadding()),
        _ => None,
    };
    let top = target.chain[0]
        .dict
        .get(b"TI")
        .and_then(Object::as_int)
        .and_then(|top| usize::try_from(top).ok())
        .unwrap_or(0);
    for (i, widget) in target.widgets.iter().enumerate() {
        if only.is_some_and(|only| only != i) {
            continue;
        }
        // The widget as this update leaves it.
        let dict = widget.node.id.and_then(|id| edits.dicts.0.get(&id)).unwrap_or(&widget.node.dict).clone();
        let Some(rect) = rect_of(reader.file, &dict) else { continue };
        let mk = dict.get(b"MK").and_then(|mk| resolve_dict(reader.file, mk)).unwrap_or_default();
        let bs = dict.get(b"BS").and_then(|bs| resolve_dict(reader.file, bs));
        let caption = (filled.kind == FieldKind::PushButton).then(|| filled.caption.as_bytes().to_vec());
        let caption = caption.or_else(|| {
            let ca = mk.get(b"CA")?;
            reader.string(&reader.file.resolve(ca)?, widget.node.owner)
        });
        let look = Look::new(rect, &da, quadding, &mk, bs.as_ref(), dict.get(b"Border"), caption.as_deref());
        match filled.kind {
            FieldKind::Checkbox | FieldKind::Radio => {
                if !buttons {
                    continue;
                }
                let Some(on) = dict
                    .get(b"AP")
                    .and_then(|ap| resolve_dict(reader.file, ap))
                    .and_then(|ap| ap.get(b"N").and_then(|n| resolve_dict(reader.file, n)))
                    .and_then(|states| {
                        states.0.iter().find(|(name, _)| name != b"Off").map(|(n, _)| n.clone())
                    })
                    .or_else(|| widget.on.clone())
                else {
                    continue;
                };
                let style =
                    if filled.kind == FieldKind::Radio { ButtonStyle::Radio } else { ButtonStyle::Check };
                let (on_look, off_look) = look.button(style);
                let (on_stream, off_stream) =
                    (writer.write_appearance(&on_look)?, writer.write_appearance(&off_look)?);
                set(edits.dicts.entry(&widget.node)?, b"AP", states(&on, on_stream, off_stream));
            }
            _ => {
                let Some(appearance) = value_appearance(&look, filled, top)? else { continue };
                let stream = writer.write_appearance(&appearance)?;
                let ap = Object::Dict(Dict(vec![(b"N".to_vec(), Object::Ref(stream, 0))]));
                set(edits.dicts.entry(&widget.node)?, b"AP", ap);
            }
        }
    }
    Ok(())
}

/// Takes `references` out of every page's `/Annots`.
fn remove_from_pages(edits: &mut Edits<'_, '_>, tree: &PageTree, references: &[Object]) -> Result<()> {
    for leaf in &tree.leaves {
        let Ok(annots) = Annots::read(edits.file(), leaf) else { continue };
        if !annots.items.iter().any(|item| references.contains(item)) {
            continue;
        }
        edits.annots(leaf)?.retain(|item| !references.contains(item));
    }
    Ok(())
}

/// Removes a field: its widgets from their pages, and the field from its
/// parent's `/Kids` or the form's `/Fields` (and `/CO`); a parent left with
/// no kids goes too.
fn remove_field(
    edits: &mut Edits<'_, '_>,
    tree: &PageTree,
    form: &Form,
    target: &super::Field,
) -> Result<()> {
    let own = |node: &Node| node.id.map(|(n, g)| Object::Ref(n, g));
    let widgets: Vec<Object> = target.widgets.iter().filter_map(|w| own(&w.node)).collect();
    remove_from_pages(edits, tree, &widgets)?;
    let mut doomed =
        own(&target.chain[0]).ok_or(Error::EditUnsupported("this form field is not an object of its own"))?;
    for parent in &target.chain[1..] {
        let Some(kids) = edits.array_in(ArrayHolder::Node(parent), b"Kids")? else { break };
        kids.retain(|kid| kid != &doomed);
        if !kids.is_empty() {
            doomed = Object::Null;
            break;
        }
        match own(parent) {
            Some(reference) => doomed = reference,
            None => break,
        }
    }
    if doomed != Object::Null {
        if let Some(fields) = edits.array_in(ArrayHolder::Acro(form), b"Fields")? {
            fields.retain(|field| field != &doomed);
        }
        let reference = own(&target.chain[0]);
        if let Some(order) = edits.array_in(ArrayHolder::Acro(form), b"CO")? {
            order.retain(|field| Some(field) != reference.as_ref() && field != &doomed);
        }
    }
    Ok(())
}

/// Puts `fields`' widgets on `leaf` first in Tab order, in that order, and
/// says the page follows its `/Annots` (`/Tabs /W`).
fn tab_order(edits: &mut Edits<'_, '_>, form: &Form, leaf: &Leaf, fields: &[u32]) -> Result<()> {
    let annots = Annots::read(edits.file(), leaf)?;
    let widget_of = |item: &Object| -> Option<(u32, usize)> {
        form.fields.iter().find_map(|field| {
            let at = field
                .widgets
                .iter()
                .position(|w| w.node.id.map(|(n, g)| Object::Ref(n, g)).as_ref() == Some(item))?;
            Some((field.info.id, at))
        })
    };
    let positions: Vec<(usize, Object, (u32, usize))> = annots
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| Some((i, item.clone(), widget_of(item)?)))
        .collect();
    let rank = |field: u32| fields.iter().position(|f| *f == field).unwrap_or(usize::MAX);
    let mut ordered: Vec<&(usize, Object, (u32, usize))> = positions.iter().collect();
    // Stable: unlisted fields, and a group's buttons, keep their order.
    ordered.sort_by_key(|(_, _, (field, _))| rank(*field));
    let mut items = annots.items.clone();
    for ((slot, _, _), (_, item, _)) in positions.iter().zip(ordered) {
        items[*slot] = item.clone();
    }
    *edits.annots(leaf)? = items;
    let page = page_node(leaf);
    set(edits.dicts.entry(&page)?, b"Tabs", Object::Name(b"W".to_vec()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(kind: FieldKind, name: &str) -> FieldSpec {
        FieldSpec {
            kind,
            name: name.into(),
            tooltip: String::new(),
            required: false,
            read_only: false,
            multiline: false,
            max_len: None,
            default_values: Vec::new(),
            align: FieldAlign::Start,
            font: Some(FontFamily::Sans),
            font_size: 0.0,
            options: Vec::new(),
            multi_select: false,
            editable: false,
            bordered: true,
            caption: String::new(),
        }
    }

    fn option(value: &str) -> FieldOption {
        FieldOption { value: value.into(), label: value.into() }
    }

    #[test]
    fn specs_are_checked() {
        assert!(spec(FieldKind::Text, "name").check().is_ok());
        assert!(spec(FieldKind::Text, " ").check().is_err(), "a name");
        assert!(spec(FieldKind::Text, "a.b").check().is_err(), "no full stops");
        assert!(spec(FieldKind::Signature, "sig").check().is_err());
        assert!(FieldSpec { font_size: 2.0, ..spec(FieldKind::Text, "x") }.check().is_err());
        let long =
            FieldSpec { max_len: Some(3), default_values: vec!["four".into()], ..spec(FieldKind::Text, "x") };
        assert!(long.check().is_err());
        let radio = spec(FieldKind::Radio, "choice");
        assert!(radio.check().is_err(), "a radio group needs a button");
        let radio = FieldSpec { options: vec![option("نعم"), option("لا")], ..radio };
        assert!(radio.check().is_ok());
        let twice = FieldSpec { options: vec![option("a"), option("a")], ..radio.clone() };
        assert!(twice.check().is_err(), "values are unique");
        let unknown = FieldSpec { default_values: vec!["x".into()], ..radio };
        assert!(unknown.check().is_err());
        let ticked = FieldSpec { default_values: vec!["Yes".into()], ..spec(FieldKind::Checkbox, "ok") };
        assert!(ticked.check().is_ok());
    }

    #[test]
    fn flags_and_default_appearance_follow_the_spec() {
        let text = FieldSpec { required: true, multiline: true, ..spec(FieldKind::Text, "x") };
        assert_eq!(text.flags(), REQUIRED | MULTILINE);
        let combo = FieldSpec { editable: true, ..spec(FieldKind::Combo, "x") };
        assert_eq!(combo.flags(), COMBO | EDIT);
        assert_eq!(spec(FieldKind::Radio, "x").flags(), RADIO | NO_TOGGLE_TO_OFF);
        let naskh =
            FieldSpec { font: Some(FontFamily::Naskh), font_size: 11.0, ..spec(FieldKind::Text, "x") };
        assert_eq!(naskh.da([0.0; 3]), b"/MirsamNaskh 11 Tf 0 g");
        assert_eq!(spec(FieldKind::Text, "x").da([0.0, 0.0, 1.0]), b"/MirsamSans 0 Tf 0 0 1 rg");
        assert_eq!(spec(FieldKind::Checkbox, "x").da([0.0; 3]), b"/ZaDb 0 Tf 0 g");
    }

    #[test]
    fn boxes_are_normalized_and_kept_sensible() {
        assert_eq!(check_rect([100.0, 50.0, 10.0, 20.0]).ok(), Some([10.0, 20.0, 100.0, 50.0]));
        assert!(check_rect([0.0, 0.0, 2.0, 20.0]).is_err());
        assert!(check_rect([0.0, 0.0, f32::NAN, 20.0]).is_err());
    }
}
