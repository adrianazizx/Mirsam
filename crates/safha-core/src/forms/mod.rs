//! Interactive forms (AcroForm, ISO 32000-2, 12.7; ADR 0045): reading a
//! document's fields, and filling them in as incremental updates.
//!
//! Filling a field sets its value (`/V`) and writes each of its widgets a new
//! appearance stream of our own, shaped by the text engine and drawn with
//! embedded font subsets (spec §6.4), so Arabic is joined and ordered the
//! same in every viewer. We never leave the drawing to `NeedAppearances`:
//! when a document asks viewers to make appearances themselves, the first
//! fill makes them for every field that has a value and turns the flag off,
//! so PDFium and other readers draw what we drew.
//!
//! Check boxes and radio buttons switch between the "on" and "Off"
//! appearances their widgets already have (`/AS`); a widget without them is
//! given a tick, a cross, or a dot drawn as paths.
//!
//! Fields are found with the crate's own bounded parser, as annotations are.
//! In an encrypted file, strings are decrypted to read them and new ones are
//! encrypted; fields inside encrypted object streams cannot be read
//! (`pdf::File` does not decrypt those) and are not listed.

mod appearance;
mod design;
mod detect;

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

pub use design::{FieldDesign, FieldSpec, MAX_FIELD_NAME, design_update};
pub use detect::{DetectedField, PageShapes, Segment, detect_fields, unique_name};

use crate::fonts::FontFamily;

use crate::annotate::{Annots, Appearance, Writer, rect_of, resolve_dict, text_string};
use crate::edit::{PageTree, Start, set};
use crate::error::{Error, Result};
use crate::pdf::crypt::Crypt;
use crate::pdf::{Dict, File, Object};
use crate::view::ViewRect;

use appearance::{ButtonStyle, Look};

/// What kind of field it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldKind {
    Text,
    Checkbox,
    Radio,
    /// A drop-down list (`/Ch` with the Combo flag).
    Combo,
    /// A list box.
    List,
    PushButton,
    Signature,
}

/// Where a field's text sits (`/Q`). In a field made here (ADR 0047) the
/// start and end are those of the text's own direction: the start of Arabic
/// is the right. In a field from elsewhere, `/Q 0` starts where the text's
/// direction starts and `/Q 2` is the right (ADR 0045).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldAlign {
    #[default]
    Start,
    Center,
    End,
}

impl FieldAlign {
    fn quadding(self) -> i64 {
        match self {
            FieldAlign::Start => 0,
            FieldAlign::Center => 1,
            FieldAlign::End => 2,
        }
    }

    fn of_quadding(q: i64) -> Self {
        match q {
            1 => FieldAlign::Center,
            2 => FieldAlign::End,
            _ => FieldAlign::Start,
        }
    }
}

/// A choice: what is stored (`/V`), and what is shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldOption {
    pub value: String,
    pub label: String,
}

/// One place a field is shown: a widget annotation.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldWidget {
    /// Its page, when it is in some page's `/Annots`.
    pub page: Option<usize>,
    /// Its `/Rect` in page space (left, bottom, right, top).
    pub rect: [f32; 4],
    /// The box on the displayed page ([`crate::Document::form`] fills it in).
    pub view: Option<ViewRect>,
    /// A check box's or radio button's "on" state.
    pub on_state: Option<String>,
    /// Its position in its page's `/Annots`: the tab order, where the page
    /// takes it from there (ADR 0047).
    pub annot_index: Option<usize>,
}

/// A field to fill in.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormField {
    /// Its object number: what [`FormEdit`] names it by.
    pub id: u32,
    /// The fully qualified name (`parent.child`).
    pub name: String,
    /// Its own part of the name (`/T`), what renaming changes (ADR 0047).
    pub partial_name: String,
    pub kind: FieldKind,
    /// What a reader is told about it (`/TU`).
    pub tooltip: String,
    /// What resetting the form returns it to (`/DV`), as [`FormField::values`].
    pub default_values: Vec<String>,
    pub align: FieldAlign,
    /// The bundled family it is set in, when it was made here (ADR 0047);
    /// none when `/DA` names a font of its own (then the Sans family draws it).
    pub font: Option<FontFamily>,
    /// Its text size; 0 fits the text to the box.
    pub font_size: f32,
    /// Its widgets have a border colour (`/MK /BC`).
    pub bordered: bool,
    /// A push button's label (`/MK /CA`).
    pub caption: String,
    /// Its value: the text of a text field or drop-down; the selected
    /// choices of a list; the "on" state of a checked box or the chosen
    /// radio button (empty when off).
    pub values: Vec<String>,
    /// A drop-down's or list's choices; the states of radio buttons and
    /// check boxes.
    pub options: Vec<FieldOption>,
    pub read_only: bool,
    pub required: bool,
    /// Text fields.
    pub multiline: bool,
    pub password: bool,
    pub comb: bool,
    pub max_len: Option<usize>,
    /// Lists that take several choices.
    pub multi_select: bool,
    /// Drop-downs that also take typed text.
    pub editable: bool,
    pub widgets: Vec<FieldWidget>,
}

