//! GenICam device-description XML → node arena.
//!
//! Two passes: walk the document creating typed nodes with links held as
//! names, then resolve names to [`NodeId`]s and build the reverse
//! pInvalidator map. Dangling references degrade to error-on-access, not
//! load failure — vendor XMLs routinely have broken corners.

use std::collections::HashMap;

use roxmltree::Node as XmlNode;

use crate::error::{GenicamError, GenicamResult};
use crate::genicam::node::{
    AccessMode, Cachable, Endianness, FormulaSlot, Genicam, Link, Node, NodeData, NodeId, RegKind,
    RegisterCommon, Sign, ValueRef,
};

/// Fallback definitions for standard features some vendor XMLs omit,
/// trimmed to the ones acquisition control relies on.
const DEFAULT_NODES: &[(&str, &str)] = &[
    (
        "TLParamsLocked",
        r#"<Integer Name="TLParamsLocked"><Value>0</Value><Min>0</Min><Max>1</Max></Integer>"#,
    ),
    (
        "GevSCPSPacketSize",
        r#"<MaskedIntReg Name="GevSCPSPacketSize">
             <Address>0xd04</Address><Length>4</Length><AccessMode>RW</AccessMode>
             <pPort>Device</pPort><Cachable>NoCache</Cachable>
             <LSB>31</LSB><MSB>16</MSB><Endianess>BigEndian</Endianess>
           </MaskedIntReg>"#,
    ),
    (
        "GevTimestampTickFrequency",
        r#"<IntReg Name="GevTimestampTickFrequency">
             <Address>0x93c</Address><Length>8</Length><AccessMode>RO</AccessMode>
             <pPort>Device</pPort><Endianess>BigEndian</Endianess>
           </IntReg>"#,
    ),
];

pub fn parse(xml: &str) -> GenicamResult<Genicam> {
    let doc = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map_err(|e| GenicamError::Xml(e.to_string()))?;

    let mut b = Builder::default();
    b.walk(doc.root_element());

    for (name, snippet) in DEFAULT_NODES {
        if !b.by_name.contains_key(*name) {
            let doc = roxmltree::Document::parse(snippet)
                .map_err(|e| GenicamError::Xml(format!("default node {name}: {e}")))?;
            b.add_element(doc.root_element());
        }
    }

    Ok(b.finish())
}

#[derive(Default)]
struct Builder {
    nodes: Vec<NodeData>,
    by_name: HashMap<Box<str>, NodeId>,
    /// (node, names of nodes whose writes invalidate it)
    invalidators: Vec<(NodeId, Vec<String>)>,
    anon: u32,
}

impl Builder {
    fn walk(&mut self, el: XmlNode<'_, '_>) {
        for child in el.children().filter(XmlNode::is_element) {
            match child.tag_name().name() {
                // Groups are transparent containers.
                "Group" => self.walk(child),
                _ => {
                    self.add_element(child);
                }
            }
        }
    }

