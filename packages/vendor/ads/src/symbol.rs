//! Wrappers for symbol operations and symbol handles.

use std::collections::HashMap;
use std::convert::TryInto;
use std::io::Read;

use byteorder::{ByteOrder, LE, ReadBytesExt};
use zerocopy::{AsBytes, FromBytes};

use crate::errors::{Error, ErrContext};
use crate::index;
use crate::{Device, Result};

/// A handle to a variable within the ADS device.
///
/// The handle is released automatically on drop.
pub struct Handle<'c> {
    device: Device<'c>,
    handle: u32,
}

impl<'c> Handle<'c> {
    /// Create a new handle to a single symbol.
    pub fn new(device: Device<'c>, symbol: &str) -> Result<Self> {
        let mut handle_bytes = [0; 4];
        device.write_read_exact(index::GET_SYMHANDLE_BYNAME, 0, symbol.as_bytes(),
                                &mut handle_bytes)?;
        Ok(Self { device, handle: u32::from_le_bytes(handle_bytes) })
    }

    /// Return the raw handle.
    pub fn raw(&self) -> u32 {
        self.handle
    }

    /// Read data from the variable (returned data must match size of buffer).
    pub fn read(&self, buf: &mut [u8]) -> Result<()> {
        self.device.read_exact(index::RW_SYMVAL_BYHANDLE, self.handle, buf)
    }

    /// Write data to the variable.
    pub fn write(&self, buf: &[u8]) -> Result<()> {
        self.device.write(index::RW_SYMVAL_BYHANDLE, self.handle, buf)
    }

    /// Read data of given type.
    ///
    /// Any type that supports `zerocopy::FromBytes` can be read.  You can also
    /// derive that trait on your own structures and read structured data
    /// directly from the symbol.
    ///
    /// Note: to be independent of the host's byte order, use the integer types
    /// defined in `zerocopy::byteorder`.
    pub fn read_value<T: Default + AsBytes + FromBytes>(&self) -> Result<T> {
        self.device.read_value(index::RW_SYMVAL_BYHANDLE, self.handle)
    }

    /// Write data of given type.
    ///
    /// See `read_value` for details.
    pub fn write_value<T: AsBytes>(&self, value: &T) -> Result<()> {
        self.device.write_value(index::RW_SYMVAL_BYHANDLE, self.handle, value)
    }
}

impl<'a> Drop for Handle<'a> {
    fn drop(&mut self) {
        let _ = self.device.write(index::RELEASE_SYMHANDLE, 0,
                                  &self.handle.to_le_bytes());
    }
}

/// Get symbol size by name.
pub fn get_size(device: Device<'_>, symbol: &str) -> Result<usize> {
    let mut buf = [0; 12];
    device.write_read_exact(index::GET_SYMINFO_BYNAME, 0, symbol.as_bytes(), &mut buf)?;
    Ok(u32::from_le_bytes(buf[8..].try_into().expect("size")) as usize)
}

/// Get symbol location (index group and index offset) by name.
pub fn get_location(device: Device<'_>, symbol: &str) -> Result<(u32, u32)> {
    let mut buf = [0; 12];
    device.write_read_exact(index::GET_SYMINFO_BYNAME, 0, symbol.as_bytes(), &mut buf)?;
    Ok((u32::from_le_bytes(buf[0..4].try_into().expect("size")),
        u32::from_le_bytes(buf[4..8].try_into().expect("size"))))
}


/// Represents a symbol in the PLC memory.
pub struct Symbol {
    /// Hierarchical name of the symbol.
    pub name:      String,
    /// Index group of the symbol location.
    pub ix_group:  u32,
    /// Index offset of the symbol location.
    pub ix_offset: u32,
    /// Type name of the symbol.
    pub typ:       String,
    /// Total size of the symbol, in bytes.
    pub size:      usize,
    /// Base type:
    /// - 0 - void
    /// - 2 - INT (i16)
    /// - 3 - DINT (i32)
    /// - 4 - REAL (f32)
    /// - 5 - LREAL (f64)
    /// - 16 - SINT (i8)
    /// - 17 - USINT/BYTE (u8)
    /// - 18 - UINT/WORD (u16)
    /// - 19 - UDINT/DWORD (u32)
    /// - 20 - LINT (i64)
    /// - 21 - ULINT/LWORD (u64)
    /// - 30 - STRING
    /// - 31 - WSTRING
    /// - 32 - REAL80 (f80)
    /// - 33 - BOOL (u1)
    /// - 65 - Other/Compound type
    pub base_type: u32,
    /// Symbol flags:
    /// - 0x01 - Persistent
    /// - 0x02 - Bit value
    /// - 0x04 - Reference to
    /// - 0x08 - Type GUID present
    /// - 0x10 - TComInterfacePtr
    /// - 0x20 - Read only
    /// - 0x40 - ITF method access
    /// - 0x80 - Method deref
    /// - 0x0F00 - Context mask
    /// - 0x1000 - Attributes present
    /// - 0x2000 - Static
    /// - 0x4000 - Init on reset
    /// - 0x8000 - Extended flags present
    pub flags:     u32,
}