/// A document's form.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormInfo {
    pub fields: Vec<FormField>,
    /// The document also has an XFA form, which is not supported (spec §5.5):
    /// readers that use XFA will not show what is filled in here.
    pub xfa: bool,
    /// The document asks viewers to draw its fields themselves; the first
    /// fill draws them all and turns that off.
    pub need_appearances: bool,
}

/// Sets a field's value. `values` as [`FormField::values`]: one text for a
/// text field or a drop-down (none: empty), the chosen values of a list,
/// the "on" state of a check box or radio button (none: off).
#[derive(Debug, Clone, PartialEq)]
pub struct FormEdit {
    pub field: u32,
    pub values: Vec<String>,
}

// Field flags (ISO 32000-2, tables 227, 229, 231, 233).
const READ_ONLY: i64 = 1;
const REQUIRED: i64 = 1 << 1;
const MULTILINE: i64 = 1 << 12;
const PASSWORD: i64 = 1 << 13;
const NO_TOGGLE_TO_OFF: i64 = 1 << 14;
const RADIO: i64 = 1 << 15;
const PUSH_BUTTON: i64 = 1 << 16;
const COMBO: i64 = 1 << 17;
const EDIT: i64 = 1 << 18;
const MULTI_SELECT: i64 = 1 << 21;
const COMB: i64 = 1 << 24;

/// The longest text a field takes, in UTF-16 code units.
pub const MAX_FIELD_TEXT: usize = 20_000;
/// Bounds for hostile field trees.
const MAX_FIELD_DEPTH: usize = 32;
const MAX_FIELDS: usize = 20_000;

/// A dictionary of the form tree, and the object whose strings it holds.
#[derive(Debug, Clone)]
struct Node {
    /// Its own object, when it is not direct.
    id: Option<(u32, u16)>,
    /// The object it is written in (its own, or its parent's).
    owner: Option<(u32, u16)>,
    dict: Dict,
}

#[derive(Debug, Clone)]
struct Widget {
    node: Node,
    /// Its "on" state as the file names it.
    on: Option<Vec<u8>>,
    /// It has no appearances for its states: we draw them.
    needs_states: bool,
}

#[derive(Debug)]
struct Field {
    info: FormField,
    /// The field, then its ancestors (for inherited entries).
    chain: Vec<Node>,
    widgets: Vec<Widget>,
}

/// The form as read from the newest revision.
struct Form {
    /// Where `/AcroForm` lives: its own object, or in the catalog's.
    acro: Node,
    /// The catalog, when `/AcroForm` is in it directly.
    catalog: Option<Node>,
    fields: Vec<Field>,
    need_appearances: bool,
    xfa: bool,
}

struct Reader<'a, 'f> {
    file: &'a File<'f>,
    crypt: Option<&'a Crypt>,
}

/// A document's form fields (none when it has no form).
pub fn read(bytes: &[u8], password: Option<&str>) -> Result<FormInfo> {
    let Start { file, .. } = Start::read(bytes)?;
    let crypt = Crypt::of(&file, password)?;
    let tree = PageTree::read(&file)?;
    let reader = Reader { file: &file, crypt: crypt.as_ref() };
    Ok(match reader.form(&tree) {
        Some(form) => FormInfo {
            fields: form.fields.into_iter().map(|field| field.info).collect(),
            xfa: form.xfa,
            need_appearances: form.need_appearances,
        },
        None => FormInfo { fields: Vec::new(), xfa: false, need_appearances: false },
    })
}

