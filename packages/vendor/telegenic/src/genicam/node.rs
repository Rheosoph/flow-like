//! The GenICam node graph: a flat arena of typed nodes plus the evaluation
//! engine that chases pValue links down to register reads and writes.
//!
//! Little-endian register default, GenICam big-endian bit numbering
//! flipped via
//! `8 * length - bit - 1`, Cachable default WriteAround, and pInvalidator
//! relationships applied on every write.

use std::collections::HashMap;

use crate::error::{GenicamError, GenicamResult};
use crate::genicam::evaluator::{Expr, Value};
use crate::genicam::port::PortIo;

#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct NodeId(pub(crate) u32);

/// A reference to another node, resolved from a name after the build pass.
/// Unresolved (dangling) references fail on access, not at load — vendor
/// XMLs routinely contain broken links in unused corners.
#[derive(Debug, Clone)]
pub(crate) enum Link {
    Name(String),
    Id(NodeId),
}

#[derive(Debug, Clone)]
pub(crate) enum ValueRef {
    None,
    LitInt(i64),
    LitFloat(f64),
    LitStr(String),
    Link(Link),
}

impl ValueRef {
    fn is_none(&self) -> bool {
        matches!(self, ValueRef::None)
    }
}

#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "py", pyo3::pyclass(eq, eq_int, skip_from_py_object))]
pub enum AccessMode {
    RO,
    WO,
    #[default]
    RW,
}