/// Represents a type in the PLC's type inventory.
pub struct Type {
    /// Name of the type.
    pub name:      String,
    /// Total size of the type, in bytes.
    pub size:      usize,
    /// If the type is an array, (lower, upper) index bounds for all dimensions.
    pub array:     Vec<(i32, i32)>,
    /// If the type is a struct, all fields it contains.
    pub fields:    Vec<Field>,
    /// Base type (see [`Symbol::base_type`]).
    pub base_type: u32,
    /// Type flags:
    /// - 0x01 - Data type
    /// - 0x02 - Data item
    /// - 0x04 - Reference to
    /// - 0x08 - Method deref
    /// - 0x10 - Oversample
    /// - 0x20 - Bit values
    /// - 0x40 - Prop item
    /// - 0x80 - Type GUID present
    /// - 0x0100 - Persistent
    /// - 0x0200 - Copy mask
    /// - 0x0400 - TComInterfacePtr
    /// - 0x0800 - Method infos present
    /// - 0x1000 - Attributes present
    /// - 0x2000 - Enum infos present
    /// - 0x010000 - Aligned
    /// - 0x020000 - Static
    /// - 0x040000 - Contains/Has Sp levels present
    /// - 0x080000 - Ignore persistent data
    /// - 0x100000 - Any size array
    /// - 0x200000 - Persistent datatype
    /// - 0x400000 - Init on reset
    /// - 0x800000 - Is/Contains PLC pointer type
    /// - 0x01000000 - Refactor infos present
    pub flags:     u32,
}

/// Represents a field of a structure type.
pub struct Field {
    /// Name of the field.
    pub name:      String,
    /// Type name of the field.
    pub typ:       String,
    /// Offset of the field in the structure.  If `None`, the field is defined
    /// in some other memory block and not inline to the structure.
    pub offset:    Option<u32>,
    /// Size of the field, in bytes.
    pub size:      usize,
    /// If the field is an array, (lower, upper) index bounds for all dimensions.
    pub array:     Vec<(i32, i32)>,
    /// Base type (see [`Symbol::base_type`]).
    pub base_type: u32,
    /// Type flags (see [`Type::flags`]).
    pub flags:     u32,
}

/// A mapping from type name to type.
pub type TypeMap = HashMap<String, Type>;

/// Get and decode symbol and type information from the PLC.
pub fn get_symbol_info(device: Device<'_>) -> Result<(Vec<Symbol>, TypeMap)> {
    // Query the sizes of symbol and type info.
    let mut read_data = [0; 64];
    device.read_exact(index::SYM_UPLOAD_INFO2, 0, &mut read_data)?;
    let symbol_len = LE::read_u32(&read_data[4..]) as usize;
    let types_len  = LE::read_u32(&read_data[12..]) as usize;

    // Query the type info.
    let mut type_data = vec![0; types_len];
    device.read_exact(index::SYM_DT_UPLOAD, 0, &mut type_data)?;

    // Query the symbol info.
    let mut symbol_data = vec![0; symbol_len];
    device.read_exact(index::SYM_UPLOAD, 0, &mut symbol_data)?;

    decode_symbol_info(symbol_data, type_data)
}