/// The bytes to append to `bytes` to apply `edit`: an incremental update.
pub fn form_update(bytes: &[u8], password: Option<&str>, edit: &FormEdit) -> Result<Vec<u8>> {
    let Start { file, section, form, mut next_number } = Start::read(bytes)?;
    let crypt = Crypt::of(&file, password)?;
    let tree = PageTree::read(&file)?;
    let reader = Reader { file: &file, crypt: crypt.as_ref() };
    let acro = reader.form(&tree).ok_or(Error::InvalidEdit("this document has no form"))?;
    let target = acro
        .fields
        .iter()
        .find(|field| field.info.id == edit.field)
        .ok_or(Error::InvalidEdit("no such form field"))?;

    let mut writer = Writer::new(&file, bytes, password, &mut next_number);
    let mut changed = Changed::default();
    let looks = Looks { reader: &reader, form: &acro };
    set_value(&looks, target, &edit.values, &mut writer, &mut changed)?;
    if acro.need_appearances {
        // Every other field with a value gets our appearance too, so no
        // reader has to make one.
        for field in acro.fields.iter().filter(|field| field.info.id != edit.field) {
            refresh(&looks, field, &mut writer, &mut changed)?;
        }
        let node = acro.catalog.as_ref().unwrap_or(&acro.acro);
        let dict = changed.entry(node)?;
        if acro.catalog.is_some() {
            if let Some((_, Object::Dict(inner))) = dict.0.iter_mut().find(|(key, _)| key == b"AcroForm") {
                set(inner, b"NeedAppearances", Object::Bool(false));
            }
        } else {
            set(dict, b"NeedAppearances", Object::Bool(false));
        }
    }
    for ((number, generation), dict) in changed.0 {
        writer.update.object(number, generation, &Object::Dict(dict));
    }
    let update = writer.update;
    Ok(update.finish(form, file.trailer.clone(), section, next_number))
}

/// Dictionaries changed by an update, written once each at its end.
#[derive(Default)]
struct Changed(BTreeMap<(u32, u16), Dict>);

impl Changed {
    fn entry(&mut self, node: &Node) -> Result<&mut Dict> {
        let id = node.id.ok_or(Error::EditUnsupported("this form field is not an object of its own"))?;
        Ok(self.0.entry(id).or_insert_with(|| node.dict.clone()))
    }
}

/// What the appearance of a field's widgets is drawn from.
struct Looks<'a, 'r, 'f> {
    reader: &'a Reader<'r, 'f>,
    form: &'a Form,
}

impl Looks<'_, '_, '_> {
    /// A widget's look: its box, colours and border, and the field's text
    /// size, colour, and alignment.
    fn of(&self, field: &Field, widget: &Widget) -> Option<Look> {
        let reader = self.reader;
        let rect = rect_of(reader.file, &widget.node.dict)?;
        let da = reader.da(&field.chain, &self.form.acro);
        let quadding = reader
            .inherited(&field.chain, b"Q")
            .and_then(|(value, _)| value.as_int())
            .or_else(|| self.form.acro.dict.get(b"Q").and_then(|q| reader.file.resolve(q)?.as_int()));
        let mk = widget.node.dict.get(b"MK").and_then(|mk| resolve_dict(reader.file, mk)).unwrap_or_default();
        let bs = widget.node.dict.get(b"BS").and_then(|bs| resolve_dict(reader.file, bs));
        let caption =
            mk.get(b"CA").and_then(|ca| reader.string(&reader.file.resolve(ca)?, widget.node.owner));
        Some(Look::new(
            rect,
            &da,
            quadding,
            &mk,
            bs.as_ref(),
            widget.node.dict.get(b"Border"),
            caption.as_deref(),
        ))
    }
}

