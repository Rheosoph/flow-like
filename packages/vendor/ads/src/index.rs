//! Well-known ADS index groups.
//!
//! These are defined
//! [here](https://infosys.beckhoff.com/content/1033/tc3_ads_intro/117241867.html?id=1944752650545554679)
//! and [here](https://github.com/Beckhoff/ADS/blob/master/AdsLib/standalone/AdsDef.h).

// Unfortunately, not all those constants are documented.
#![allow(missing_docs)]

/// PLC: Read/write PLC memory (%M fields).
pub const PLC_RW_M: u32 = 0x4020;
/// PLC: Read/write PLC memory as bits (%MX fields).  Offset is (byte*8 + bit) address.
pub const PLC_RW_MX: u32 = 0x4021;
/// PLC: Read byte length of %M area (only offset 0).
pub const PLC_SIZE_M: u32 = 0x4025;
/// PLC: Read/write retain data area.
pub const PLC_RW_RB: u32 = 0x4030;
/// PLC: Read byte length of the retain data area (only offset 0).
pub const PLC_SIZE_RB: u32 = 0x4035;
/// PLC: Read/write data area.
pub const PLC_RW_DB: u32 = 0x4040;
/// PLC: Read byte length of data area (only offset 0).
pub const PLC_SIZE_DB: u32 = 0x4045;

/// Get u32 handle to the name in the write data.  Index offset is 0.
/// Use with a `write_read` transaction.
pub const GET_SYMHANDLE_BYNAME: u32 = 0xF003;
/// Read/write data for a symbol by handle.
/// Use the handle as the index offset.
pub const RW_SYMVAL_BYHANDLE: u32 = 0xF005;
/// Release a symbol handle.  Index offset is 0.
pub const RELEASE_SYMHANDLE: u32 = 0xF006;

// undocumented; from AdsDef.h
pub const SYMTAB: u32 = 0xF000;
pub const SYMNAME: u32 = 0xF001;
pub const SYMVAL: u32 = 0xF002;
pub const GET_SYMVAL_BYNAME: u32 = 0xF004;
pub const GET_SYMINFO_BYNAME: u32 = 0xF007;
pub const GET_SYMVERSION: u32 = 0xF008;
pub const GET_SYMINFO_BYNAME_EX: u32 = 0xF009;
pub const SYM_DOWNLOAD: u32 = 0xF00A;
pub const SYM_UPLOAD: u32 = 0xF00B;
pub const SYM_UPLOAD_INFO: u32 = 0xF00C;
pub const SYM_DOWNLOAD2: u32 = 0xF00D;
pub const SYM_DT_UPLOAD: u32 = 0xF00E;
pub const SYM_UPLOAD_INFO2: u32 = 0xF00F;
pub const SYM_NOTE: u32 = 0xF010;

/// Read/write process image of physical inputs (%I fields).
pub const IO_RW_I: u32 = 0xF020;
/// Read/write process image of physical inputs as bits (%IX fields).
pub const IO_RW_IX: u32 = 0xF021;
/// Read byte length of the physical inputs (only offset 0).
pub const IO_SIZE_I: u32 = 0xF025;

/// Read/write process image of physical outputs (%Q fields).
pub const IO_RW_Q: u32 = 0xF030;
/// Read/write process image of physical outputs as bits (%QX fields).
pub const IO_RW_QX: u32 = 0xF031;
/// Read byte length of the physical outputs (only offset 0).
pub const IO_SIZE_Q: u32 = 0xF035;

pub const IO_CLEAR_I: u32 = 0xF040;
pub const IO_CLEAR_O: u32 = 0xF050;
pub const IO_RW_IOB: u32 = 0xF060;

/// Combine multiple index group/offset reads.
/// See Beckhoff docs for the format of the data.
pub const SUMUP_READ: u32 = 0xF080;
/// Combine multiple index group/offset writes.
/// See Beckhoff docs for the format of the data.
pub const SUMUP_WRITE: u32 = 0xF081;
/// Combine multiple index group/offset write+reads.
/// See Beckhoff docs for the format of the data.
pub const SUMUP_READWRITE: u32 = 0xF082;
/// Combine multiple index group/offset reads.
/// See Beckhoff docs for the format of the data.
pub const SUMUP_READ_EX: u32 = 0xF083;
/// Combine multiple index group/offset reads.
/// See Beckhoff docs for the format of the data.
pub const SUMUP_READ_EX_2: u32 = 0xF084;
/// Combine multiple device notification adds.
/// See Beckhoff docs for the format of the data.
pub const SUMUP_ADDDEVNOTE: u32 = 0xF085;
/// Combine multiple device notification deletes.
/// See Beckhoff docs for the format of the data.
pub const SUMUP_DELDEVNOTE: u32 = 0xF086;

pub const DEVICE_DATA: u32 = 0xF100;

/// File service: open a file.
pub const FILE_OPEN: u32 = 120;
/// File service: close an open file.
pub const FILE_CLOSE: u32 = 121;
/// File service: read from an open file.
pub const FILE_READ: u32 = 122;
/// File service: write to an open file.
pub const FILE_WRITE: u32 = 123;
/// File service: delete a file.
pub const FILE_DELETE: u32 = 131;
/// File service: browse files.
pub const FILE_BROWSE: u32 = 133;

/// Index group for target desc query.
pub const TARGET_DESC: u32 = 0x2bc;

/// Index group for license queries.
pub const LICENSE: u32 = 0x0101_0004;
pub const LICENSE_MODULES: u32 = 0x0101_0006;

// Diverse officially undocumented ports, used with the system service.
pub const WIN_REGISTRY: u32 = 200;
pub const EXECUTE: u32 = 500;
pub const TC_TARGET_XML: u32 = 700;
pub const ROUTE_ADD: u32 = 801;
pub const ROUTE_REMOVE: u32 = 802;
pub const ROUTE_LIST: u32 = 803;