/// Decode symbol and type information from the PLC.
///
/// The data must come from the `SYM_UPLOAD` and `SYM_DT_UPLOAD` queries,
/// respectively.
///
/// Returns a list of symbols, and a map of type names to types.
pub fn decode_symbol_info(symbol_data: Vec<u8>, type_data: Vec<u8>) -> Result<(Vec<Symbol>, TypeMap)> {
    // Decode the type info.
    let mut buf = [0; 1024];
    let mut data_ptr = type_data.as_slice();
    let mut type_map = HashMap::new();

    fn decode_type_info(mut ptr: &[u8], parent: Option<&mut Type>) -> Result<Option<Type>> {
        let ctx = "decoding type info";

        let mut buf = [0; 1024];
        let version = ptr.read_u32::<LE>().ctx(ctx)?;
        if version != 1 {
            return Err(Error::Reply(ctx, "unknown type info version", version));
        }
        let _subitem_index = ptr.read_u16::<LE>().ctx(ctx)?;
        let _plc_interface_id = ptr.read_u16::<LE>().ctx(ctx)?;
        let _reserved = ptr.read_u32::<LE>().ctx(ctx)?;
        let size = ptr.read_u32::<LE>().ctx(ctx)? as usize;
        let offset = ptr.read_u32::<LE>().ctx(ctx)?;
        let base_type = ptr.read_u32::<LE>().ctx(ctx)?;
        let flags = ptr.read_u32::<LE>().ctx(ctx)?;
        let len_name = ptr.read_u16::<LE>().ctx(ctx)? as usize;
        let len_type = ptr.read_u16::<LE>().ctx(ctx)? as usize;
        let len_comment = ptr.read_u16::<LE>().ctx(ctx)? as usize;
        let array_dim = ptr.read_u16::<LE>().ctx(ctx)?;
        let sub_items = ptr.read_u16::<LE>().ctx(ctx)?;
        ptr.read_exact(&mut buf[..len_name + 1]).ctx(ctx)?;
        let name = String::from_utf8_lossy(&buf[..len_name]).into_owned();
        ptr.read_exact(&mut buf[..len_type + 1]).ctx(ctx)?;
        let typ = String::from_utf8_lossy(&buf[..len_type]).into_owned();
        ptr = &ptr[len_comment + 1..];

        let mut array = vec![];
        for _ in 0..array_dim {
            let lower = ptr.read_i32::<LE>().ctx(ctx)?;
            let total = ptr.read_i32::<LE>().ctx(ctx)?;
            array.push((lower, lower + total - 1));
        }

        if let Some(parent) = parent {
            assert_eq!(sub_items, 0);
            // Offset -1 marks that the field is placed somewhere else in memory
            // (e.g. AT %Mxx).
            let offset = if offset == 0xFFFF_FFFF { None } else { Some(offset) };
            parent.fields.push(Field { name, typ, offset, size, array, base_type, flags });
            Ok(None)
        } else {
            assert_eq!(offset, 0);
            let mut typinfo = Type { name, size, array, base_type, flags, fields: Vec::new() };

            for _ in 0..sub_items {
                let sub_size = ptr.read_u32::<LE>().ctx(ctx)? as usize;
                let (sub_ptr, rest) = ptr.split_at(sub_size - 4);
                decode_type_info(sub_ptr, Some(&mut typinfo))?;
                ptr = rest;
            }

            // following fields (variable length), which we jump over:
            // - type GUID if flags has Type GUID
            // - copy mask of *size* bytes
            // - # of methods and method entries if flags has Method infos
            // - # of attributes and attributes if flags has Attributes
            // - # of enum infos and enum infos if flags has Enum infos
            // - refactor infos if flags has Refactor infos
            // - splevels if flags has SP levels

            Ok(Some(typinfo))
        }
    }

    while !data_ptr.is_empty() {
        let entry_size = data_ptr.read_u32::<LE>()
            .ctx("decoding type info")? as usize;
        let (entry_ptr, rest) = data_ptr.split_at(entry_size - 4);
        let typ = decode_type_info(entry_ptr, None)?.expect("base type");
        type_map.insert(typ.name.clone(), typ);
        data_ptr = rest;
    }

    // Decode the symbol info.
    let mut symbols = Vec::new();
    let mut data_ptr = symbol_data.as_slice();
    let ctx = "decoding symbol info";
    while !data_ptr.is_empty() {
        let entry_size = data_ptr.read_u32::<LE>().ctx(ctx)? as usize;
        let (mut entry_ptr, rest) = data_ptr.split_at(entry_size - 4);
        let ix_group = entry_ptr.read_u32::<LE>().ctx(ctx)?;
        let ix_offset = entry_ptr.read_u32::<LE>().ctx(ctx)?;
        let size = entry_ptr.read_u32::<LE>().ctx(ctx)? as usize;
        let base_type = entry_ptr.read_u32::<LE>().ctx(ctx)?;
        let flags = entry_ptr.read_u16::<LE>().ctx(ctx)? as u32;
        let _legacy_array_dim = entry_ptr.read_u16::<LE>().ctx(ctx)?;
        let len_name = entry_ptr.read_u16::<LE>().ctx(ctx)? as usize;
        let len_type = entry_ptr.read_u16::<LE>().ctx(ctx)? as usize;
        let _len_comment = entry_ptr.read_u16::<LE>().ctx(ctx)? as usize;
        entry_ptr.read_exact(&mut buf[..len_name + 1]).ctx(ctx)?;
        let name = String::from_utf8_lossy(&buf[..len_name]).into_owned();
        entry_ptr.read_exact(&mut buf[..len_type + 1]).ctx(ctx)?;
        let typ = String::from_utf8_lossy(&buf[..len_type]).into_owned();
        // following fields (variable length), which we jump over:
        // - comment with \0
        // - type GUID if flags has Type GUID
        // - # of attributes and attribute entries if flags has Attributes
        // - flags2 if flags has Extended flags
        // - if flags2 has Old names

        symbols.push(Symbol { name, ix_group, ix_offset, typ, size, base_type, flags });

        data_ptr = rest;
    }

    Ok((symbols, type_map))
}