/// Sets `field` to `values`, with its widgets' appearances.
fn set_value(
    looks: &Looks<'_, '_, '_>,
    field: &Field,
    values: &[String],
    writer: &mut Writer<'_, '_>,
    changed: &mut Changed,
) -> Result<()> {
    let info = &field.info;
    if info.read_only {
        return Err(Error::InvalidEdit("this field is read-only"));
    }
    let node = &field.chain[0];
    let value: Option<Object> = match info.kind {
        FieldKind::Text => {
            let text = values.first().map(String::as_str).unwrap_or("");
            let text = if info.multiline { text.to_string() } else { text.replace(['\r', '\n'], " ") };
            if text.encode_utf16().count() > MAX_FIELD_TEXT {
                return Err(Error::InvalidEdit("a field holds at most 20,000 characters"));
            }
            if info.max_len.is_some_and(|max| text.chars().count() > max) {
                return Err(Error::InvalidEdit("the text is longer than the field allows"));
            }
            Some(Object::Str(text_string(&text)))
        }
        FieldKind::Combo => {
            let text = values.first().map(String::as_str).unwrap_or("");
            let known = info.options.iter().any(|option| option.value == text);
            if !text.is_empty() && !known && !info.editable {
                return Err(Error::InvalidEdit("not one of the field's choices"));
            }
            if text.encode_utf16().count() > MAX_FIELD_TEXT {
                return Err(Error::InvalidEdit("a field holds at most 20,000 characters"));
            }
            Some(Object::Str(text_string(text)))
        }
        FieldKind::List => {
            if values.len() > 1 && !info.multi_select {
                return Err(Error::InvalidEdit("this list takes one choice"));
            }
            if values.iter().any(|value| !info.options.iter().any(|option| &option.value == value)) {
                return Err(Error::InvalidEdit("not one of the field's choices"));
            }
            match values {
                [] => None,
                [one] => Some(Object::Str(text_string(one))),
                many => {
                    Some(Object::Array(many.iter().map(|value| Object::Str(text_string(value))).collect()))
                }
            }
        }
        FieldKind::Checkbox | FieldKind::Radio => {
            let wanted = values.first().filter(|value| !value.is_empty() && value.as_str() != "Off");
            let state = match wanted {
                Some(wanted) => Some(
                    field
                        .widgets
                        .iter()
                        .filter_map(|widget| widget.on.clone())
                        .find(|on| String::from_utf8_lossy(on) == wanted.as_str())
                        .ok_or(Error::InvalidEdit("not one of the field's choices"))?,
                ),
                None => None,
            };
            let name = state.clone().unwrap_or_else(|| b"Off".to_vec());
            for widget in &field.widgets {
                let on = widget.on.as_ref().is_some_and(|on| Some(on) == state.as_ref());
                if widget.needs_states {
                    draw_states(looks, field, widget, writer, changed)?;
                }
                let dict = changed.entry(&widget.node)?;
                let shown = if on { widget.on.clone().unwrap_or_default() } else { b"Off".to_vec() };
                set(dict, b"AS", Object::Name(shown));
            }
            let dict = changed.entry(node)?;
            set(dict, b"V", Object::Name(name));
            return Ok(());
        }
        FieldKind::PushButton | FieldKind::Signature => {
            return Err(Error::InvalidEdit("this field cannot be filled in here"));
        }
    };

    let owner = node.id.ok_or(Error::EditUnsupported("this form field is not an object of its own"))?;
    let crypt = writer.crypt()?.cloned();
    let dict = changed.entry(node)?;
    match value {
        Some(mut value) => {
            if let Some(crypt) = &crypt {
                crypt.encrypt_strings(owner.0, owner.1, &mut value);
            }
            set(dict, b"V", value);
        }
        None => dict.0.retain(|(key, _)| key != b"V"),
    }
    if info.kind == FieldKind::List {
        let indices: Vec<Object> = info
            .options
            .iter()
            .enumerate()
            .filter(|(_, option)| values.contains(&option.value))
            .map(|(i, _)| Object::Int(i as i64))
            .collect();
        if indices.is_empty() {
            dict.0.retain(|(key, _)| key != b"I");
        } else {
            set(dict, b"I", Object::Array(indices));
        }
    } else if info.kind == FieldKind::Combo {
        dict.0.retain(|(key, _)| key != b"I");
    }
    let filled = FormField { values: values.to_vec(), ..info.clone() };
    draw_widgets(looks, field, &filled, writer, changed)
}

/// Writes our appearance for each widget of a text or choice field showing
/// `filled`'s value.
fn draw_widgets(
    looks: &Looks<'_, '_, '_>,
    field: &Field,
    filled: &FormField,
    writer: &mut Writer<'_, '_>,
    changed: &mut Changed,
) -> Result<()> {
    let top = field.chain[0]
        .dict
        .get(b"TI")
        .and_then(Object::as_int)
        .and_then(|top| usize::try_from(top).ok())
        .unwrap_or(0);
    for widget in &field.widgets {
        let Some(look) = looks.of(field, widget) else { continue };
        let Some(appearance) = value_appearance(&look, filled, top)? else { continue };
        let stream = writer.write_appearance(&appearance)?;
        let dict = changed.entry(&widget.node)?;
        set(dict, b"AP", Object::Dict(Dict(vec![(b"N".to_vec(), Object::Ref(stream, 0))])));
    }
    Ok(())
}

/// The appearance of a text, choice, or push button widget drawn as `look`
/// showing `filled`'s value (a push button: its caption). `top`: a list's
/// first row shown. None for buttons with states, which have two.
fn value_appearance(look: &Look, filled: &FormField, top: usize) -> Result<Option<Appearance>> {
    Ok(Some(match filled.kind {
        FieldKind::Text => {
            let text = filled.values.first().map(String::as_str).unwrap_or("");
            let comb = filled.comb.then_some(filled.max_len).flatten().filter(|n| *n > 0);
            look.text(text, filled.multiline, comb, filled.password)?
        }
        FieldKind::Combo => {
            let value = filled.values.first().map(String::as_str).unwrap_or("");
            let label = filled
                .options
                .iter()
                .find(|option| option.value == value)
                .map_or(value, |option| option.label.as_str());
            look.text(label, false, None, false)?
        }
        FieldKind::List => {
            let labels: Vec<&str> = filled.options.iter().map(|option| option.label.as_str()).collect();
            let selected: Vec<usize> = filled
                .options
                .iter()
                .enumerate()
                .filter(|(_, option)| filled.values.contains(&option.value))
                .map(|(i, _)| i)
                .collect();
            look.list(&labels, &selected, top)?
        }
        FieldKind::PushButton => look.caption(&filled.caption)?,
        FieldKind::Checkbox | FieldKind::Radio | FieldKind::Signature => return Ok(None),
    }))
}