    fn push(&mut self, name: String, node: Node) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.by_name.insert(name.clone().into_boxed_str(), id);
        self.nodes.push(NodeData {
            name: name.into_boxed_str(),
            node,
        });
        id
    }

    fn add_element(&mut self, el: XmlNode<'_, '_>) -> Option<NodeId> {
        let tag = el.tag_name().name();
        let name = el.attribute("Name").map(str::to_string).unwrap_or_else(|| {
            self.anon += 1;
            format!("__anonymous_{}_{}", tag, self.anon)
        });

        let node = match tag {
            "Category" => Node::Category {
                features: texts(el, "pFeature")
                    .into_iter()
                    .map(|n| Link::Name(n.to_string()))
                    .collect(),
            },
            "Integer" => Node::Integer {
                value: int_value_ref(el, "Value", "pValue"),
                min: int_value_ref(el, "Min", "pMin"),
                max: int_value_ref(el, "Max", "pMax"),
                inc: int_value_ref(el, "Inc", "pInc"),
            },
            "Float" => Node::Float {
                value: float_value_ref(el, "Value", "pValue"),
                min: float_value_ref(el, "Min", "pMin"),
                max: float_value_ref(el, "Max", "pMax"),
            },
            "Boolean" => Node::Boolean {
                value: int_value_ref(el, "Value", "pValue"),
                on_value: int_text(el, "OnValue").unwrap_or(1),
                off_value: int_text(el, "OffValue").unwrap_or(0),
            },
            "String" => Node::StringFeat {
                value: match (text(el, "Value"), text(el, "pValue")) {
                    (_, Some(p)) => ValueRef::Link(Link::Name(p.to_string())),
                    (Some(v), None) => ValueRef::LitStr(v.to_string()),
                    (None, None) => ValueRef::None,
                },
            },
            "Enumeration" => {
                let entries = el
                    .children()
                    .filter(|c| c.is_element() && c.tag_name().name() == "EnumEntry")
                    .filter_map(|entry| {
                        let entry_name = entry.attribute("Name")?.to_string();
                        let value = int_text(entry, "Value")?;
                        Some((entry_name, value))
                    })
                    .collect();
                Node::Enumeration {
                    value: int_value_ref(el, "Value", "pValue"),
                    entries,
                }
            }
            "Command" => Node::Command {
                value: int_value_ref(el, "Value", "pValue"),
                command_value: int_value_ref(el, "CommandValue", "pCommandValue"),
            },
            "Register" => Node::Register {
                common: self.register_common(el),
                kind: RegKind::Raw,
            },
            "IntReg" => Node::Register {
                common: self.register_common(el),
                kind: RegKind::Int {
                    sign: sign(el),
                    endianness: endianness(el),
                    lsb: None,
                    msb: None,
                },
            },
            "MaskedIntReg" => Node::Register {
                common: self.register_common(el),
                kind: masked_int_kind(el),
            },
            "FloatReg" => Node::Register {
                common: self.register_common(el),
                kind: RegKind::Float {
                    endianness: endianness(el),
                },
            },
            "StringReg" => Node::Register {
                common: self.register_common(el),
                kind: RegKind::Text,
            },
            "StructReg" => {
                // Each StructEntry becomes its own masked register sharing
                // the parent's address block.
                let common = self.register_common(el);
                let parent_endianness = endianness(el);
                for entry in el
                    .children()
                    .filter(|c| c.is_element() && c.tag_name().name() == "StructEntry")
                {
                    let Some(entry_name) = entry.attribute("Name") else {
                        continue;
                    };
                    let mut entry_common = common.clone();
                    if let Some(mode) = access_mode(entry) {
                        entry_common.access = mode;
                    }
                    let mut kind = masked_int_kind(entry);
                    if let RegKind::Int {
                        endianness: e,
                        sign: s,
                        ..
                    } = &mut kind
                    {
                        if entry.children().all(|c| c.tag_name().name() != "Endianess") {
                            *e = parent_endianness;
                        }
                        if entry.children().all(|c| c.tag_name().name() != "Sign") {
                            *s = sign(el);
                        }
                    }
                    let id = self.push(
                        entry_name.to_string(),
                        Node::Register {
                            common: entry_common,
                            kind,
                        },
                    );
                    self.collect_invalidators(id, entry);
                }
                return None;
            }
            "Converter" | "IntConverter" => Node::Converter {
                value: int_value_ref(el, "Value", "pValue"),
                formula_to: FormulaSlot::new(text(el, "FormulaTo").unwrap_or_default().to_string()),
                formula_from: FormulaSlot::new(
                    text(el, "FormulaFrom").unwrap_or_default().to_string(),
                ),
                variables: variables(el),
            },
            "SwissKnife" | "IntSwissKnife" => Node::SwissKnife {
                formula: FormulaSlot::new(text(el, "Formula").unwrap_or_default().to_string()),
                variables: variables(el),
            },
            "Port" => Node::Port,
            // StructEntry handled by StructReg; everything else ignored.
            _ => return None,
        };

        let id = self.push(name, node);
        self.collect_invalidators(id, el);
        Some(id)
    }

    fn collect_invalidators(&mut self, id: NodeId, el: XmlNode<'_, '_>) {
        let names: Vec<String> = texts(el, "pInvalidator")
            .into_iter()
            .map(str::to_string)
            .collect();
        if !names.is_empty() {
            self.invalidators.push((id, names));
        }
    }

    fn register_common(&mut self, el: XmlNode<'_, '_>) -> RegisterCommon {
        let mut address_terms = Vec::new();
        for child in el.children().filter(XmlNode::is_element) {
            match child.tag_name().name() {
                "Address" => {
                    if let Some(v) = child.text().and_then(|t| parse_i64(t.trim())) {
                        address_terms.push(ValueRef::LitInt(v));
                    }
                }
                "pAddress" => {
                    if let Some(t) = child.text() {
                        address_terms.push(ValueRef::Link(Link::Name(t.trim().to_string())));
                    }
                }
                // An inline anonymous address computation.
                "IntSwissKnife" => {
                    if let Some(id) = self.add_element(child) {
                        address_terms.push(ValueRef::Link(Link::Id(id)));
                    }
                }
                _ => {}
            }
        }
        RegisterCommon {
            address_terms,
            length: int_value_ref(el, "Length", "pLength"),
            access: access_mode(el).unwrap_or_default(),
            cachable: cachable(el),
            cache: None,
        }
    }

    fn finish(self) -> Genicam {
        let Builder {
            mut nodes,
            by_name,
            invalidators,
            ..
        } = self;

        let resolve = |link: &mut Link| {
            if let Link::Name(name) = link
                && let Some(id) = by_name.get(name.as_str())
            {
                *link = Link::Id(*id);
            }
        };
        let resolve_ref = |vr: &mut ValueRef| {
            if let ValueRef::Link(link) = vr {
                resolve(link);
            }
        };

        for data in &mut nodes {
            match &mut data.node {
                Node::Category { features } => features.iter_mut().for_each(resolve),
                Node::Integer {
                    value,
                    min,
                    max,
                    inc,
                } => {
                    [value, min, max, inc].into_iter().for_each(resolve_ref);
                }
                Node::Float { value, min, max } => {
                    [value, min, max].into_iter().for_each(resolve_ref);
                }
                Node::Boolean { value, .. }
                | Node::StringFeat { value }
                | Node::Enumeration { value, .. } => resolve_ref(value),
                Node::Command {
                    value,
                    command_value,
                } => {
                    [value, command_value].into_iter().for_each(resolve_ref);
                }
                Node::Register { common, .. } => {
                    common.address_terms.iter_mut().for_each(resolve_ref);
                    resolve_ref(&mut common.length);
                }
                Node::Converter {
                    value, variables, ..
                } => {
                    resolve_ref(value);
                    variables.iter_mut().for_each(|(_, link)| resolve(link));
                }
                Node::SwissKnife { variables, .. } => {
                    variables.iter_mut().for_each(|(_, link)| resolve(link));
                }
                Node::Port => {}
            }
        }

        let mut invalidates: HashMap<u32, Vec<NodeId>> = HashMap::new();
        for (node, names) in invalidators {
            for name in names {
                if let Some(source) = by_name.get(name.as_str()) {
                    invalidates.entry(source.0).or_default().push(node);
                }
            }
        }

        Genicam::from_parts(nodes, by_name, invalidates)
    }
}

