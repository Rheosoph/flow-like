//! Const-generic string types for representing fixed-length strings.

use zerocopy::{FromBytes, FromZeroes};

/// Represents a fixed-length byte string.
///
/// This type can be created from a `&str` or `&[u8]` if their byte
/// length does not exceed the fixed length.
///
/// It can be freely converted from and to a `[u8; N]` array, and
/// to a `Vec<u8>` where it will be cut at the first null byte.
///
/// It can be converted to a Rust `String` if it is UTF8 encoded.
#[repr(C)]
#[derive(Clone, Copy, FromBytes, FromZeroes)]
pub struct String<const LEN: usize>([u8; LEN], u8);  // one extra NULL byte

impl<const LEN: usize> String<LEN> {
    /// Create a new empty string.
    pub fn new() -> Self {
        Self([0; LEN], 0)
    }

    /// Return the number of bytes up to the first null byte.
    pub fn len(&self) -> usize {
        self.0.iter().position(|&b| b == 0).unwrap_or(self.0.len())
    }

    /// Returns true if the string is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the slice up to the first null byte.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0[..self.len()]
    }

    /// Get a reference to the full array of bytes.
    pub fn backing_array(&mut self) -> &mut [u8; LEN] {
        &mut self.0
    }
}

// standard traits

impl<const LEN: usize> std::default::Default for String<LEN> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const LEN: usize> std::fmt::Debug for String<LEN> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::fmt::Debug::fmt(&std::string::String::from_utf8_lossy(self.as_bytes()), fmt)
    }
}

impl<const LEN: usize> std::cmp::PartialEq for String<LEN> {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl<const LEN: usize> std::cmp::PartialEq<&[u8]> for String<LEN> {
    fn eq(&self, other: &&[u8]) -> bool {
        self.as_bytes() == *other
    }
}

impl<const LEN: usize> std::cmp::PartialEq<&str> for String<LEN> {
    fn eq(&self, other: &&str) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

// conversion with [u8; N]

impl<const LEN: usize> std::convert::From<[u8; LEN]> for String<LEN> {
    fn from(arr: [u8; LEN]) -> Self {
        Self(arr, 0)
    }
}

impl<const LEN: usize> std::convert::From<String<LEN>> for [u8; LEN] {
    fn from(bstr: String<LEN>) -> Self {
        bstr.0
    }
}

// conversion with bytes

impl<const LEN: usize> std::convert::TryFrom<&'_ [u8]> for String<LEN> {
    type Error = ();
    fn try_from(arr: &[u8]) -> std::result::Result<Self, ()> {
        if arr.len() > LEN {
            return Err(());
        }
        let mut bstr = Self::new();
        bstr.0[..arr.len()].copy_from_slice(arr);
        Ok(bstr)
    }
}

impl<const LEN: usize> std::convert::From<String<LEN>> for std::vec::Vec<u8> {
    fn from(bstr: String<LEN>) -> Self {
        bstr.as_bytes().to_vec()
    }
}

// conversion with strings

impl<const LEN: usize> std::convert::TryFrom<&'_ str> for String<LEN> {
    type Error = ();
    fn try_from(s: &str) -> std::result::Result<Self, ()> {
        Self::try_from(s.as_bytes())
    }
}

impl<const LEN: usize> std::convert::TryFrom<String<LEN>> for std::string::String {
    type Error = std::str::Utf8Error;
    fn try_from(bstr: String<LEN>) -> std::result::Result<Self, Self::Error> {
        std::str::from_utf8(bstr.as_bytes()).map(Into::into)
    }
}

// zerocopy implementations

unsafe impl<const LEN: usize> zerocopy::AsBytes for String<LEN> {
    fn only_derive_is_allowed_to_implement_this_trait() { }
}

/// Represents a fixed-length wide string.
///
/// This type can be created from a `&[u16]` if its length does not
/// exceed the fixed length.  It can be created from a `&str` if its
/// length, encoded in UTF16, does not exceed the fixed length.
///
/// It can be freely converted from and to a `[u16; N]` array, and
/// to a `Vec<u16>` where it will be cut at the first null.
///
/// It can be converted to a Rust `String` if it is properly UTF16
/// encoded.
#[repr(C)]
#[derive(Clone, Copy, FromBytes, FromZeroes)]
pub struct WString<const LEN: usize>([u16; LEN], u16);  // one extra NULL byte