/// Draws a check box's or radio button's on and off appearances.
fn draw_states(
    looks: &Looks<'_, '_, '_>,
    field: &Field,
    widget: &Widget,
    writer: &mut Writer<'_, '_>,
    changed: &mut Changed,
) -> Result<()> {
    let Some(on) = &widget.on else { return Ok(()) };
    let Some(look) = looks.of(field, widget) else { return Ok(()) };
    let style = if field.info.kind == FieldKind::Radio { ButtonStyle::Radio } else { ButtonStyle::Check };
    let (on_look, off_look) = look.button(style);
    let on_stream = writer.write_appearance(&on_look)?;
    let off_stream = writer.write_appearance(&off_look)?;
    let dict = changed.entry(&widget.node)?;
    let states =
        Dict(vec![(on.clone(), Object::Ref(on_stream, 0)), (b"Off".to_vec(), Object::Ref(off_stream, 0))]);
    set(dict, b"AP", Object::Dict(Dict(vec![(b"N".to_vec(), Object::Dict(states))])));
    Ok(())
}

/// Gives a field our appearance for its current value (a document that
/// asked viewers to draw its fields).
fn refresh(
    looks: &Looks<'_, '_, '_>,
    field: &Field,
    writer: &mut Writer<'_, '_>,
    changed: &mut Changed,
) -> Result<()> {
    match field.info.kind {
        FieldKind::Text | FieldKind::Combo | FieldKind::List => {
            if field.info.values.iter().all(String::is_empty) {
                return Ok(());
            }
            draw_widgets(looks, field, &field.info, writer, changed)
        }
        FieldKind::Checkbox | FieldKind::Radio => {
            for widget in field.widgets.iter().filter(|widget| widget.needs_states) {
                draw_states(looks, field, widget, writer, changed)?;
            }
            Ok(())
        }
        FieldKind::PushButton | FieldKind::Signature => Ok(()),
    }
}

impl Reader<'_, '_> {
    fn dict(&self, object: &Object) -> Option<Dict> {
        resolve_dict(self.file, object)
    }

    /// A string as stored by object `owner`, decrypted.
    fn string(&self, object: &Object, owner: Option<(u32, u16)>) -> Option<Vec<u8>> {
        let Object::Str(bytes) = object else { return None };
        match (self.crypt, owner) {
            (Some(crypt), Some((number, generation))) => crypt.decrypt_string(number, generation, bytes),
            _ => Some(bytes.clone()),
        }
    }

    fn text(&self, object: &Object, owner: Option<(u32, u16)>) -> Option<String> {
        match self.file.resolve(object)? {
            Object::Name(name) => Some(String::from_utf8_lossy(&name).into_owned()),
            resolved => self.string(&resolved, owner).map(|bytes| decode_text(&bytes)),
        }
    }

    /// An entry of the field or the nearest ancestor that has it, resolved,
    /// with the object its strings belong to.
    fn inherited(&self, chain: &[Node], key: &[u8]) -> Option<(Object, Option<(u32, u16)>)> {
        chain.iter().find_map(|node| {
            let value = node.dict.get(key)?;
            let owner = match value {
                Object::Ref(number, generation) => Some((*number, *generation)),
                _ => node.owner,
            };
            Some((self.file.resolve(value)?, owner))
        })
    }

