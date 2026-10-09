//! Defines ADS error types.

/// Result alias for `ads::Error`.
pub type Result<T> = std::result::Result<T, Error>;

/// A collection of different errors that can happen with ADS requests.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An IO error occurred.
    #[error("{0}: {1}")]
    Io(&'static str, std::io::Error),

    /// The ADS server responded with an error code.
    #[error("{0}: {1} ({2:#x})")]
    Ads(&'static str, &'static str, u32),

    /// An unexpected or inconsistent reply was received.
    #[error("{0}: {1} ({2})")]
    Reply(&'static str, &'static str, u32),

    /// A value exceeds the allowed 32 bits for ADS.
    #[error("data length or duration exceeds 32 bits")]
    Overflow(#[from] std::num::TryFromIntError),
}

pub(crate) trait ErrContext {
    type Success;
    fn ctx(self, context: &'static str) -> Result<Self::Success>;
}

impl<T> ErrContext for std::result::Result<T, std::io::Error> {
    type Success = T;
    fn ctx(self, context: &'static str) -> Result<Self::Success> {
        self.map_err(|e| Error::Io(context, e))
    }
}

/// The list of known ADS error codes from
/// [here](https://infosys.beckhoff.com/content/1033/tc3_ads_intro_howto/374277003.html?id=2736996179007627436).
pub const ADS_ERRORS: &[(u32, &str)] = &[
    (0x001, "Internal error"),
    (0x002, "No real-time"),
    (0x003, "Allocation locked - memory error"),
    (0x004, "Mailbox full - ADS message could not be sent"),
    (0x005, "Wrong receive HMSG"),
    (0x006, "Target port not found, possibly ADS server not started"),
    (0x007, "Target machine not found, possibly missing ADS routes"),
    (0x008, "Unknown command ID"),
    (0x009, "Invalid task ID"),
    (0x00A, "No IO"),
    (0x00B, "Unknown AMS command"),
    (0x00C, "Win32 error"),
    (0x00D, "Port not connected"),
    (0x00E, "Invalid AMS length"),
    (0x00F, "Invalid AMS NetID"),
    (0x010, "Low installation level"),
    (0x011, "No debugging available"),
    (0x012, "Port disabled - system service not started"),
    (0x013, "Port already connected"),
    (0x014, "AMS Sync Win32 error"),
    (0x015, "AMS Sync timeout"),
    (0x016, "AMS Sync error"),
    (0x017, "AMS Sync no index map"),
    (0x018, "Invalid AMS port"),
    (0x019, "No memory"),
    (0x01A, "TCP send error"),
    (0x01B, "Host unreachable"),
    (0x01C, "Invalid AMS fragment"),
    (0x01D, "TLS send error - secure ADS connection failed"),
    (0x01E, "Access denied - secure ADS access denied"),

    (0x500, "Router: no locked memory"),
    (0x501, "Router: memory size could not be changed"),
    (0x502, "Router: mailbox full"),
    (0x503, "Router: debug mailbox full"),
    (0x504, "Router: port type is unknown"),
    (0x505, "Router is not initialized"),
    (0x506, "Router: desired port number is already assigned"),
    (0x507, "Router: port not registered"),
    (0x508, "Router: maximum number of ports reached"),
    (0x509, "Router: port is invalid"),
    (0x50A, "Router is not active"),
    (0x50B, "Router: mailbox full for fragmented messages"),
    (0x50C, "Router: fragment timeout occurred"),
    (0x50D, "Router: port removed"),

    (0x700, "General device error"),
    (0x701, "Service is not supported by server"),
    (0x702, "Invalid index group"),
    (0x703, "Invalid index offset"),
    (0x704, "Reading/writing not permitted"),
    (0x705, "Parameter size not correct"),
    (0x706, "Invalid parameter value(s)"),
    (0x707, "Device is not in a ready state"),
    (0x708, "Device is busy"),
    (0x709, "Invalid OS context -> use multi-task data access"),
    (0x70A, "Out of memory"),
    (0x70B, "Invalid parameter value(s)"),
    (0x70C, "Not found (files, ...)"),
    (0x70D, "Syntax error in command or file"),
    (0x70E, "Objects do not match"),
    (0x70F, "Object already exists"),
    (0x710, "Symbol not found"),
    (0x711, "Symbol version invalid -> create a new handle"),
    (0x712, "Server is in an invalid state"),
    (0x713, "AdsTransMode not supported"),
    (0x714, "Notification handle is invalid"),
    (0x715, "Notification client not registered"),
    (0x716, "No more notification handles"),
    (0x717, "Notification size too large"),
    (0x718, "Device not initialized"),
    (0x719, "Device has a timeout"),
    (0x71A, "Query interface failed"),
    (0x71B, "Wrong interface required"),
    (0x71C, "Class ID is invalid"),
    (0x71D, "Object ID is invalid"),
    (0x71E, "Request is pending"),
    (0x71F, "Request is aborted"),
    (0x720, "Signal warning"),
    (0x721, "Invalid array index"),
    (0x722, "Symbol not active -> release handle and try again"),
    (0x723, "Access denied"),
    (0x724, "No license found -> activate license"),
    (0x725, "License expired"),
    (0x726, "License exceeded"),
    (0x727, "License invalid"),
    (0x728, "Invalid system ID in license"),
    (0x729, "License not time limited"),
    (0x72A, "License issue time in the future"),
    (0x72B, "License time period too long"),
    (0x72C, "Exception in device specific code -> check each device"),
    (0x72D, "License file read twice"),
    (0x72E, "Invalid signature"),
    (0x72F, "Invalid public key certificate"),
    (0x730, "Public key not known from OEM"),
    (0x731, "License not valid for this system ID"),
    (0x732, "Demo license prohibited"),
    (0x733, "Invalid function ID"),
    (0x734, "Outside the valid range"),
    (0x735, "Invalid alignment"),
    (0x736, "Invalid platform level"),
    (0x737, "Context - forward to passive level"),
    (0x738, "Content - forward to dispatch level"),
    (0x739, "Context - forward to real-time"),

    (0x740, "General client error"),
    (0x741, "Invalid parameter at service"),
    (0x742, "Polling list is empty"),
    (0x743, "Var connection already in use"),
    (0x744, "Invoke ID in use"),
    (0x745, "Timeout elapsed -> check route setting"),
    (0x746, "Error in Win32 subsystem"),
    (0x747, "Invalid client timeout value"),
    (0x748, "ADS port not opened"),
    (0x749, "No AMS address"),
    (0x750, "Internal error in ADS sync"),
    (0x751, "Hash table overflow"),
    (0x752, "Key not found in hash"),
    (0x753, "No more symbols in cache"),
    (0x754, "Invalid response received"),
    (0x755, "Sync port is locked"),

    (0x1000, "Internal error in real-time system"),
    (0x1001, "Timer value not valid"),
    (0x1002, "Task pointer has invalid value 0"),
    (0x1003, "Stack pointer has invalid value 0"),
    (0x1004, "Requested task priority already assigned"),
    (0x1005, "No free Task Control Block"),
    (0x1006, "No free semaphores"),
    (0x1007, "No free space in the queue"),
    (0x100D, "External sync interrupt already applied"),
    (0x100E, "No external sync interrupt applied"),
    (0x100F, "External sync interrupt application failed"),
    (0x1010, "Call of service function in wrong context"),
    (0x1017, "Intel VT-x not supported"),
    (0x1018, "Intel VT-x not enabled in BIOS"),
    (0x1019, "Missing function in Intel VT-x"),
    (0x101A, "Activation of Intel VT-x failed"),
];

/// Return an `Error` corresponding to the given ADS result code.
pub fn ads_error<T>(action: &'static str, err: u32) -> Result<T> {
    match ADS_ERRORS.binary_search_by_key(&err, |e| e.0) {
        Ok(idx) => Err(Error::Ads(action, ADS_ERRORS[idx].1, err)),
        Err(_) => Err(Error::Ads(action, "Unknown error code", err)),
    }
}