impl<const LEN: usize> WString<LEN> {
    /// Create a new empty string.
    pub fn new() -> Self {
        Self([0; LEN], 0)
    }

    /// Return the number of code units up to the first null.
    pub fn len(&self) -> usize {
        self.0.iter().position(|&b| b == 0).unwrap_or(self.0.len())
    }

    /// Returns true if the string is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the slice up to the first null code unit.
    pub fn as_slice(&self) -> &[u16] {
        &self.0[..self.len()]
    }

    /// Get a reference to the full array of code units.
    pub fn backing_array(&mut self) -> &mut [u16; LEN] {
        &mut self.0
    }
}

impl<const LEN: usize> std::default::Default for WString<LEN> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const LEN: usize> std::fmt::Debug for WString<LEN> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let fmted: std::string::String =
            std::char::decode_utf16(self.0.iter().cloned().take_while(|&b| b != 0))
            .map(|ch| ch.unwrap_or(std::char::REPLACEMENT_CHARACTER))
            .collect();
        std::fmt::Debug::fmt(&fmted, fmt)
    }
}

impl<const LEN: usize> std::cmp::PartialEq for WString<LEN> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<const LEN: usize> std::cmp::PartialEq<&[u16]> for WString<LEN> {
    fn eq(&self, other: &&[u16]) -> bool {
        self.as_slice() == *other
    }
}

impl<const LEN: usize> std::cmp::PartialEq<&str> for WString<LEN> {
    fn eq(&self, other: &&str) -> bool {
        self.as_slice().iter().cloned().eq(other.encode_utf16())
    }
}

// conversion with [u16; N]

impl<const LEN: usize> std::convert::From<[u16; LEN]> for WString<LEN> {
    fn from(arr: [u16; LEN]) -> Self {
        Self(arr, 0)
    }
}

impl<const LEN: usize> std::convert::From<WString<LEN>> for [u16; LEN] {
    fn from(wstr: WString<LEN>) -> Self {
        wstr.0
    }
}

// conversion with [u16]

impl<const LEN: usize> std::convert::TryFrom<&'_ [u16]> for WString<LEN> {
    type Error = ();
    fn try_from(arr: &[u16]) -> std::result::Result<Self, ()> {
        if arr.len() > LEN {
            return Err(());
        }
        let mut wstr = Self::new();
        wstr.0[..arr.len()].copy_from_slice(arr);
        Ok(wstr)
    }
}

impl<const LEN: usize> std::convert::From<WString<LEN>> for std::vec::Vec<u16> {
    fn from(wstr: WString<LEN>) -> Self {
        wstr.as_slice().to_vec()
    }
}

// conversion with strings

impl<const LEN: usize> std::convert::TryFrom<&'_ str> for WString<LEN> {
    type Error = ();
    fn try_from(s: &str) -> std::result::Result<Self, ()> {
        let mut wstr = Self::new();
        for (i, unit) in s.encode_utf16().enumerate() {
            if i >= wstr.0.len() {
                return Err(());
            }
            wstr.0[i] = unit;
        }
        Ok(wstr)
    }
}

impl<const LEN: usize> std::convert::TryFrom<WString<LEN>> for std::string::String {
    type Error = std::char::DecodeUtf16Error;
    fn try_from(wstr: WString<LEN>) -> std::result::Result<Self, Self::Error> {
        std::char::decode_utf16(wstr.0.iter().cloned().take_while(|&b| b != 0)).collect()
    }
}

// zerocopy implementations

unsafe impl<const LEN: usize> zerocopy::AsBytes for WString<LEN> {
    fn only_derive_is_allowed_to_implement_this_trait() { }
}

// compatibility aliases

/// Alias for `String<80>`.
pub type String80 = String<80>;

/// Alias for `WString<80>`.
pub type WString80 = WString<80>;