    fn form(&self, tree: &PageTree) -> Option<Form> {
        let root_ref = self.file.trailer.get(b"Root")?;
        let Object::Ref(root_number, root_generation) = *root_ref else { return None };
        let catalog = self.dict(root_ref)?;
        let catalog_node = Node {
            id: Some((root_number, root_generation)),
            owner: Some((root_number, root_generation)),
            dict: catalog.clone(),
        };
        let (acro, catalog) = match catalog.get(b"AcroForm")? {
            Object::Ref(number, generation) => {
                let id = Some((*number, *generation));
                (Node { id, owner: id, dict: self.dict(&Object::Ref(*number, *generation))? }, None)
            }
            Object::Dict(dict) => {
                (Node { id: None, owner: catalog_node.owner, dict: dict.clone() }, Some(catalog_node))
            }
            _ => return None,
        };
        let need_appearances = matches!(
            acro.dict.get(b"NeedAppearances").and_then(|v| self.file.resolve(v)),
            Some(Object::Bool(true))
        );
        let xfa = acro.dict.get(b"XFA").is_some();

        // Which page each annotation is on, and where in its `/Annots`.
        let mut pages = HashMap::new();
        for (index, leaf) in tree.leaves.iter().enumerate() {
            if let Ok(annots) = Annots::read(self.file, leaf) {
                for (position, item) in annots.items.iter().enumerate() {
                    if let Object::Ref(number, _) = item {
                        pages.entry(*number).or_insert((index, position));
                    }
                }
            }
        }

        let mut fields = Vec::new();
        let mut seen = HashSet::new();
        if let Some(Object::Array(items)) = acro.dict.get(b"Fields").and_then(|f| self.file.resolve(f)) {
            for item in &items {
                self.walk(&acro, item, acro.owner, &[], 0, &pages, &mut seen, &mut fields);
            }
        }
        Some(Form { acro, catalog, fields, need_appearances, xfa })
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(
        &self,
        acro: &Node,
        item: &Object,
        owner: Option<(u32, u16)>,
        parents: &[Node],
        depth: usize,
        pages: &HashMap<u32, (usize, usize)>,
        seen: &mut HashSet<u32>,
        out: &mut Vec<Field>,
    ) {
        if depth > MAX_FIELD_DEPTH || out.len() >= MAX_FIELDS {
            return;
        }
        let id = match item {
            Object::Ref(number, generation) => {
                if !seen.insert(*number) {
                    return;
                }
                Some((*number, *generation))
            }
            _ => None,
        };
        let Some(dict) = self.dict(item) else { return };
        let node = Node { id, owner: id.or(owner), dict };
        let mut chain = vec![node.clone()];
        chain.extend(parents.iter().cloned());

        let kids = match node.dict.get(b"Kids").and_then(|kids| self.file.resolve(kids)) {
            Some(Object::Array(kids)) => kids,
            _ => Vec::new(),
        };
        let mut widgets = Vec::new();
        for kid in &kids {
            let Some(kid_dict) = self.dict(kid) else { continue };
            let is_field = kid_dict.get(b"T").is_some()
                || (kid_dict.get(b"Kids").is_some()
                    && kid_dict.get(b"Subtype").and_then(Object::as_name) != Some(b"Widget"));
            if is_field {
                self.walk(acro, kid, node.owner, &chain, depth + 1, pages, seen, out);
            } else {
                let kid_id = match kid {
                    Object::Ref(number, generation) if seen.insert(*number) => Some((*number, *generation)),
                    Object::Ref(..) => continue,
                    _ => None,
                };
                widgets.push(Node { id: kid_id, owner: kid_id.or(node.owner), dict: kid_dict });
            }
        }
        if kids.is_empty()
            && (node.dict.get(b"Subtype").and_then(Object::as_name) == Some(b"Widget")
                || node.dict.get(b"Rect").is_some())
        {
            widgets.push(node.clone());
        }
        if widgets.is_empty() {
            return;
        }
        if let Some(field) = self.field(chain, widgets, pages, acro) {
            out.push(field);
        }
    }

    fn field(
        &self,
        chain: Vec<Node>,
        widget_nodes: Vec<Node>,
        pages: &HashMap<u32, (usize, usize)>,
        acro: &Node,
    ) -> Option<Field> {
        let id = chain[0].id?.0;
        let ft = self.inherited(&chain, b"FT")?.0;
        // macOS PDFKit, writing a radio group again, moves its flags to the
        // buttons: a field without any takes its first widget's.
        let flags = self
            .inherited(&chain, b"Ff")
            .or_else(|| {
                let own = widget_nodes.first().filter(|w| w.id != chain[0].id)?.dict.get(b"Ff")?;
                Some((self.file.resolve(own)?, None))
            })
            .and_then(|(v, _)| v.as_int())
            .unwrap_or(0);
        let kind = match ft.as_name()? {
            b"Tx" => FieldKind::Text,
            b"Ch" if flags & COMBO != 0 => FieldKind::Combo,
            b"Ch" => FieldKind::List,
            b"Btn" if flags & PUSH_BUTTON != 0 => FieldKind::PushButton,
            b"Btn" if flags & RADIO != 0 => FieldKind::Radio,
            b"Btn" => FieldKind::Checkbox,
            b"Sig" => FieldKind::Signature,
            _ => return None,
        };
        let name = chain
            .iter()
            .rev()
            .filter_map(|node| self.text(node.dict.get(b"T")?, node.owner))
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(".");

        // `/Opt`: a choice field's choices, or a button's export values.
        let opt: Vec<FieldOption> = match self.inherited(&chain, b"Opt") {
            Some((Object::Array(items), owner)) => items
                .iter()
                .filter_map(|item| match self.file.resolve(item)? {
                    Object::Array(pair) if pair.len() >= 2 => {
                        let value = self.text(&pair[0], owner)?;
                        let label = self.text(&pair[1], owner)?;
                        Some(FieldOption { value, label })
                    }
                    other => {
                        let text = self.text(&other, owner)?;
                        Some(FieldOption { value: text.clone(), label: text })
                    }
                })
                .collect(),
            _ => Vec::new(),
        };

        let buttons = matches!(kind, FieldKind::Checkbox | FieldKind::Radio);
        let mut widgets = Vec::new();
        for (i, node) in widget_nodes.into_iter().enumerate() {
            let (on, needs_states) = match buttons.then(|| self.on_state(&node.dict)) {
                Some(Some(on)) => (Some(on), false),
                // No states drawn: the export value, or the widget's index (as `/Opt` numbers them).
                Some(None) => {
                    let on = if opt.is_empty() {
                        if kind == FieldKind::Radio { i.to_string() } else { "Yes".into() }
                    } else {
                        i.to_string()
                    };
                    (Some(on.into_bytes()), true)
                }
                None => (None, false),
            };
            widgets.push(Widget { node, on, needs_states });
        }

        let value = self.inherited(&chain, b"V");
        let mut values: Vec<String> = match &value {
            Some((Object::Array(items), owner)) => {
                items.iter().filter_map(|item| self.text(item, *owner)).collect()
            }
            Some((Object::Name(name), _)) => vec![String::from_utf8_lossy(name).into_owned()],
            Some((other @ Object::Str(_), owner)) => {
                self.string(other, *owner).map(|b| decode_text(&b)).into_iter().collect()
            }
            _ => Vec::new(),
        };
        let mut options = opt.clone();
        if buttons {
            let states: Vec<Vec<u8>> = widgets.iter().filter_map(|w| w.on.clone()).collect();
            values.retain(|value| {
                value != "Off" && states.iter().any(|s| String::from_utf8_lossy(s) == value.as_str())
            });
            if values.is_empty() && value.is_none() {
                // No `/V`: the widget shown on.
                values = widgets
                    .iter()
                    .filter_map(|w| {
                        let shown = w.node.dict.get(b"AS")?.as_name()?;
                        (shown != b"Off" && w.on.as_deref() == Some(shown))
                            .then(|| String::from_utf8_lossy(shown).into_owned())
                    })
                    .take(1)
                    .collect();
            }
            let mut unique: Vec<Vec<u8>> = Vec::new();
            for state in states {
                if !unique.contains(&state) {
                    unique.push(state);
                }
            }
            options = unique
                .iter()
                .map(|state| {
                    let value = String::from_utf8_lossy(state).into_owned();
                    // `/Opt` labels widgets by index; their states are often "0", "1", ….
                    let label = value
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| opt.get(i))
                        .map_or_else(|| value.clone(), |option| option.label.clone());
                    FieldOption { value, label }
                })
                .collect();
        }
        let max_len = self
            .inherited(&chain, b"MaxLen")
            .and_then(|(v, _)| v.as_int())
            .and_then(|n| usize::try_from(n).ok());
        let default_values: Vec<String> = match self.inherited(&chain, b"DV") {
            Some((Object::Array(items), owner)) => {
                items.iter().filter_map(|item| self.text(item, owner)).collect()
            }
            Some((Object::Name(name), _)) if name != b"Off" => {
                vec![String::from_utf8_lossy(&name).into_owned()]
            }
            Some((other @ Object::Str(_), owner)) => {
                self.string(&other, owner).map(|b| decode_text(&b)).into_iter().collect()
            }
            _ => Vec::new(),
        };
        let da = self.da(&chain, acro);
        let (font_size, _) = appearance::parse_da(&da);
        let font = appearance::da_font(&da).as_deref().and_then(appearance::family_of);
        let quadding = self
            .inherited(&chain, b"Q")
            .and_then(|(value, _)| value.as_int())
            .or_else(|| acro.dict.get(b"Q").and_then(|q| self.file.resolve(q)?.as_int()))
            .unwrap_or(0);
        let first_mk = widgets.first().and_then(|w| w.node.dict.get(b"MK")).and_then(|mk| self.dict(mk));
        let bordered = first_mk
            .as_ref()
            .and_then(|mk| mk.get(b"BC"))
            .and_then(|bc| self.file.resolve(bc))
            .is_some_and(|bc| bc.as_array().is_some_and(|a| !a.is_empty()));
        let caption = match (kind, first_mk.as_ref().and_then(|mk| mk.get(b"CA")), widgets.first()) {
            (FieldKind::PushButton, Some(ca), Some(widget)) => {
                self.text(ca, widget.node.owner).unwrap_or_default()
            }
            _ => String::new(),
        };
        let own = &chain[0];
        let partial_name = own.dict.get(b"T").and_then(|t| self.text(t, own.owner)).unwrap_or_default();
        let tooltip = own.dict.get(b"TU").and_then(|t| self.text(t, own.owner)).unwrap_or_default();

        let info = FormField {
            id,
            name,
            partial_name,
            kind,
            tooltip,
            default_values,
            align: FieldAlign::of_quadding(quadding),
            font,
            font_size,
            bordered,
            caption,
            values,
            options,
            read_only: flags & READ_ONLY != 0,
            required: flags & REQUIRED != 0,
            multiline: kind == FieldKind::Text && flags & MULTILINE != 0,
            password: kind == FieldKind::Text && flags & PASSWORD != 0,
            comb: kind == FieldKind::Text && flags & COMB != 0 && max_len.is_some(),
            max_len: (kind == FieldKind::Text).then_some(max_len).flatten(),
            multi_select: kind == FieldKind::List && flags & MULTI_SELECT != 0,
            editable: kind == FieldKind::Combo && flags & EDIT != 0,
            widgets: widgets
                .iter()
                .map(|widget| {
                    let at = widget.node.id.and_then(|(number, _)| pages.get(&number).copied());
                    FieldWidget {
                        page: at.map(|(page, _)| page),
                        rect: rect_of(self.file, &widget.node.dict).unwrap_or_default(),
                        view: None,
                        on_state: widget.on.as_ref().map(|on| String::from_utf8_lossy(on).into_owned()),
                        annot_index: at.map(|(_, position)| position),
                    }
                })
                .collect(),
        };
        Some(Field { info, chain, widgets })
    }