fn text<'a>(el: XmlNode<'a, '_>, tag: &str) -> Option<&'a str> {
    el.children()
        .find(|c| c.is_element() && c.tag_name().name() == tag)
        .and_then(|c| c.text())
        .map(str::trim)
}

fn texts<'a>(el: XmlNode<'a, '_>, tag: &str) -> Vec<&'a str> {
    el.children()
        .filter(|c| c.is_element() && c.tag_name().name() == tag)
        .filter_map(|c| c.text())
        .map(str::trim)
        .collect()
}

/// GenICam integers: decimal or `0x` hex (signed decimal allowed).
fn parse_i64(s: &str) -> Option<i64> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok().map(|v| v as i64)
    } else {
        s.parse().ok()
    }
}

fn int_text(el: XmlNode<'_, '_>, tag: &str) -> Option<i64> {
    text(el, tag).and_then(parse_i64)
}

fn int_value_ref(el: XmlNode<'_, '_>, literal: &str, pointer: &str) -> ValueRef {
    if let Some(p) = text(el, pointer) {
        return ValueRef::Link(Link::Name(p.to_string()));
    }
    match int_text(el, literal) {
        Some(v) => ValueRef::LitInt(v),
        None => ValueRef::None,
    }
}

fn float_value_ref(el: XmlNode<'_, '_>, literal: &str, pointer: &str) -> ValueRef {
    if let Some(p) = text(el, pointer) {
        return ValueRef::Link(Link::Name(p.to_string()));
    }
    match text(el, literal).and_then(|t| t.parse::<f64>().ok()) {
        Some(v) => ValueRef::LitFloat(v),
        None => match int_text(el, literal) {
            Some(v) => ValueRef::LitInt(v),
            None => ValueRef::None,
        },
    }
}

fn variables(el: XmlNode<'_, '_>) -> Vec<(String, Link)> {
    el.children()
        .filter(|c| c.is_element() && c.tag_name().name() == "pVariable")
        .filter_map(|c| {
            let var = c.attribute("Name")?.to_string();
            let target = c.text()?.trim().to_string();
            Some((var, Link::Name(target)))
        })
        .collect()
}

fn access_mode(el: XmlNode<'_, '_>) -> Option<AccessMode> {
    match text(el, "AccessMode")? {
        "RO" => Some(AccessMode::RO),
        "WO" => Some(AccessMode::WO),
        "RW" => Some(AccessMode::RW),
        _ => None,
    }
}

fn endianness(el: XmlNode<'_, '_>) -> Endianness {
    match text(el, "Endianess") {
        Some("BigEndian") => Endianness::Big,
        _ => Endianness::Little,
    }
}

fn sign(el: XmlNode<'_, '_>) -> Sign {
    match text(el, "Sign") {
        Some("Signed") => Sign::Signed,
        _ => Sign::Unsigned,
    }
}

fn cachable(el: XmlNode<'_, '_>) -> Cachable {
    match text(el, "Cachable") {
        Some("NoCache") => Cachable::NoCache,
        Some("WriteThrough") => Cachable::WriteThrough,
        Some("WriteAround") => Cachable::WriteAround,
        _ => Cachable::default(),
    }
}

fn masked_int_kind(el: XmlNode<'_, '_>) -> RegKind {
    let bit = int_text(el, "Bit").map(|v| v as u32);
    let lsb = int_text(el, "LSB").map(|v| v as u32).or(bit);
    let msb = int_text(el, "MSB").map(|v| v as u32).or(bit);
    RegKind::Int {
        sign: sign(el),
        endianness: endianness(el),
        lsb,
        msb,
    }
}