/// Coarse classification of a node for external introspection (e.g. a
/// generic settings UI walking the category tree). Register / converter /
/// plumbing nodes that don't map to a typed feature report [`NodeKind::Other`].
#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NodeKind {
    Category,
    Integer,
    Float,
    Boolean,
    String,
    Enumeration,
    Command,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Endianness {
    #[default]
    Little,
    Big,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Sign {
    Signed,
    #[default]
    Unsigned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Cachable {
    NoCache,
    WriteThrough,
    #[default]
    WriteAround,
}

#[derive(Debug, Clone)]
pub(crate) struct RegisterCommon {
    /// Summed to form the register address (literals, pAddress links,
    /// anonymous IntSwissKnife children).
    pub address_terms: Vec<ValueRef>,
    pub length: ValueRef,
    pub access: AccessMode,
    pub cachable: Cachable,
    pub cache: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub(crate) enum RegKind {
    Raw,
    Int {
        sign: Sign,
        endianness: Endianness,
        lsb: Option<u32>,
        msb: Option<u32>,
    },
    Float {
        endianness: Endianness,
    },
    Text,
}

#[derive(Debug, Clone)]
pub(crate) struct FormulaSlot {
    pub src: String,
    pub compiled: Option<Expr>,
}

impl FormulaSlot {
    pub fn new(src: String) -> Self {
        Self {
            src,
            compiled: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Node {
    Category {
        features: Vec<Link>,
    },
    Integer {
        value: ValueRef,
        min: ValueRef,
        max: ValueRef,
        inc: ValueRef,
    },
    Float {
        value: ValueRef,
        min: ValueRef,
        max: ValueRef,
    },
    Boolean {
        value: ValueRef,
        on_value: i64,
        off_value: i64,
    },
    StringFeat {
        value: ValueRef,
    },
    Enumeration {
        value: ValueRef,
        entries: Vec<(String, i64)>,
    },
    Command {
        value: ValueRef,
        command_value: ValueRef,
    },
    Register {
        common: RegisterCommon,
        kind: RegKind,
    },
    Converter {
        value: ValueRef,
        formula_to: FormulaSlot,
        formula_from: FormulaSlot,
        variables: Vec<(String, Link)>,
    },
    SwissKnife {
        formula: FormulaSlot,
        variables: Vec<(String, Link)>,
    },
    Port,
}

pub(crate) struct NodeData {
    pub name: Box<str>,
    pub node: Node,
}

pub struct Genicam {
    pub(crate) nodes: Vec<NodeData>,
    pub(crate) by_name: HashMap<Box<str>, NodeId>,
    /// Writing the key node invalidates the caches of the listed nodes
    /// (reverse of the XML's pInvalidator direction).
    pub(crate) invalidates: HashMap<u32, Vec<NodeId>>,
    visit: Vec<NodeId>,
}

impl std::fmt::Debug for Genicam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Genicam")
            .field("nodes", &self.nodes.len())
            .finish()
    }
}

impl Genicam {
    pub(crate) fn from_parts(
        nodes: Vec<NodeData>,
        by_name: HashMap<Box<str>, NodeId>,
        invalidates: HashMap<u32, Vec<NodeId>>,
    ) -> Self {
        Self {
            nodes,
            by_name,
            invalidates,
            visit: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn lookup(&self, name: &str) -> GenicamResult<NodeId> {
        self.by_name
            .get(name)
            .copied()
            .ok_or_else(|| GenicamError::NotFound(name.to_string()))
    }

    pub fn node_names(&self) -> impl Iterator<Item = &str> {
        self.nodes.iter().map(|n| &*n.name)
    }

    pub fn node_name(&self, id: NodeId) -> &str {
        &self.nodes[id.0 as usize].name
    }

    pub fn kind_of(&self, id: NodeId) -> NodeKind {
        match &self.nodes[id.0 as usize].node {
            Node::Category { .. } => NodeKind::Category,
            Node::Integer { .. } => NodeKind::Integer,
            Node::Float { .. } => NodeKind::Float,
            Node::Boolean { .. } => NodeKind::Boolean,
            Node::StringFeat { .. } => NodeKind::String,
            Node::Enumeration { .. } => NodeKind::Enumeration,
            Node::Command { .. } => NodeKind::Command,
            // Converters and SwissKnives present as float features.
            Node::Converter { .. } | Node::SwissKnife { .. } => NodeKind::Float,
            Node::Register { .. } | Node::Port => NodeKind::Other,
        }
    }

    /// The features a `Category` node lists, in XML order. Empty for
    /// non-category nodes and for dangling links.
    pub fn category_features(&self, id: NodeId) -> Vec<NodeId> {
        match &self.nodes[id.0 as usize].node {
            Node::Category { features } => features
                .iter()
                .filter_map(|link| match link {
                    Link::Id(id) => Some(*id),
                    Link::Name(name) => self.by_name.get(name.as_str()).copied(),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn name_of(&self, id: NodeId) -> &str {
        &self.nodes[id.0 as usize].name
    }

    /// Clear every register cache (e.g. after acquisition state changes).
    pub fn invalidate_caches(&mut self) {
        for data in &mut self.nodes {
            if let Node::Register { common, .. } = &mut data.node {
                common.cache = None;
            }
        }
    }

    fn enter(&mut self, id: NodeId) -> GenicamResult<()> {
        if self.visit.contains(&id) {
            self.visit.clear();
            return Err(GenicamError::Circular(self.name_of(id).to_string()));
        }
        self.visit.push(id);
        Ok(())
    }

    fn leave(&mut self) {
        self.visit.pop();
    }

    fn resolve(&self, link: &Link) -> GenicamResult<NodeId> {
        match link {
            Link::Id(id) => Ok(*id),
            Link::Name(name) => Err(GenicamError::Dangling(name.clone())),
        }
    }

    fn ref_int(&mut self, vr: &ValueRef, port: &dyn PortIo) -> GenicamResult<i64> {
        match vr {
            ValueRef::LitInt(v) => Ok(*v),
            ValueRef::LitFloat(v) => Ok(v.round() as i64),
            ValueRef::Link(link) => {
                let id = self.resolve(link)?;
                self.int_value(id, port)
            }
            ValueRef::None | ValueRef::LitStr(_) => {
                Err(GenicamError::WrongType("<value reference>".into()))
            }
        }
    }

    fn ref_float(&mut self, vr: &ValueRef, port: &dyn PortIo) -> GenicamResult<f64> {
        match vr {
            ValueRef::LitInt(v) => Ok(*v as f64),
            ValueRef::LitFloat(v) => Ok(*v),
            ValueRef::Link(link) => {
                let id = self.resolve(link)?;
                self.float_value(id, port)
            }
            ValueRef::None | ValueRef::LitStr(_) => {
                Err(GenicamError::WrongType("<value reference>".into()))
            }
        }
    }

    pub fn int_value(&mut self, id: NodeId, port: &dyn PortIo) -> GenicamResult<i64> {
        self.enter(id)?;
        let result = self.int_value_inner(id, port);
        self.leave();
        result
    }

    fn int_value_inner(&mut self, id: NodeId, port: &dyn PortIo) -> GenicamResult<i64> {
        let node = self.nodes[id.0 as usize].node.clone();
        match node {
            Node::Integer { value, .. }
            | Node::Boolean { value, .. }
            | Node::Enumeration { value, .. } => self.ref_int(&value, port),
            Node::Register {
                kind:
                    RegKind::Int {
                        sign,
                        endianness,
                        lsb,
                        msb,
                    },
                ..
            } => {
                let bytes = self.register_read(id, port)?;
                extract_int(&bytes, sign, endianness, lsb, msb)
                    .map_err(|e| GenicamError::Formula(self.name_of(id).to_string(), e))
            }
            Node::Converter {
                value,
                mut formula_from,
                variables,
                ..
            } => {
                let v =
                    self.eval_converter(id, &value, &mut formula_from, &variables, None, port)?;
                self.store_compiled_from(id, formula_from);
                Ok(v.as_i64())
            }
            Node::SwissKnife {
                mut formula,
                variables,
                ..
            } => {
                let v = self.eval_formula(id, &mut formula, &variables, &[], port)?;
                self.store_compiled_swissknife(id, formula);
                Ok(v.as_i64())
            }
            Node::Float { value, .. } => Ok(self.ref_float(&value, port)?.round() as i64),
            _ => Err(GenicamError::WrongType(self.name_of(id).to_string())),
        }
    }

    pub fn set_int_value(&mut self, id: NodeId, v: i64, port: &dyn PortIo) -> GenicamResult<()> {
        self.enter(id)?;
        let result = self.set_int_value_inner(id, v, port);
        self.leave();
        result
    }

    fn set_int_value_inner(&mut self, id: NodeId, v: i64, port: &dyn PortIo) -> GenicamResult<()> {
        let node = self.nodes[id.0 as usize].node.clone();
        match node {
            Node::Integer { value, .. }
            | Node::Boolean { value, .. }
            | Node::Enumeration { value, .. }
            | Node::Command { value, .. } => self.set_ref_int(id, &value, v, port),
            Node::Register {
                common,
                kind:
                    RegKind::Int {
                        sign,
                        endianness,
                        lsb,
                        msb,
                    },
            } => {
                let length = self.ref_int(&common.length, port)? as usize;
                let bytes = if lsb.is_some() || msb.is_some() {
                    // Masked field: read-modify-write the whole register.
                    let current = self.register_read(id, port)?;
                    insert_int(&current, v, sign, endianness, lsb, msb)
                        .map_err(|e| GenicamError::Formula(self.name_of(id).to_string(), e))?
                } else {
                    encode_int(v, length, endianness)
                        .map_err(|e| GenicamError::Formula(self.name_of(id).to_string(), e))?
                };
                self.register_write(id, &bytes, port)
            }
            Node::Converter {
                value,
                mut formula_to,
                variables,
                ..
            } => {
                let out = self.eval_to(id, &mut formula_to, &variables, Value::I(v), port)?;
                self.store_compiled_to(id, formula_to);
                self.set_ref_value(id, &value, out, port)
            }
            _ => Err(GenicamError::WrongType(self.name_of(id).to_string())),
        }
    }

    fn set_ref_int(
        &mut self,
        id: NodeId,
        vr: &ValueRef,
        v: i64,
        port: &dyn PortIo,
    ) -> GenicamResult<()> {
        match vr {
            ValueRef::Link(link) => {
                let target = self.resolve(link)?;
                self.set_int_value(target, v, port)
            }
            ValueRef::LitInt(_) => {
                // A literal value is a writable in-graph dummy (e.g. the
                // injected TLParamsLocked).
                if let Node::Integer { value, .. }
                | Node::Boolean { value, .. }
                | Node::Enumeration { value, .. }
                | Node::Command { value, .. } = &mut self.nodes[id.0 as usize].node
                {
                    *value = ValueRef::LitInt(v);
                }
                Ok(())
            }
            _ => Err(GenicamError::Access(self.name_of(id).to_string())),
        }
    }

    fn set_ref_value(
        &mut self,
        id: NodeId,
        vr: &ValueRef,
        v: Value,
        port: &dyn PortIo,
    ) -> GenicamResult<()> {
        match vr {
            ValueRef::Link(link) => {
                let target = self.resolve(link)?;
                match (&self.nodes[target.0 as usize].node, v) {
                    (
                        Node::Float { .. }
                        | Node::Register {
                            kind: RegKind::Float { .. },
                            ..
                        },
                        v,
                    ) => self.set_float_value(target, v.as_f64(), port),
                    (_, v) => self.set_int_value(target, v.as_i64(), port),
                }
            }
            _ => Err(GenicamError::Access(self.name_of(id).to_string())),
        }
    }

    pub fn float_value(&mut self, id: NodeId, port: &dyn PortIo) -> GenicamResult<f64> {
        self.enter(id)?;
        let result = self.float_value_inner(id, port);
        self.leave();
        result
    }

    fn float_value_inner(&mut self, id: NodeId, port: &dyn PortIo) -> GenicamResult<f64> {
        let node = self.nodes[id.0 as usize].node.clone();
        match node {
            Node::Float { value, .. } => self.ref_float(&value, port),
            Node::Integer { value, .. } => Ok(self.ref_int(&value, port)? as f64),
            Node::Register {
                kind: RegKind::Float { endianness },
                ..
            } => {
                let bytes = self.register_read(id, port)?;
                extract_float(&bytes, endianness)
                    .map_err(|e| GenicamError::Formula(self.name_of(id).to_string(), e))
            }
            Node::Converter {
                value,
                mut formula_from,
                variables,
                ..
            } => {
                let v =
                    self.eval_converter(id, &value, &mut formula_from, &variables, None, port)?;
                self.store_compiled_from(id, formula_from);
                Ok(v.as_f64())
            }
            Node::SwissKnife {
                mut formula,
                variables,
                ..
            } => {
                let v = self.eval_formula(id, &mut formula, &variables, &[], port)?;
                self.store_compiled_swissknife(id, formula);
                Ok(v.as_f64())
            }
            _ => Err(GenicamError::WrongType(self.name_of(id).to_string())),
        }
    }

    pub fn set_float_value(&mut self, id: NodeId, v: f64, port: &dyn PortIo) -> GenicamResult<()> {
        self.enter(id)?;
        let result = self.set_float_value_inner(id, v, port);
        self.leave();
        result
    }

    fn set_float_value_inner(
        &mut self,
        id: NodeId,
        v: f64,
        port: &dyn PortIo,
    ) -> GenicamResult<()> {
        let node = self.nodes[id.0 as usize].node.clone();
        match node {
            Node::Float { value, .. } => match &value {
                ValueRef::Link(link) => {
                    let target = self.resolve(link)?;
                    self.set_float_value(target, v, port)
                }
                ValueRef::LitFloat(_) => {
                    if let Node::Float { value, .. } = &mut self.nodes[id.0 as usize].node {
                        *value = ValueRef::LitFloat(v);
                    }
                    Ok(())
                }
                _ => Err(GenicamError::Access(self.name_of(id).to_string())),
            },
            Node::Integer { .. } => self.set_int_value_inner(id, v.round() as i64, port),
            Node::Register {
                common,
                kind: RegKind::Float { endianness },
            } => {
                let length = self.ref_int(&common.length, port)? as usize;
                let bytes = encode_float(v, length, endianness)
                    .map_err(|e| GenicamError::Formula(self.name_of(id).to_string(), e))?;
                self.register_write(id, &bytes, port)
            }
            Node::Converter {
                value,
                mut formula_to,
                variables,
                ..
            } => {
                let out = self.eval_to(id, &mut formula_to, &variables, Value::F(v), port)?;
                self.store_compiled_to(id, formula_to);
                self.set_ref_value(id, &value, out, port)
            }
            _ => Err(GenicamError::WrongType(self.name_of(id).to_string())),
        }
    }

    pub fn string_value(&mut self, id: NodeId, port: &dyn PortIo) -> GenicamResult<String> {
        let node = self.nodes[id.0 as usize].node.clone();
        match node {
            Node::StringFeat { value } => match &value {
                ValueRef::LitStr(s) => Ok(s.clone()),
                ValueRef::Link(link) => {
                    let target = self.resolve(link)?;
                    self.string_value(target, port)
                }
                _ => Err(GenicamError::WrongType(self.name_of(id).to_string())),
            },
            Node::Register {
                kind: RegKind::Text,
                ..
            } => {
                let bytes = self.register_read(id, port)?;
                let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
            }
            Node::Enumeration { .. } => {
                let v = self.int_value(id, port)?;
                let Node::Enumeration { entries, .. } = &self.nodes[id.0 as usize].node else {
                    return Err(GenicamError::WrongType(self.name_of(id).to_string()));
                };
                entries
                    .iter()
                    .find(|(_, value)| *value == v)
                    .map(|(name, _)| name.clone())
                    .ok_or_else(|| {
                        GenicamError::NoSuchEntry(
                            self.name_of(id).to_string(),
                            format!("value {v}"),
                        )
                    })
            }
            _ => Err(GenicamError::WrongType(self.name_of(id).to_string())),
        }
    }

    pub fn set_string_value(
        &mut self,
        id: NodeId,
        s: &str,
        port: &dyn PortIo,
    ) -> GenicamResult<()> {
        let node = self.nodes[id.0 as usize].node.clone();
        match node {
            Node::StringFeat { value } => match &value {
                ValueRef::LitStr(_) => {
                    if let Node::StringFeat { value } = &mut self.nodes[id.0 as usize].node {
                        *value = ValueRef::LitStr(s.to_string());
                    }
                    Ok(())
                }
                ValueRef::Link(link) => {
                    let target = self.resolve(link)?;
                    self.set_string_value(target, s, port)
                }
                _ => Err(GenicamError::Access(self.name_of(id).to_string())),
            },
            Node::Register {
                common,
                kind: RegKind::Text,
            } => {
                let length = self.ref_int(&common.length, port)? as usize;
                let mut bytes = vec![0u8; length];
                let n = s.len().min(length);
                bytes[..n].copy_from_slice(&s.as_bytes()[..n]);
                self.register_write(id, &bytes, port)
            }
            Node::Enumeration { .. } => self.set_enum_entry(id, s, port),
            _ => Err(GenicamError::WrongType(self.name_of(id).to_string())),
        }
    }

    pub fn enum_entries(&self, id: NodeId) -> GenicamResult<Vec<String>> {
        match &self.nodes[id.0 as usize].node {
            Node::Enumeration { entries, .. } => {
                Ok(entries.iter().map(|(n, _)| n.clone()).collect())
            }
            _ => Err(GenicamError::WrongType(self.name_of(id).to_string())),
        }
    }

    pub fn set_enum_entry(
        &mut self,
        id: NodeId,
        entry: &str,
        port: &dyn PortIo,
    ) -> GenicamResult<()> {
        let value = match &self.nodes[id.0 as usize].node {
            Node::Enumeration { entries, .. } => entries
                .iter()
                .find(|(name, _)| name == entry)
                .map(|(_, v)| *v)
                .ok_or_else(|| {
                    GenicamError::NoSuchEntry(self.name_of(id).to_string(), entry.to_string())
                })?,
            _ => return Err(GenicamError::WrongType(self.name_of(id).to_string())),
        };
        self.set_int_value(id, value, port)
    }

    pub fn execute(&mut self, id: NodeId, port: &dyn PortIo) -> GenicamResult<()> {
        let node = self.nodes[id.0 as usize].node.clone();
        match node {
            Node::Command {
                value,
                command_value,
            } => {
                let v = if command_value.is_none() {
                    1
                } else {
                    self.ref_int(&command_value, port)?
                };
                self.set_ref_int(id, &value, v, port)
            }
            _ => Err(GenicamError::WrongType(self.name_of(id).to_string())),
        }
    }

    pub fn int_bounds(&mut self, id: NodeId, port: &dyn PortIo) -> GenicamResult<(i64, i64)> {
        let node = self.nodes[id.0 as usize].node.clone();
        match node {
            Node::Integer { min, max, .. } => {
                let lo = if min.is_none() {
                    i64::MIN
                } else {
                    self.ref_int(&min, port)?
                };
                let hi = if max.is_none() {
                    i64::MAX
                } else {
                    self.ref_int(&max, port)?
                };
                Ok((lo, hi))
            }
            Node::Register {
                kind: RegKind::Int { .. },
                ..
            } => Ok((i64::MIN, i64::MAX)),
            _ => Err(GenicamError::WrongType(self.name_of(id).to_string())),
        }
    }

    pub fn int_increment(&mut self, id: NodeId, port: &dyn PortIo) -> GenicamResult<i64> {
        let node = self.nodes[id.0 as usize].node.clone();
        match node {
            Node::Integer { inc, .. } => {
                if inc.is_none() {
                    Ok(1)
                } else {
                    self.ref_int(&inc, port)
                }
            }
            _ => Err(GenicamError::WrongType(self.name_of(id).to_string())),
        }
    }

    pub fn float_bounds(&mut self, id: NodeId, port: &dyn PortIo) -> GenicamResult<(f64, f64)> {
        let node = self.nodes[id.0 as usize].node.clone();
        match node {
            Node::Float { min, max, .. } => {
                let lo = if min.is_none() {
                    f64::MIN
                } else {
                    self.ref_float(&min, port)?
                };
                let hi = if max.is_none() {
                    f64::MAX
                } else {
                    self.ref_float(&max, port)?
                };
                Ok((lo, hi))
            }
            _ => Err(GenicamError::WrongType(self.name_of(id).to_string())),
        }
    }

    pub fn bool_value(&mut self, id: NodeId, port: &dyn PortIo) -> GenicamResult<bool> {
        let node = self.nodes[id.0 as usize].node.clone();
        match node {
            Node::Boolean {
                value, on_value, ..
            } => Ok(self.ref_int(&value, port)? == on_value),
            _ => Ok(self.int_value(id, port)? != 0),
        }
    }

    pub fn set_bool_value(&mut self, id: NodeId, v: bool, port: &dyn PortIo) -> GenicamResult<()> {
        let node = self.nodes[id.0 as usize].node.clone();
        match node {
            Node::Boolean {
                on_value,
                off_value,
                ..
            } => self.set_int_value(id, if v { on_value } else { off_value }, port),
            _ => self.set_int_value(id, i64::from(v), port),
        }
    }

    pub fn access_mode(&self, id: NodeId) -> AccessMode {
        match &self.nodes[id.0 as usize].node {
            Node::Register { common, .. } => common.access,
            Node::SwissKnife { .. } => AccessMode::RO,
            Node::Integer { value, .. }
            | Node::Float { value, .. }
            | Node::Boolean { value, .. }
            | Node::Enumeration { value, .. } => match value {
                ValueRef::Link(Link::Id(id)) => self.access_mode(*id),
                ValueRef::LitInt(_) | ValueRef::LitFloat(_) => AccessMode::RW,
                _ => AccessMode::RO,
            },
            Node::Command { .. } => AccessMode::WO,
            _ => AccessMode::RO,
        }
    }

    fn eval_converter(
        &mut self,
        id: NodeId,
        value: &ValueRef,
        formula: &mut FormulaSlot,
        variables: &[(String, Link)],
        _unused: Option<()>,
        port: &dyn PortIo,
    ) -> GenicamResult<Value> {
        // FROM direction: TO = the raw pValue, result = the user value.
        let raw = match value {
            ValueRef::Link(link) => {
                let target = self.resolve(link)?;
                match &self.nodes[target.0 as usize].node {
                    Node::Float { .. }
                    | Node::Register {
                        kind: RegKind::Float { .. },
                        ..
                    } => Value::F(self.float_value(target, port)?),
                    _ => Value::I(self.int_value(target, port)?),
                }
            }
            other => Value::I(self.ref_int(other, port)?),
        };
        self.eval_formula(id, formula, variables, &[("TO", raw)], port)
    }

    fn eval_to(
        &mut self,
        id: NodeId,
        formula: &mut FormulaSlot,
        variables: &[(String, Link)],
        from: Value,
        port: &dyn PortIo,
    ) -> GenicamResult<Value> {
        self.eval_formula(id, formula, variables, &[("FROM", from)], port)
    }

    fn eval_formula(
        &mut self,
        id: NodeId,
        formula: &mut FormulaSlot,
        variables: &[(String, Link)],
        builtins: &[(&str, Value)],
        port: &dyn PortIo,
    ) -> GenicamResult<Value> {
        if formula.compiled.is_none() {
            let expr = Expr::parse(&formula.src)
                .map_err(|e| GenicamError::Formula(self.name_of(id).to_string(), e))?;
            formula.compiled = Some(expr);
        }
        let Some(expr) = &formula.compiled else {
            unreachable!()
        };
        let expr = expr.clone();

        let mut values = Vec::with_capacity(expr.variables().len());
        for name in expr.variables() {
            if let Some((_, v)) = builtins.iter().find(|(n, _)| n == name) {
                values.push(*v);
                continue;
            }
            let link = variables
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, link)| link.clone())
                // GenICam also allows referencing nodes directly by name.
                .or_else(|| self.by_name.get(name.as_str()).map(|id| Link::Id(*id)))
                .ok_or_else(|| {
                    GenicamError::Formula(
                        self.name_of(id).to_string(),
                        format!("unbound variable '{name}'"),
                    )
                })?;
            let target = self.resolve(&link)?;
            let v = match &self.nodes[target.0 as usize].node {
                Node::Float { .. }
                | Node::Register {
                    kind: RegKind::Float { .. },
                    ..
                } => Value::F(self.float_value(target, port)?),
                _ => Value::I(self.int_value(target, port)?),
            };
            values.push(v);
        }
        expr.eval(&values)
            .map_err(|e| GenicamError::Formula(self.name_of(id).to_string(), e))
    }

    fn store_compiled_from(&mut self, id: NodeId, slot: FormulaSlot) {
        if let Node::Converter { formula_from, .. } = &mut self.nodes[id.0 as usize].node {
            *formula_from = slot;
        }
    }

    fn store_compiled_to(&mut self, id: NodeId, slot: FormulaSlot) {
        if let Node::Converter { formula_to, .. } = &mut self.nodes[id.0 as usize].node {
            *formula_to = slot;
        }
    }

    fn store_compiled_swissknife(&mut self, id: NodeId, slot: FormulaSlot) {
        if let Node::SwissKnife { formula, .. } = &mut self.nodes[id.0 as usize].node {
            *formula = slot;
        }
    }

    fn register_address(&mut self, id: NodeId, port: &dyn PortIo) -> GenicamResult<(u64, usize)> {
        let Node::Register { common, .. } = self.nodes[id.0 as usize].node.clone() else {
            return Err(GenicamError::WrongType(self.name_of(id).to_string()));
        };
        let mut address: i64 = 0;
        for term in &common.address_terms {
            address = address.wrapping_add(self.ref_int(term, port)?);
        }
        let length = if common.length.is_none() {
            4
        } else {
            self.ref_int(&common.length, port)?
        };
        Ok((address as u64, length as usize))
    }

    fn register_read(&mut self, id: NodeId, port: &dyn PortIo) -> GenicamResult<Vec<u8>> {
        {
            let Node::Register { common, .. } = &self.nodes[id.0 as usize].node else {
                return Err(GenicamError::WrongType(self.name_of(id).to_string()));
            };
            if common.access == AccessMode::WO {
                return Err(GenicamError::Access(self.name_of(id).to_string()));
            }
            if common.cachable != Cachable::NoCache
                && let Some(cache) = &common.cache
            {
                return Ok(cache.clone());
            }
        }
        let (address, length) = self.register_address(id, port)?;
        let mut buf = vec![0u8; length];
        port.read(address, &mut buf)?;
        if let Node::Register { common, .. } = &mut self.nodes[id.0 as usize].node
            && common.cachable != Cachable::NoCache
        {
            common.cache = Some(buf.clone());
        }
        Ok(buf)
    }

    fn register_write(&mut self, id: NodeId, bytes: &[u8], port: &dyn PortIo) -> GenicamResult<()> {
        {
            let Node::Register { common, .. } = &self.nodes[id.0 as usize].node else {
                return Err(GenicamError::WrongType(self.name_of(id).to_string()));
            };
            if common.access == AccessMode::RO {
                return Err(GenicamError::Access(self.name_of(id).to_string()));
            }
        }
        let (address, _) = self.register_address(id, port)?;
        port.write(address, bytes)?;
        if let Node::Register { common, .. } = &mut self.nodes[id.0 as usize].node {
            common.cache = match common.cachable {
                Cachable::WriteThrough => Some(bytes.to_vec()),
                Cachable::WriteAround | Cachable::NoCache => None,
            };
        }
        if let Some(dependents) = self.invalidates.get(&id.0).cloned() {
            for dep in dependents {
                if let Node::Register { common, .. } = &mut self.nodes[dep.0 as usize].node {
                    common.cache = None;
                }
            }
        }
        Ok(())
    }
}

/// Load `length` register bytes into a u64 and extract the (optionally
/// masked) integer field. GenICam
/// big-endian bit numbering counts bit 0 as the MSB, so BE registers flip:
/// `bit = 8 * length - bit - 1`.
fn extract_int(
    bytes: &[u8],
    sign: Sign,
    endianness: Endianness,
    register_lsb: Option<u32>,
    register_msb: Option<u32>,
) -> Result<i64, String> {
    let length = bytes.len();
    if length == 0 || length > 8 {
        return Err(format!("unsupported integer register length {length}"));
    }
    let mut value = load_u64(bytes, endianness);

    let bits = 8 * length as u32;
    let (lsb, msb) = match (register_lsb, register_msb, endianness) {
        (None, None, _) => (0, bits - 1),
        (lsb, msb, Endianness::Little) => (lsb.unwrap_or(0), msb.unwrap_or(lsb.unwrap_or(0))),
        (lsb, msb, Endianness::Big) => {
            let l = lsb.or(msb).unwrap_or(0);
            let m = msb.or(lsb).unwrap_or(0);
            (bits - l - 1, bits - m - 1)
        }
    };
    if msb < lsb || msb >= bits {
        return Err(format!("bad bit range {lsb}..{msb} for {length} bytes"));
    }

    let width = msb - lsb + 1;
    let mask = if width >= 64 {
        u64::MAX
    } else {
        ((1u64 << width) - 1) << lsb
    };
    value = (value & mask) >> lsb;
    if sign == Sign::Signed && width < 64 && value & (1u64 << (width - 1)) != 0 {
        value |= u64::MAX ^ (mask >> lsb);
    }
    Ok(value as i64)
}

/// Read-modify-write companion of [`extract_int`].
fn insert_int(
    current: &[u8],
    v: i64,
    _sign: Sign,
    endianness: Endianness,
    register_lsb: Option<u32>,
    register_msb: Option<u32>,
) -> Result<Vec<u8>, String> {
    let length = current.len();
    if length == 0 || length > 8 {
        return Err(format!("unsupported integer register length {length}"));
    }
    let mut value = load_u64(current, endianness);
    let bits = 8 * length as u32;
    let (lsb, msb) = match (register_lsb, register_msb, endianness) {
        (None, None, _) => (0, bits - 1),
        (lsb, msb, Endianness::Little) => (lsb.unwrap_or(0), msb.unwrap_or(lsb.unwrap_or(0))),
        (lsb, msb, Endianness::Big) => {
            let l = lsb.or(msb).unwrap_or(0);
            let m = msb.or(lsb).unwrap_or(0);
            (bits - l - 1, bits - m - 1)
        }
    };
    if msb < lsb || msb >= bits {
        return Err(format!("bad bit range {lsb}..{msb} for {length} bytes"));
    }
    let width = msb - lsb + 1;
    let mask = if width >= 64 {
        u64::MAX
    } else {
        ((1u64 << width) - 1) << lsb
    };
    value = (value & !mask) | (((v as u64) << lsb) & mask);
    Ok(store_u64(value, length, endianness))
}

fn load_u64(bytes: &[u8], endianness: Endianness) -> u64 {
    let mut value = 0u64;
    match endianness {
        Endianness::Big => {
            for &b in bytes {
                value = (value << 8) | u64::from(b);
            }
        }
        Endianness::Little => {
            for &b in bytes.iter().rev() {
                value = (value << 8) | u64::from(b);
            }
        }
    }
    value
}

fn store_u64(value: u64, length: usize, endianness: Endianness) -> Vec<u8> {
    let le = value.to_le_bytes();
    match endianness {
        Endianness::Little => le[..length].to_vec(),
        Endianness::Big => {
            let mut out = le[..length].to_vec();
            out.reverse();
            out
        }
    }
}

fn encode_int(v: i64, length: usize, endianness: Endianness) -> Result<Vec<u8>, String> {
    if length == 0 || length > 8 {
        return Err(format!("unsupported integer register length {length}"));
    }
    Ok(store_u64(v as u64, length, endianness))
}

fn extract_float(bytes: &[u8], endianness: Endianness) -> Result<f64, String> {
    match bytes.len() {
        4 => {
            let raw = load_u64(bytes, endianness) as u32;
            Ok(f64::from(f32::from_bits(raw)))
        }
        8 => Ok(f64::from_bits(load_u64(bytes, endianness))),
        other => Err(format!("unsupported float register length {other}")),
    }
}

fn encode_float(v: f64, length: usize, endianness: Endianness) -> Result<Vec<u8>, String> {
    match length {
        4 => Ok(store_u64(u64::from((v as f32).to_bits()), 4, endianness)),
        8 => Ok(store_u64(v.to_bits(), 8, endianness)),
        other => Err(format!("unsupported float register length {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masked_big_endian_flip() {
        // The bootstrap LLA bit: 4-byte BE register, GenICam Bit 29 ==
        // conventional bit 2.
        let bytes = (1u32 << 2).to_be_bytes();
        let v = extract_int(&bytes, Sign::Unsigned, Endianness::Big, Some(29), Some(29)).unwrap();
        assert_eq!(v, 1);
        let v = extract_int(&bytes, Sign::Unsigned, Endianness::Big, Some(31), Some(16)).unwrap();
        assert_eq!(v, 4); // low 16 bits
    }

    #[test]
    fn masked_little_endian() {
        let bytes = 0xab_cdu16.to_le_bytes();
        let v = extract_int(
            &bytes,
            Sign::Unsigned,
            Endianness::Little,
            Some(8),
            Some(15),
        )
        .unwrap();
        assert_eq!(v, 0xab);
    }

    #[test]
    fn signed_extraction() {
        let bytes = (-2i32).to_be_bytes();
        let v = extract_int(&bytes, Sign::Signed, Endianness::Big, None, None).unwrap();
        assert_eq!(v, -2);
        let v = extract_int(&bytes, Sign::Unsigned, Endianness::Big, None, None).unwrap();
        assert_eq!(v, u64::from(u32::MAX - 1) as i64);
    }

    #[test]
    fn insert_preserves_other_bits() {
        let current = 0xffff_0000u32.to_be_bytes();
        let out = insert_int(
            &current,
            0x12,
            Sign::Unsigned,
            Endianness::Big,
            Some(31),
            Some(16),
        )
        .unwrap();
        assert_eq!(u32::from_be_bytes(out.try_into().unwrap()), 0xffff_0012);
    }

    #[test]
    fn float_roundtrip() {
        let bytes = encode_float(1.5, 4, Endianness::Big).unwrap();
        assert_eq!(extract_float(&bytes, Endianness::Big).unwrap(), 1.5);
        let bytes = encode_float(-0.25, 8, Endianness::Little).unwrap();
        assert_eq!(extract_float(&bytes, Endianness::Little).unwrap(), -0.25);
    }
}