    /// A field's default appearance string: its own or an ancestor's, else
    /// the form's.
    fn da(&self, chain: &[Node], acro: &Node) -> Vec<u8> {
        self.inherited(chain, b"DA")
            .and_then(|(value, owner)| self.string(&value, owner))
            .or_else(|| {
                let value = acro.dict.get(b"DA")?;
                self.string(&self.file.resolve(value)?, acro.owner)
            })
            .unwrap_or_else(|| b"/Helv 0 Tf 0 g".to_vec())
    }

    /// A button widget's "on" state: the name of its normal appearance
    /// other than `Off`.
    fn on_state(&self, widget: &Dict) -> Option<Vec<u8>> {
        let ap = self.dict(widget.get(b"AP")?)?;
        for key in [&b"N"[..], b"D"] {
            let Some(value) = ap.get(key) else { continue };
            let states = match value {
                Object::Dict(states) => states.clone(),
                reference => match self.dict(reference) {
                    // A form XObject is one appearance, not a set of states.
                    Some(dict) if dict.get(b"BBox").is_none() && dict.get(b"Subtype").is_none() => dict,
                    _ => continue,
                },
            };
            if let Some((name, _)) = states.0.iter().find(|(name, _)| name != b"Off") {
                return Some(name.clone());
            }
        }
        None
    }
}

/// A PDF text string: UTF-16BE or UTF-8 with a byte-order mark, else
/// PDFDocEncoding.
pub(crate) fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|pair| u16::from_be_bytes(*pair)).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    // PDFDocEncoding: Latin-1, but for 0x18–0x1F and 0x80–0xA0.
    const HIGH: [char; 33] = [
        '•', '†', '‡', '…', '—', '–', 'ƒ', '⁄', '‹', '›', '−', '‰', '„', '“', '”', '‘', '’', '‚', '™', 'ﬁ',
        'ﬂ', 'Ł', 'Œ', 'Š', 'Ÿ', 'Ž', 'ı', 'ł', 'œ', 'š', 'ž', '\u{FFFD}', '€',
    ];
    const LOW: [char; 8] = ['˘', 'ˇ', 'ˆ', '˙', '˝', '˛', '˚', '˜'];
    bytes
        .iter()
        .map(|&byte| match byte {
            0x18..=0x1F => LOW[usize::from(byte - 0x18)],
            0x80..=0xA0 => HIGH[usize::from(byte - 0x80)],
            _ => char::from(byte),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_strings_decode_in_every_encoding() {
        assert_eq!(decode_text(b"Name"), "Name");
        assert_eq!(decode_text(&[0xFE, 0xFF, 0x06, 0x45, 0x06, 0x31]), "مر");
        assert_eq!(decode_text(&[0xEF, 0xBB, 0xBF, 0xD9, 0x85]), "م");
        assert_eq!(decode_text(&[0x8D, 0x41, 0x8E, 0xE9]), "“A”é");
    }
}
