//! Contains the TCP client to connect to an ADS server.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::convert::{TryFrom, TryInto};
use std::io::{self, Read, Write};
use std::mem::size_of;
use std::net::{IpAddr, Shutdown, TcpStream, ToSocketAddrs};
use std::str::FromStr;
use std::time::Duration;

use byteorder::{ByteOrder, ReadBytesExt, LE};
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use itertools::Itertools;

use crate::errors::{ads_error, ErrContext};
use crate::notif;
use crate::{AmsAddr, AmsNetId, Error, Result};

use zerocopy::byteorder::{U16, U32};
use zerocopy::{AsBytes, FromBytes, FromZeroes};

/// An ADS protocol command.
// https://infosys.beckhoff.com/content/1033/tc3_ads_intro/115847307.html?id=7738940192708835096
#[repr(u16)]
#[derive(Clone, Copy, Debug)]
pub enum Command {
    /// Return device info
    DevInfo = 1,
    /// Read some data
    Read = 2,
    /// Write some data
    Write = 3,
    /// Write some data, then read back some data
    /// (used as a poor-man's function call)
    ReadWrite = 9,
    /// Read the ADS and device state
    ReadState = 4,
    /// Set the ADS and device state
    WriteControl = 5,
    /// Add a notification for a given index
    AddNotification = 6,
    /// Add a notification for a given index
    DeleteNotification = 7,
    /// Change occurred in a given notification,
    /// can be sent by the PLC only
    Notification = 8,
}

impl Command {
    fn action(self) -> &'static str {
        match self {
            Command::DevInfo => "get device info",
            Command::Read => "read data",
            Command::Write => "write data",
            Command::ReadWrite => "write and read data",
            Command::ReadState => "read state",
            Command::WriteControl => "write control",
            Command::AddNotification => "add notification",
            Command::DeleteNotification => "delete notification",
            Command::Notification => "notification",
        }
    }
}

/// Size of the AMS/TCP + AMS headers
// https://infosys.beckhoff.com/content/1033/tc3_ads_intro/115845259.html?id=6032227753916597086
pub(crate) const TCP_HEADER_SIZE: usize = 6;
pub(crate) const AMS_HEADER_SIZE: usize = 38;  // including AMS/TCP header
pub(crate) const DEFAULT_BUFFER_SIZE: usize = 100;

/// Holds the different timeouts that will be used by the Client.
/// None means no timeout in every case.
#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    /// Connect timeout
    pub connect: Option<Duration>,
    /// Reply read timeout
    pub read: Option<Duration>,
    /// Socket write timoeut
    pub write: Option<Duration>,
}

impl Timeouts {
    /// Create a new `Timeouts` where all values are identical.
    pub fn new(duration: Duration) -> Self {
        Self { connect: Some(duration), read: Some(duration), write: Some(duration) }
    }

    /// Create a new `Timeouts` without any timeouts specified.
    pub fn none() -> Self {
        Self { connect: None, read: None, write: None }
    }
}

/// Specifies the source AMS address to use.
#[derive(Clone, Copy, Debug)]
pub enum Source {
    /// Auto-generate a source address from the local address and a random port.
    Auto,
    /// Use a specified source address.
    Addr(AmsAddr),
    /// Request to open a port in the connected router and get the address from
    /// it.  This is necessary when connecting to a local PLC on `127.0.0.1`.
    Request,
}

/// Represents a connection to a ADS server.
///
/// The Client's communication methods use `&self`, so that it can be freely
/// shared within one thread, or sent, between threads.  Wrappers such as
/// `Device` or `symbol::Handle` use a `&Client` as well.
pub struct Client {
    /// TCP connection (duplicated with the reader)
    socket: TcpStream,
    /// Current invoke ID (identifies the request/reply pair), incremented
    /// after each request
    invoke_id: Cell<u32>,
    /// Read timeout (actually receive timeout for the channel)
    read_timeout: Option<Duration>,
    /// The AMS address of the client
    source: AmsAddr,
    /// Sender for used Vec buffers to the reader thread
    buf_send: Sender<Vec<u8>>,
    /// Receiver for synchronous replies: used in `communicate`
    reply_recv: Receiver<Result<Vec<u8>>>,
    /// Receiver for notifications: cloned and given out to interested parties
    notif_recv: Receiver<notif::Notification>,
    /// Active notification handles: these will be closed on Drop
    notif_handles: RefCell<BTreeSet<(AmsAddr, notif::Handle)>>,
    /// If we opened our local port with the router
    source_port_opened: bool,
}

impl Drop for Client {
    fn drop(&mut self) {
        // Close all open notification handles.
        let handles = std::mem::take(&mut *self.notif_handles.borrow_mut());
        for (addr, handle) in handles {
            let _ = self.device(addr).delete_notification(handle);
        }

        // Remove our port from the router, if necessary.
        if self.source_port_opened {
            let mut close_port_msg = [1, 0, 2, 0, 0, 0, 0, 0];
            LE::write_u16(&mut close_port_msg[6..], self.source.port());
            let _ = self.socket.write_all(&close_port_msg);
        }

        // Need to shutdown the connection since the socket is duplicated in the
        // reader thread.  This will cause the read() in the thread to return
        // with no data.
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}

impl Client {
    /// Open a new connection to an ADS server.
    ///
    /// If connecting to a server that has an AMS router, it needs to have a
    /// route set for the source IP and NetID, otherwise the connection will be
    /// closed immediately.  The route can be added from TwinCAT, or this
    /// crate's `udp::add_route` helper can be used to add a route via UDP
    /// message.
    ///
    /// `source` is the AMS address to to use as the source; the NetID needs to
    /// match the route entry in the server.  If `Source::Auto`, the NetID is
    /// constructed from the local IP address with .1.1 appended; if there is no
    /// IPv4 address, `127.0.0.1.1.1` is used.
    ///
    /// The AMS port of `source` is not important, as long as it is not a
    /// well-known service port; an ephemeral port number > 49152 is
    /// recommended.  If Auto, the port is set to 58913.
    ///
    /// If you are connecting to the local PLC, you need to set `source` to
    /// `Source::Request`.  This will ask the local AMS router for a new
    /// port and use it as the source port.
    ///
    /// Since all communications is supposed to be handled by an ADS router,
    /// only one TCP/ADS connection can exist between two hosts. Non-TwinCAT
    /// clients should make sure to replicate this behavior, as opening a second
    /// connection will close the first.
    pub fn new(addr: impl ToSocketAddrs, timeouts: Timeouts, source: Source) -> Result<Self> {
        Self::new_with_packet_limit(addr, timeouts, source, u32::MAX as usize)
    }

    /// Open a connection with a maximum incoming packet length, excluding the
    /// six-byte AMS/TCP header. The limit applies to replies, notification
    /// batches, and router control messages. Oversized packets close the
    /// connection before allocating their payload. The minimum limit is 32.
    pub fn new_with_packet_limit(addr: impl ToSocketAddrs, timeouts: Timeouts,
                                 source: Source, max_packet_length: usize) -> Result<Self> {
        if max_packet_length < AMS_HEADER_SIZE - TCP_HEADER_SIZE {
            return Err(Error::Io("validating packet limit",
                io::Error::new(io::ErrorKind::InvalidInput, "packet limit must be at least 32 bytes")));
        }
        // Connect, taking the timeout into account.  Unfortunately
        // connect_timeout wants a single SocketAddr.
        let addr = addr.to_socket_addrs().ctx("converting address to SocketAddr")?
                                         .next().expect("at least one SocketAddr");
        let mut socket = if let Some(timeout) = timeouts.connect {
            TcpStream::connect_timeout(&addr, timeout).ctx("connecting TCP socket with timeout")?
        } else {
            TcpStream::connect(addr).ctx("connecting TCP socket")?
        };

        // Disable Nagle to ensure small requests are sent promptly; we're
        // playing ping-pong with request reply, so no pipelining.
        socket.set_nodelay(true).ctx("setting NODELAY")?;
        socket.set_write_timeout(timeouts.write).ctx("setting write timeout")?;

        // Determine our source AMS address.  If it's not specified, try to use
        // the socket's local IPv4 address, if it's IPv6 (not sure if Beckhoff
        // devices support that) use `127.0.0.1` as the last resort.
        //
        // If source is Request, send an AMS port open message to the connected
        // router to get our source address.  This is required when connecting
        // via localhost, apparently.
        let mut source_port_opened = false;
        let source = match source {
            Source::Addr(id) => id,
            Source::Auto => {
                let my_addr = socket.local_addr().ctx("getting local socket address")?.ip();
                if let IpAddr::V4(ip) = my_addr {
                    let [a, b, c, d] = ip.octets();
                    // use some random ephemeral port
                    AmsAddr::new(AmsNetId::new(a, b, c, d, 1, 1), 58913)
                } else {
                    AmsAddr::new(AmsNetId::new(127, 0, 0, 1, 1, 1), 58913)
                }
            }
            Source::Request => {
                let request_port_msg = [0, 16, 2, 0, 0, 0, 0, 0];
                let mut reply = [0; 14];
                socket.write_all(&request_port_msg).ctx("requesting port from router")?;
                socket.read_exact(&mut reply).ctx("requesting port from router")?;
                if reply[..6] != [0, 16, 8, 0, 0, 0] {
                    return Err(Error::Reply("requesting port", "unexpected reply header", 0));
                }
                source_port_opened = true;
                AmsAddr::new(AmsNetId::from_slice(&reply[6..12]).expect("size"),
                             LE::read_u16(&reply[12..14]))
            }
        };

        // Clone the socket for the reader thread and create our channels for
        // bidirectional communication.
        let socket_clone = socket.try_clone().ctx("cloning TCP socket")?;
        let (buf_send, buf_recv) = bounded(10);
        let (reply_send, reply_recv) = bounded(1);
        let (notif_send, notif_recv) = unbounded();
        let mut source_bytes = [0; 8];
        source.write_to(&mut &mut source_bytes[..]).expect("size");

        // Start the reader thread.
        let reader = Reader {
            socket: socket_clone,
            max_packet_length,
            source: source_bytes,
            buf_recv,
            reply_send,
            notif_send,
        };
        std::thread::spawn(|| reader.run());

        Ok(Client {
            socket,
            source,
            buf_send,
            reply_recv,
            notif_recv,
            invoke_id: Cell::new(0),
            read_timeout: timeouts.read,
            notif_handles: RefCell::default(),
            source_port_opened,
        })
    }

    /// Return the source address the client is using.
    pub fn source(&self) -> AmsAddr {
        self.source
    }

    /// Get a receiver for notifications.
    pub fn get_notification_channel(&self) -> Receiver<notif::Notification> {
        self.notif_recv.clone()
    }

    /// Return a wrapper that executes operations for a target device (known by
    /// NetID and port).
    ///
    /// The local NetID `127.0.0.1.1.1` is mapped to the client's source NetID,
    /// so that you can connect to a local PLC using:
    ///
    /// ```rust,ignore
    /// let client = Client::new("127.0.0.1", ..., Source::Request);
    /// let device = client.device(AmsAddr::new(AmsNetId::local(), 851));
    /// ```
    ///
    /// without knowing its NetID.
    pub fn device(&self, mut addr: AmsAddr) -> Device<'_> {
        if addr.netid() == AmsNetId::local() {
            addr = AmsAddr::new(self.source().netid(), addr.port());
        }
        Device { client: self, addr }
    }

    /// Low-level function to execute an ADS command.
    ///
    /// Writes a data from a number of input buffers, and returns data in a
    /// number of output buffers.  The latter might not be filled completely;
    /// the return value specifies the number of total valid bytes.  It is up to
    /// the caller to determine what this means in terms of the passed buffers.
    pub fn communicate(&self,
                       cmd: Command,
                       target: AmsAddr,
                       data_in: &[&[u8]],
                       data_out: &mut [&mut [u8]]) -> Result<usize> {
        // Increase the invoke ID.  We could also generate a random u32, but
        // this way the sequence of packets can be tracked.
        self.invoke_id.set(self.invoke_id.get().wrapping_add(1));

        // The data we send is the sum of all data_in buffers.
        let data_in_len = data_in.iter().map(|v| v.len()).sum::<usize>();

        // Create outgoing header.
        let ads_data_len = AMS_HEADER_SIZE - TCP_HEADER_SIZE + data_in_len;
        let header = AdsHeader {
            ams_cmd:     0,  // send command
            length:      U32::new(ads_data_len.try_into()?),
            dest_netid:  target.netid(),
            dest_port:   U16::new(target.port()),
            src_netid:   self.source.netid(),
            src_port:    U16::new(self.source.port()),
            command:     U16::new(cmd as u16),
            state_flags: U16::new(4),                   // state flags (4 = send command)
            data_length: U32::new(data_in_len as u32),  // overflow checked above
            error_code:  U32::new(0),
            invoke_id:   U32::new(self.invoke_id.get()),
        };

        // Collect the outgoing data.  Note, allocating a Vec and calling
        // `socket.write_all` only once is faster than writing in multiple
        // steps, even with TCP_NODELAY.
        let mut request = Vec::with_capacity(ads_data_len);
        request.extend_from_slice(header.as_bytes());
        for buf in data_in {
            request.extend_from_slice(buf);
        }
        // &T impls Write for T: Write, so no &mut self required.
        (&self.socket).write_all(&request).ctx("sending request")?;

        // Get a reply from the reader thread, with timeout or not.
        let reply = if let Some(tmo) = self.read_timeout {
            self.reply_recv.recv_timeout(tmo).map_err(|_| io::ErrorKind::TimedOut.into())
                                             .ctx("receiving reply (route set?)")?
        } else {
            self.reply_recv.recv().map_err(|_| io::ErrorKind::UnexpectedEof.into())
                                  .ctx("receiving reply (route set?)")?
        }?;

        // Validate the incoming reply.  The reader thread already made sure that
        // it is consistent and addressed to us.

        // The source netid/port must match what we sent.
        if reply[14..22] != request[6..14] {
            return Err(Error::Reply(cmd.action(), "unexpected source address", 0));
        }
        // Read the other fields we need.
        assert!(reply.len() >= AMS_HEADER_SIZE);
        let mut ptr = &reply[22..];
        let ret_cmd = ptr.read_u16::<LE>().expect("size");
        let state_flags = ptr.read_u16::<LE>().expect("size");
        let data_len = ptr.read_u32::<LE>().expect("size");
        let error_code = ptr.read_u32::<LE>().expect("size");
        let invoke_id = ptr.read_u32::<LE>().expect("size");
        let result = if reply.len() >= AMS_HEADER_SIZE + 4 {
            ptr.read_u32::<LE>().expect("size")
        } else {
            0  // this must be because an error code is already set
        };

        // Command must match.
        if ret_cmd != cmd as u16 {
            return Err(Error::Reply(cmd.action(), "unexpected command", ret_cmd.into()));
        }
        // State flags must be "4 | 1".
        if state_flags != 5 {
            return Err(Error::Reply(cmd.action(), "unexpected state flags", state_flags.into()));
        }
        // Invoke ID must match what we sent.
        if invoke_id != self.invoke_id.get() {
            return Err(Error::Reply(cmd.action(), "unexpected invoke ID", invoke_id));
        }
        // Check error code in AMS header.
        if error_code != 0 {
            return ads_error(cmd.action(), error_code);
        }
        // Check result field in payload, only relevant if error_code == 0.
        if result != 0 {
            return ads_error(cmd.action(), result);
        }

        // If we don't want return data, we're done.
        if data_out.is_empty() {
            let _ = self.buf_send.send(reply);
            return Ok(0);
        }

        // Check returned length, it needs to fill at least the first data_out
        // buffer.  This also ensures that we had a result field.
        if (data_len as usize) < data_out[0].len() + 4 {
            return Err(Error::Reply(cmd.action(), "got less data than expected", data_len));
        }

        // The pure user data length, without the result field.
        let data_len = data_len as usize - 4;

        // Distribute the data into the user output buffers, up to the returned
        // data length.
        let mut offset = AMS_HEADER_SIZE + 4;
        let mut rest_len = data_len;
        for buf in data_out {
            let n = buf.len().min(rest_len);
            buf[..n].copy_from_slice(&reply[offset..][..n]);
            offset += n;
            rest_len -= n;
            if rest_len == 0 {
                break;
            }
        }

        // Send back the Vec buffer to the reader thread.
        let _ = self.buf_send.send(reply);

        // Return either the error or the length of data.
        Ok(data_len)
    }
}

// Implementation detail: reader thread that takes replies and notifications
// and distributes them accordingly.
struct Reader {
    socket: TcpStream,
    max_packet_length: usize,
    source: [u8; 8],
    buf_recv: Receiver<Vec<u8>>,
    reply_send: Sender<Result<Vec<u8>>>,
    notif_send: Sender<notif::Notification>,
}

fn packet_buffer_length(packet_length: usize, limit: usize) -> Result<usize> {
    packet_length.checked_add(TCP_HEADER_SIZE)
        .filter(|_| packet_length <= limit)
        .ok_or(Error::Reply("reading packet", "packet exceeds configured limit", packet_length as _))
}

impl Reader {
    fn run(mut self) {
        self.run_inner();
        // We can't do much here.  But try to shut down the socket so that
        // the main client can't be used anymore either.
        let _ = self.socket.shutdown(Shutdown::Both);
    }

    fn run_inner(&mut self) {
        loop {
            // Get a buffer from the free-channel or create a new one.
            let mut buf = self.buf_recv.try_recv()
                                       .unwrap_or_else(|_| Vec::with_capacity(DEFAULT_BUFFER_SIZE));

            // Read a header from the socket.
            buf.resize(TCP_HEADER_SIZE, 0);
            if self.socket.read_exact(&mut buf).ctx("reading AMS packet header").is_err() {
                // Not sending an error back; we don't know if something was
                // requested or the socket was just closed from either side.
                return;
            }

            // Read the rest of the packet.
            let packet_length = LE::read_u32(&buf[2..6]) as usize;
            let buffer_length = match packet_buffer_length(packet_length, self.max_packet_length) {
                Ok(length) => length,
                Err(error) => {
                    let _ = self.reply_send.send(Err(error));
                    return;
                }
            };
            let ams_cmd = LE::read_u16(&buf);
            if ams_cmd == 0 && packet_length < AMS_HEADER_SIZE - TCP_HEADER_SIZE {
                let _ = self.reply_send.send(Err(Error::Reply(
                    "reading packet", "truncated AMS header", packet_length as _)));
                return;
            }
            buf.resize(buffer_length, 0);
            if let Err(e) = self.socket.read_exact(&mut buf[6..])
                                       .ctx("reading rest of packet") {
                let _ = self.reply_send.send(Err(e));
                return;
            }

            // Is it something other than an ADS command packet?
            if ams_cmd != 0 {
                // if it's a known packet type, continue
                if matches!(ams_cmd, 1 | 4096 | 4097 | 4098) {
                    continue;
                }
                let _ = self.reply_send.send(Err(Error::Reply(
                    "reading packet", "invalid packet or unknown AMS command", ams_cmd as _)));
                return;
            }

            // If the header length fields aren't self-consistent, abort the connection.
            let rest_length = LE::read_u32(&buf[26..30]) as usize;
            if rest_length != packet_length + TCP_HEADER_SIZE - AMS_HEADER_SIZE {
                let _ = self.reply_send.send(Err(Error::Reply("reading packet",
                                                              "inconsistent packet", 0)));
                return;
            }

            // Check that the packet is meant for us.
            if buf[6..14] != self.source {
                continue;
            }

            // If it looks like a reply, send it back to the requesting thread,
            // it will handle further validation.
            if LE::read_u16(&buf[22..24]) != Command::Notification as u16 {
                if self.reply_send.send(Ok(buf)).is_err() {
                    // Client must have been shut down.
                    return;
                }
                continue;
            }

            // Validate notification message fields.
            let state_flags = LE::read_u16(&buf[24..26]);
            let error_code = LE::read_u32(&buf[30..34]);
            if state_flags != 4 || error_code != 0 {
                continue;
            }

            // Send the notification to whoever wants to receive it.
            if let Ok(notif) = notif::Notification::new(buf) {
                self.notif_send.send(notif).expect("never disconnects");
            }
        }
    }
}

/// A `Client` wrapper that talks to a specific ADS device.
#[derive(Clone, Copy)]
pub struct Device<'c> {
    /// The underlying `Client`.
    pub client: &'c Client,
    addr: AmsAddr,
}

impl<'c> Device<'c> {
    /// Read the device's name + version.
    pub fn get_info(&self) -> Result<DeviceInfo> {
        let mut data = DeviceInfoRaw::new_zeroed();
        self.client.communicate(Command::DevInfo, self.addr,
                                &[], &mut [data.as_bytes_mut()])?;

        // Decode the name string, which is null-terminated.  Technically it's
        // Windows-1252, but in practice no non-ASCII occurs.
        let name = data.name.iter().take_while(|&&ch| ch > 0)
                                   .map(|&ch| ch as char).collect::<String>();
        Ok(DeviceInfo {
            major: data.major,
            minor: data.minor,
            version: data.version.get(),
            name,
        })
    }

    /// Read some data at a given index group/offset.  Returned data can be shorter than
    /// the buffer, the length is the return value.
    pub fn read(&self, index_group: u32, index_offset: u32, data: &mut [u8]) -> Result<usize> {
        let header = IndexLength {
            index_group:  U32::new(index_group),
            index_offset: U32::new(index_offset),
            length:       U32::new(data.len().try_into()?),
        };
        let mut read_len = U32::<LE>::new(0);

        self.client.communicate(Command::Read, self.addr,
                                &[header.as_bytes()], &mut [read_len.as_bytes_mut(), data])?;

        Ok(read_len.get() as usize)
    }

    /// Read some data at a given index group/offset, ensuring that the returned data has
    /// exactly the size of the passed buffer.
    pub fn read_exact(&self, index_group: u32, index_offset: u32, data: &mut [u8]) -> Result<()> {
        let len = self.read(index_group, index_offset, data)?;
        if len != data.len() {
            return Err(Error::Reply("read data", "got less data than expected", len as u32));
        }
        Ok(())
    }

    /// Read data of given type.
    ///
    /// Any type that supports `zerocopy::FromBytes` can be read.  You can also
    /// derive that trait on your own structures and read structured data
    /// directly from the symbol.
    ///
    /// Note: to be independent of the host's byte order, use the integer types
    /// defined in `zerocopy::byteorder`.
    pub fn read_value<T: Default + AsBytes + FromBytes>(&self, index_group: u32,
                                                        index_offset: u32) -> Result<T> {
        let mut buf = T::default();
        self.read_exact(index_group, index_offset, buf.as_bytes_mut())?;
        Ok(buf)
    }

    /// Read multiple index groups/offsets with one ADS request (a "sum-up" request).
    ///
    /// The returned data can be shorter than the buffer in each case, the `length`
    /// member of the `ReadRequest` is set to the returned length.
    ///
    /// This function only returns Err on errors that cause the whole sum-up
    /// request to fail (e.g. if the device doesn't support such requests).  If
    /// the request as a whole succeeds, each single read can have returned its
    /// own error.  The [`ReadRequest::data`] method will return either the
    /// properly truncated returned data or the error for each read.
    ///
    /// Example:
    /// ```ignore
    /// // create buffers
    /// let mut buf_1 = [0; 128];  // request reading 128 bytes
    /// let mut buf_2 = [0; 128];  // from two indices
    /// // create the request structures
    /// let mut req_1 = ReadRequest::new(ix1, off1, &mut buf_1);
    /// let mut req_2 = ReadRequest::new(ix2, off2, &mut buf_2);
    /// //  actual request
    /// device.read_multi(&mut [req_1, req_2])?;
    /// // extract the resulting data, checking individual reads for
    /// // errors and getting the returned data otherwise
    /// let res_1 = req_1.data()?;
    /// let res_2 = req_2.data()?;
    /// ```
    pub fn read_multi(&self, requests: &mut [ReadRequest]) -> Result<()> {
        let nreq = requests.len();
        let rlen = requests.iter().map(|r| size_of::<ResultLength>() + r.rbuf.len()).sum::<usize>();
        let wlen = size_of::<IndexLength>() * nreq;
        let header = IndexLengthRW {
            // using SUMUP_READ_EX_2 since would return the actual returned
            // number of bytes, and no empty bytes if the read is short,
            // but then we'd have to reshuffle the buffers
            index_group:  U32::new(crate::index::SUMUP_READ_EX),
            index_offset: U32::new(nreq as u32),
            read_length:  U32::new(rlen.try_into()?),
            write_length: U32::new(wlen.try_into()?),
        };
        let mut read_len = U32::<LE>::new(0);
        let mut w_buffers = vec![header.as_bytes()];
        let mut r_buffers = (0..2*nreq + 1).map(|_| &mut [][..]).collect_vec();
        r_buffers[0] = read_len.as_bytes_mut();
        for (i, req) in requests.iter_mut().enumerate() {
            w_buffers.push(req.req.as_bytes());
            r_buffers[1 + i] = req.res.as_bytes_mut();
            r_buffers[1 + nreq + i] = req.rbuf;
        }
        self.client.communicate(Command::ReadWrite, self.addr, &w_buffers, &mut r_buffers)?;
        Ok(())
    }

    /// Write some data to a given index group/offset.
    pub fn write(&self, index_group: u32, index_offset: u32, data: &[u8]) -> Result<()> {
        let header = IndexLength {
            index_group:  U32::new(index_group),
            index_offset: U32::new(index_offset),
            length:       U32::new(data.len().try_into()?),
        };
        self.client.communicate(Command::Write, self.addr,
                                &[header.as_bytes(), data], &mut [])?;
        Ok(())
    }

    /// Write data of given type.
    ///
    /// See `read_value` for details.
    pub fn write_value<T: AsBytes>(&self, index_group: u32, index_offset: u32,
                                   value: &T) -> Result<()> {
        self.write(index_group, index_offset, value.as_bytes())
    }

    /// Write multiple index groups/offsets with one ADS request (a "sum-up" request).
    ///
    /// This function only returns Err on errors that cause the whole sum-up
    /// request to fail (e.g. if the device doesn't support such requests).  If
    /// the request as a whole succeeds, each single write can have returned its
    /// own error.  The [`WriteRequest::ensure`] method will return the error for
    /// each write.
    pub fn write_multi(&self, requests: &mut [WriteRequest]) -> Result<()> {
        let nreq = requests.len();
        let rlen = size_of::<u32>() * nreq;
        let wlen = requests.iter().map(|r| size_of::<IndexLength>() + r.wbuf.len()).sum::<usize>();
        let header = IndexLengthRW {
            index_group:  U32::new(crate::index::SUMUP_WRITE),
            index_offset: U32::new(nreq as u32),
            read_length:  U32::new(rlen.try_into()?),
            write_length: U32::new(wlen.try_into()?),
        };
        let mut read_len = U32::<LE>::new(0);
        let mut w_buffers = vec![&[][..]; 2*nreq + 1];
        let mut r_buffers = vec![read_len.as_bytes_mut()];
        w_buffers[0] = header.as_bytes();
        for (i, req) in requests.iter_mut().enumerate() {
            w_buffers[1 + i] = req.req.as_bytes();
            w_buffers[1 + nreq + i] = req.wbuf;
            r_buffers.push(req.res.as_bytes_mut());
        }
        self.client.communicate(Command::ReadWrite, self.addr, &w_buffers, &mut r_buffers)?;
        Ok(())
    }

    /// Write some data to a given index group/offset and then read back some
    /// reply from there.  This is not the same as a write() followed by read();
    /// it is used as a kind of RPC call.
    pub fn write_read(&self, index_group: u32, index_offset: u32, write_data: &[u8],
                      read_data: &mut [u8]) -> Result<usize> {
        let header = IndexLengthRW {
            index_group:  U32::new(index_group),
            index_offset: U32::new(index_offset),
            read_length:  U32::new(read_data.len().try_into()?),
            write_length: U32::new(write_data.len().try_into()?),
        };
        let mut read_len = U32::<LE>::new(0);
        self.client.communicate(Command::ReadWrite, self.addr,
                                &[header.as_bytes(), write_data],
                                &mut [read_len.as_bytes_mut(), read_data])?;
        Ok(read_len.get() as usize)
    }

    /// Like `write_read`, but ensure the returned data length matches the output buffer.
    pub fn write_read_exact(&self, index_group: u32, index_offset: u32, write_data: &[u8],
                            read_data: &mut [u8]) -> Result<()> {
        let len = self.write_read(index_group, index_offset, write_data, read_data)?;
        if len != read_data.len() {
            return Err(Error::Reply("write/read data", "got less data than expected", len as u32));
        }
        Ok(())
    }

    /// Write multiple index groups/offsets with one ADS request (a "sum-up" request).
    ///
    /// This function only returns Err on errors that cause the whole sum-up
    /// request to fail (e.g. if the device doesn't support such requests).  If
    /// the request as a whole succeeds, each single write/read can have
    /// returned its own error.  The [`WriteReadRequest::data`] method will
    /// return either the properly truncated returned data or the error for each
    /// write/read.
    pub fn write_read_multi(&self, requests: &mut [WriteReadRequest]) -> Result<()> {
        let nreq = requests.len();
        let rlen = requests.iter().map(|r| size_of::<ResultLength>() + r.rbuf.len()).sum::<usize>();
        let wlen = requests.iter().map(|r| size_of::<IndexLengthRW>() + r.wbuf.len()).sum::<usize>();
        let header = IndexLengthRW {
            index_group:  U32::new(crate::index::SUMUP_READWRITE),
            index_offset: U32::new(nreq as u32),
            read_length:  U32::new(rlen.try_into()?),
            write_length: U32::new(wlen.try_into()?),
        };
        let mut read_len = U32::<LE>::new(0);
        let mut w_buffers = vec![&[][..]; 2*nreq + 1];
        let mut r_buffers = (0..2*nreq + 1).map(|_| &mut [][..]).collect_vec();
        w_buffers[0] = header.as_bytes();
        r_buffers[0] = read_len.as_bytes_mut();
        for (i, req) in requests.iter_mut().enumerate() {
            w_buffers[1 + i] = req.req.as_bytes();
            w_buffers[1 + nreq + i] = req.wbuf;
            r_buffers[1 + i] = req.res.as_bytes_mut();
            r_buffers[1 + nreq + i] = req.rbuf;
        }
        self.client.communicate(Command::ReadWrite, self.addr, &w_buffers, &mut r_buffers)?;
        // unfortunately SUMUP_READWRITE returns only the actual read bytes for each
        // request, so if there are short reads the buffers got filled wrongly
        fixup_write_read_return_buffers(requests);
        Ok(())
    }

    /// Return the ADS and device state of the device.
    pub fn get_state(&self) -> Result<(AdsState, u16)> {
        let mut state = ReadState::new_zeroed();
        self.client.communicate(Command::ReadState, self.addr,
                                &[], &mut [state.as_bytes_mut()])?;

        // Convert ADS state to the enum type
        let ads_state = AdsState::try_from(state.ads_state.get())
            .map_err(|e| Error::Reply("read state", e, state.ads_state.get().into()))?;

        Ok((ads_state, state.dev_state.get()))
    }

    /// (Try to) set the ADS and device state of the device.
    pub fn write_control(&self, ads_state: AdsState, dev_state: u16) -> Result<()> {
        let data = WriteControl {
            ads_state:   U16::new(ads_state as _),
            dev_state:   U16::new(dev_state),
            data_length: U32::new(0),
        };
        self.client.communicate(Command::WriteControl, self.addr,
                                &[data.as_bytes()], &mut [])?;
        Ok(())
    }

    /// Add a notification handle for some index group/offset.
    ///
    /// Notifications are delivered via a MPMC channel whose reading end can be
    /// obtained from `get_notification_channel` on the `Client` object.
    /// The returned `Handle` can be used to check which notification has fired.
    ///
    /// If the notification is not deleted explictly using `delete_notification`
    /// and the `Handle`, it is deleted when the `Client` object is dropped.
    pub fn add_notification(&self, index_group: u32, index_offset: u32,
                            attributes: &notif::Attributes) -> Result<notif::Handle> {
        let data = AddNotif {
            index_group:  U32::new(index_group),
            index_offset: U32::new(index_offset),
            length:       U32::new(attributes.length.try_into()?),
            trans_mode:   U32::new(attributes.trans_mode as u32),
            max_delay:    U32::new(notification_ticks(attributes.max_delay).try_into()?),
            cycle_time:   U32::new(notification_ticks(attributes.cycle_time).try_into()?),
            reserved:     [0; 16],
        };
        let mut handle = U32::<LE>::new(0);
        self.client.communicate(Command::AddNotification, self.addr,
                                &[data.as_bytes()], &mut [handle.as_bytes_mut()])?;
        self.client.notif_handles.borrow_mut().insert((self.addr, handle.get()));
        Ok(handle.get())
    }

    /// Add multiple notification handles.
    ///
    /// This function only returns Err on errors that cause the whole sum-up
    /// request to fail (e.g. if the device doesn't support such requests).  If
    /// the request as a whole succeeds, each single read can have returned its
    /// own error.  The [`AddNotifRequest::handle`] method will return either
    /// the returned handle or the error for each read.
    pub fn add_notification_multi(&self, requests: &mut [AddNotifRequest]) -> Result<()> {
        let nreq = requests.len();
        let rlen = size_of::<ResultLength>() * nreq;
        let wlen = size_of::<AddNotif>() * nreq;
        let header = IndexLengthRW {
            index_group:  U32::new(crate::index::SUMUP_ADDDEVNOTE),
            index_offset: U32::new(nreq as u32),
            read_length:  U32::new(rlen.try_into()?),
            write_length: U32::new(wlen.try_into()?),
        };
        let mut read_len = U32::<LE>::new(0);
        let mut w_buffers = vec![header.as_bytes()];
        let mut r_buffers = vec![read_len.as_bytes_mut()];
        for req in requests.iter_mut() {
            w_buffers.push(req.req.as_bytes());
            r_buffers.push(req.res.as_bytes_mut());
        }
        self.client.communicate(Command::ReadWrite, self.addr, &w_buffers, &mut r_buffers)?;
        for req in requests {
            if let Ok(handle) = req.handle() {
                self.client.notif_handles.borrow_mut().insert((self.addr, handle));
            }
        }
        Ok(())
    }

    /// Delete a notification with given handle.
    pub fn delete_notification(&self, handle: notif::Handle) -> Result<()> {
        self.client.communicate(Command::DeleteNotification, self.addr,
                                &[U32::<LE>::new(handle).as_bytes()], &mut [])?;
        self.client.notif_handles.borrow_mut().remove(&(self.addr, handle));
        Ok(())
    }

    /// Delete multiple notification handles.
    ///
    /// This function only returns Err on errors that cause the whole sum-up
    /// request to fail (e.g. if the device doesn't support such requests).  If
    /// the request as a whole succeeds, each single read can have returned its
    /// own error.  The [`DelNotifRequest::ensure`] method will return either the
    /// returned data or the error for each read.
    pub fn delete_notification_multi(&self, requests: &mut [DelNotifRequest]) -> Result<()> {
        let nreq = requests.len();
        let rlen = size_of::<u32>() * nreq;
        let wlen = size_of::<u32>() * nreq;
        let header = IndexLengthRW {
            index_group:  U32::new(crate::index::SUMUP_DELDEVNOTE),
            index_offset: U32::new(nreq as u32),
            read_length:  U32::new(rlen.try_into()?),
            write_length: U32::new(wlen.try_into()?),
        };
        let mut read_len = U32::<LE>::new(0);
        let mut w_buffers = vec![header.as_bytes()];
        let mut r_buffers = vec![read_len.as_bytes_mut()];
        for req in requests.iter_mut() {
            w_buffers.push(req.req.as_bytes());
            r_buffers.push(req.res.as_bytes_mut());
        }
        self.client.communicate(Command::ReadWrite, self.addr, &w_buffers, &mut r_buffers)?;
        for req in requests {
            if req.ensure().is_ok() {
                self.client.notif_handles.borrow_mut().remove(&(self.addr, req.req.get()));
            }
        }
        Ok(())
    }
}

/// Device info returned from an ADS server.
#[derive(Debug)]
pub struct DeviceInfo {
    /// Name of the ADS device/service.
    pub name: String,
    /// Major version.
    pub major: u8,
    /// Minor version.
    pub minor: u8,
    /// Build version.
    pub version: u16,
}

/// The ADS state of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
#[repr(u16)]
pub enum AdsState {
    Invalid      = 0,
    Idle         = 1,
    Reset        = 2,
    Init         = 3,
    Start        = 4,
    Run          = 5,
    Stop         = 6,
    SaveCfg      = 7,
    LoadCfg      = 8,
    PowerFail    = 9,
    PowerGood    = 10,
    Error        = 11,
    Shutdown     = 12,
    Suspend      = 13,
    Resume       = 14,
    Config       = 15,
    Reconfig     = 16,
    Stopping     = 17,
    Incompatible = 18,
    Exception    = 19,
}

impl TryFrom<u16> for AdsState {
    type Error = &'static str;

    fn try_from(value: u16) -> std::result::Result<Self, &'static str> {
        Ok(match value {
            0  => Self::Invalid,
            1  => Self::Idle,
            2  => Self::Reset,
            3  => Self::Init,
            4  => Self::Start,
            5  => Self::Run,
            6  => Self::Stop,
            7  => Self::SaveCfg,
            8  => Self::LoadCfg,
            9  => Self::PowerFail,
            10 => Self::PowerGood,
            11 => Self::Error,
            12 => Self::Shutdown,
            13 => Self::Suspend,
            14 => Self::Resume,
            15 => Self::Config,
            16 => Self::Reconfig,
            17 => Self::Stopping,
            18 => Self::Incompatible,
            19 => Self::Exception,
            _  => return Err("invalid state constant")
        })
    }
}

impl FromStr for AdsState {
    type Err = &'static str;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match &*s.to_ascii_lowercase() {
            "invalid" => Self::Invalid,
            "idle" => Self::Idle,
            "reset" => Self::Reset,
            "init" => Self::Init,
            "start" => Self::Start,
            "run" => Self::Run,
            "stop" => Self::Stop,
            "savecfg" => Self::SaveCfg,
            "loadcfg" => Self::LoadCfg,
            "powerfail" => Self::PowerFail,
            "powergood" => Self::PowerGood,
            "error" => Self::Error,
            "shutdown" => Self::Shutdown,
            "suspend" => Self::Suspend,
            "resume" => Self::Resume,
            "config" => Self::Config,
            "reconfig" => Self::Reconfig,
            "stopping" => Self::Stopping,
            "incompatible" => Self::Incompatible,
            "exception" => Self::Exception,
            _  => return Err("invalid state name")
        })
    }
}

// Structures used in communication, not exposed to user,
// but pub(crate) for the test suite.

#[derive(AsBytes, FromBytes, FromZeroes, Debug)]
#[repr(C)]
pub(crate) struct AdsHeader {
    /// 0x0 - ADS command
    /// 0x1 - close port
    /// 0x1000 - open port
    /// 0x1001 - note from router (router state changed)
    /// 0x1002 - get local netid
    pub ams_cmd:     u16,
    pub length:      U32<LE>,
    pub dest_netid:  AmsNetId,
    pub dest_port:   U16<LE>,
    pub src_netid:   AmsNetId,
    pub src_port:    U16<LE>,
    pub command:     U16<LE>,
    /// 0x01 - response
    /// 0x02 - no return
    /// 0x04 - ADS command
    /// 0x08 - system command
    /// 0x10 - high priority
    /// 0x20 - with time stamp (8 bytes added)
    /// 0x40 - UDP
    /// 0x80 - command during init phase
    /// 0x8000 - broadcast
    pub state_flags: U16<LE>,
    pub data_length: U32<LE>,
    pub error_code:  U32<LE>,
    pub invoke_id:   U32<LE>,
}

#[derive(FromBytes, FromZeroes, AsBytes)]
#[repr(C)]
pub(crate) struct DeviceInfoRaw {
    pub major:   u8,
    pub minor:   u8,
    pub version: U16<LE>,
    pub name:    [u8; 16],
}

#[derive(FromBytes, FromZeroes, AsBytes)]
#[repr(C)]
pub(crate) struct IndexLength {
    pub index_group:  U32<LE>,
    pub index_offset: U32<LE>,
    pub length:       U32<LE>,
}

#[derive(FromBytes, FromZeroes, AsBytes)]
#[repr(C)]
pub(crate) struct ResultLength {
    pub result:       U32<LE>,
    pub length:       U32<LE>,
}

#[derive(FromBytes, FromZeroes, AsBytes)]
#[repr(C)]
pub(crate) struct IndexLengthRW {
    pub index_group:  U32<LE>,
    pub index_offset: U32<LE>,
    pub read_length:  U32<LE>,
    pub write_length: U32<LE>,
}

#[derive(FromBytes, FromZeroes, AsBytes)]
#[repr(C)]
pub(crate) struct ReadState {
    pub ads_state:   U16<LE>,
    pub dev_state:   U16<LE>,
}

#[derive(FromBytes, FromZeroes, AsBytes)]
#[repr(C)]
pub(crate) struct WriteControl {
    pub ads_state:   U16<LE>,
    pub dev_state:   U16<LE>,
    pub data_length: U32<LE>,
}

#[derive(FromBytes, FromZeroes, AsBytes)]
#[repr(C)]
pub(crate) struct AddNotif {
    pub index_group:  U32<LE>,
    pub index_offset: U32<LE>,
    pub length:       U32<LE>,
    pub trans_mode:   U32<LE>,
    pub max_delay:    U32<LE>,
    pub cycle_time:   U32<LE>,
    pub reserved:     [u8; 16],
}

/// A single request for a [`Device::read_multi`] request.
pub struct ReadRequest<'buf> {
    req:  IndexLength,
    res:  ResultLength,
    rbuf: &'buf mut [u8],
}

impl<'buf> ReadRequest<'buf> {
    /// Create the request with given index group, index offset and result buffer.
    pub fn new(index_group: u32, index_offset: u32, buffer: &'buf mut [u8]) -> Self {
        Self {
            req: IndexLength {
                index_group:  U32::new(index_group),
                index_offset: U32::new(index_offset),
                length:       U32::new(buffer.len() as u32),
            },
            res: ResultLength::new_zeroed(),
            rbuf: buffer
        }
    }

    /// Get the actual returned data.
    ///
    /// If the request returned an error, returns Err.
    pub fn data(&self) -> Result<&[u8]> {
        if self.res.result.get() != 0 {
            ads_error("multi-read data", self.res.result.get())
        } else {
            Ok(&self.rbuf[..self.res.length.get() as usize])
        }
    }
}

/// A single request for a [`Device::write_multi`] request.
pub struct WriteRequest<'buf> {
    req:  IndexLength,
    res:  U32<LE>,
    wbuf: &'buf [u8],
}

impl<'buf> WriteRequest<'buf> {
    /// Create the request with given index group, index offset and input buffer.
    pub fn new(index_group: u32, index_offset: u32, buffer: &'buf [u8]) -> Self {
        Self {
            req: IndexLength {
                index_group:  U32::new(index_group),
                index_offset: U32::new(index_offset),
                length:       U32::new(buffer.len() as u32),
            },
            res: U32::default(),
            wbuf: buffer
        }
    }

    /// Verify that the data was successfully written.
    ///
    /// If the request returned an error, returns Err.
    pub fn ensure(&self) -> Result<()> {
        if self.res.get() != 0 {
            ads_error("multi-write data", self.res.get())
        } else {
            Ok(())
        }
    }
}

/// A single request for a [`Device::write_read_multi`] request.
pub struct WriteReadRequest<'buf> {
    req:  IndexLengthRW,
    res:  ResultLength,
    wbuf: &'buf [u8],
    rbuf: &'buf mut [u8],
}

impl<'buf> WriteReadRequest<'buf> {
    /// Create the request with given index group, index offset and input and
    /// result buffers.
    pub fn new(index_group: u32, index_offset: u32, write_buffer: &'buf [u8],
               read_buffer: &'buf mut [u8]) -> Self {
        Self {
            req: IndexLengthRW {
                index_group:  U32::new(index_group),
                index_offset: U32::new(index_offset),
                read_length:  U32::new(read_buffer.len() as u32),
                write_length: U32::new(write_buffer.len() as u32),
            },
            res:  ResultLength::new_zeroed(),
            wbuf: write_buffer,
            rbuf: read_buffer,
        }
    }

    /// Get the actual returned data.
    ///
    /// If the request returned an error, returns Err.
    pub fn data(&self) -> Result<&[u8]> {
        if self.res.result.get() != 0 {
            ads_error("multi-read/write data", self.res.result.get())
        } else {
            Ok(&self.rbuf[..self.res.length.get() as usize])
        }
    }
}

// ADS notification intervals use 100 ns ticks on the wire, as emitted by
// Beckhoff AdsLib and measured against the vendor's symbolic server.
fn notification_ticks(duration: Duration) -> u128 {
    duration.as_nanos() / 100
}

/// A single request for a [`Device::add_notification_multi`] request.
pub struct AddNotifRequest {
    req:  AddNotif,
    res:  ResultLength, // length is the handle
}

impl AddNotifRequest {
    /// Create the request with given index group, index offset and notification
    /// attributes. Durations exceeding the 32-bit wire range are clamped.
    pub fn new(index_group: u32, index_offset: u32, attributes: &notif::Attributes) -> Self {
        Self {
            req: AddNotif {
                index_group:  U32::new(index_group),
                index_offset: U32::new(index_offset),
                length:       U32::new(attributes.length as u32),
                trans_mode:   U32::new(attributes.trans_mode as u32),
                max_delay:    U32::new(notification_ticks(attributes.max_delay).min(u32::MAX as u128) as u32),
                cycle_time:   U32::new(notification_ticks(attributes.cycle_time).min(u32::MAX as u128) as u32),
                reserved:     [0; 16],
            },
            res: ResultLength::new_zeroed(),
        }
    }

    /// Get the returned notification handle.
    ///
    /// If the request returned an error, returns Err.
    pub fn handle(&self) -> Result<notif::Handle> {
        if self.res.result.get() != 0 {
            ads_error("multi-read/write data", self.res.result.get())
        } else {
            Ok(self.res.length.get())
        }
    }
}

/// A single request for a [`Device::delete_notification_multi`] request.
pub struct DelNotifRequest {
    req:  U32<LE>,
    res:  U32<LE>,
}

impl DelNotifRequest {
    /// Create the request with given index group, index offset and notification
    /// attributes.
    pub fn new(handle: notif::Handle) -> Self {
        Self {
            req: U32::new(handle),
            res: U32::default(),
        }
    }

    /// Verify that the handle was successfully deleted.
    ///
    /// If the request returned an error, returns Err.
    pub fn ensure(&self) -> Result<()> {
        if self.res.get() != 0 {
            ads_error("multi-read/write data", self.res.get())
        } else {
            Ok(())
        }
    }
}


fn fixup_write_read_return_buffers(requests: &mut [WriteReadRequest]) {
    // Calculate the initial (using buffer sizes) and actual (using result
    // sizes) offsets of each request.
    let offsets = requests.iter().scan((0, 0), |(init_cum, act_cum), req| {
        let (init, act) = (req.rbuf.len(), req.res.length.get() as usize);
        let current = Some((*init_cum, *act_cum, init, act));
        assert!(init >= act);
        *init_cum += init;
        *act_cum += act;
        current
    }).collect_vec();

    // Go through the buffers in reverse order.
    for i in (0..requests.len()).rev() {
        let (my_initial, my_actual, _, mut size) = offsets[i];
        if size == 0 {
            continue;
        }
        if my_initial == my_actual {
            // Offsets match, no further action required since all
            // previous buffers must be of full length too.
            break;
        }

        // Check in which buffer our last byte is.
        let mut j = offsets[..i+1].iter().rposition(|r| r.0 < my_actual + size)
                                         .expect("index must be somewhere");
        let mut j_end = my_actual + size - offsets[j].0;

        // Copy the required number of bytes from every buffer from j up to i.
        loop {
            let n = j_end.min(size);
            size -= n;
            if i == j {
                requests[i].rbuf.copy_within(j_end-n..j_end, size);
            } else {
                let (first, second) = requests.split_at_mut(i);
                second[0].rbuf[size..][..n].copy_from_slice(&first[j].rbuf[j_end-n..j_end]);
            }
            if size == 0 {
                break;
            }
            j -= 1;
            j_end = offsets[j].2;
        }
    }
}

#[test]
fn test_fixup_buffers() {
    let mut buf0 = *b"12345678AB";
    let mut buf1 = *b"CDEFabc";
    let mut buf2 = *b"dxyUVW";
    let mut buf3 = *b"XYZY";
    let mut buf4 = *b"XW----";
    let mut buf5 = *b"-------------";
    let reqs = &mut [
        WriteReadRequest::new(0, 0, &[], &mut buf0),
        WriteReadRequest::new(0, 0, &[], &mut buf1),
        WriteReadRequest::new(0, 0, &[], &mut buf2),
        WriteReadRequest::new(0, 0, &[], &mut buf3),
        WriteReadRequest::new(0, 0, &[], &mut buf4),
        WriteReadRequest::new(0, 0, &[], &mut buf5),
    ];
    reqs[0].res.length.set(8);
    reqs[1].res.length.set(6);
    reqs[2].res.length.set(0);
    reqs[3].res.length.set(4);
    reqs[4].res.length.set(2);
    reqs[5].res.length.set(9);

    fixup_write_read_return_buffers(reqs);

    assert!(&reqs[5].rbuf[..9] == b"UVWXYZYXW");
    assert!(&reqs[4].rbuf[..2] == b"xy");
    assert!(&reqs[3].rbuf[..4] == b"abcd");
    assert!(&reqs[1].rbuf[..6] == b"ABCDEF");
    assert!(&reqs[0].rbuf[..8] == b"12345678");
}

#[test]
fn notification_intervals_use_100ns_ticks_without_overflow() {
    let attributes = notif::Attributes::new(4, notif::TransmissionMode::ServerCycle,
        Duration::from_millis(25), Duration::from_millis(200));
    let request = AddNotifRequest::new(0xF005, 42, &attributes);
    assert_eq!(&request.req.as_bytes()[16..20], &250_000u32.to_le_bytes());
    assert_eq!(&request.req.as_bytes()[20..24], &2_000_000u32.to_le_bytes());
    assert_eq!(notification_ticks(Duration::from_millis(300_000)), 3_000_000_000);
    assert!(u32::try_from(notification_ticks(Duration::from_secs(430))).is_err());
    let overflow = notif::Attributes::new(4, notif::TransmissionMode::ServerCycle,
        Duration::from_secs(u64::MAX), Duration::from_secs(u64::MAX));
    let request = AddNotifRequest::new(0xF005, 42, &overflow);
    assert_eq!(request.req.max_delay.get(), u32::MAX);
    assert_eq!(request.req.cycle_time.get(), u32::MAX);
}

#[test]
fn short_ams_headers_return_an_error_without_panicking() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; AMS_HEADER_SIZE];
        socket.read_exact(&mut request).unwrap();
        socket.write_all(&[0; TCP_HEADER_SIZE]).unwrap();
    });
    let client = Client::new(address, Timeouts::new(Duration::from_secs(1)), Source::Auto).unwrap();
    let error = client.device(crate::AmsAddr::new([1, 2, 3, 4, 1, 1].into(), 851))
        .get_state().unwrap_err();
    assert!(matches!(error, Error::Reply(_, "truncated AMS header", 0)));
    server.join().unwrap();
}

#[test]
fn short_router_control_packets_do_not_disrupt_ads_replies() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; AMS_HEADER_SIZE];
        socket.read_exact(&mut request).unwrap();
        for (command, length) in [(1u16, 2u32), (4096, 8), (4097, 0), (4098, 4)] {
            socket.write_all(&command.to_le_bytes()).unwrap();
            socket.write_all(&length.to_le_bytes()).unwrap();
            socket.write_all(&vec![0; length as usize]).unwrap();
        }
        let mut response = request.to_vec();
        response[6..14].copy_from_slice(&request[14..22]);
        response[14..22].copy_from_slice(&request[6..14]);
        response[2..6].copy_from_slice(&40u32.to_le_bytes());
        response[24..26].copy_from_slice(&5u16.to_le_bytes());
        response[26..30].copy_from_slice(&8u32.to_le_bytes());
        response.extend_from_slice(&[0, 0, 0, 0, 5, 0, 0x34, 0x12]);
        socket.write_all(&response).unwrap();
    });
    let client = Client::new(address, Timeouts::new(Duration::from_secs(1)), Source::Auto).unwrap();
    let state = client.device(crate::AmsAddr::new([1, 2, 3, 4, 1, 1].into(), 851))
        .get_state().unwrap();
    assert_eq!(state, (AdsState::Run, 0x1234));
    server.join().unwrap();
}

#[test]
fn packet_limit_rejects_giant_declarations_without_reading_the_payload() {
    let limit = 16 * 1024 * 1024;
    assert_eq!(packet_buffer_length(limit, limit).unwrap(), limit + TCP_HEADER_SIZE);
    assert!(packet_buffer_length(limit + 1, limit).is_err());
    assert!(packet_buffer_length(usize::MAX, usize::MAX).is_err());
    for command in [0u16, 4096] {
        for length in [(limit + 1) as u32, u32::MAX] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = [0; AMS_HEADER_SIZE];
                socket.read_exact(&mut request).unwrap();
                socket.write_all(&command.to_le_bytes()).unwrap();
                socket.write_all(&length.to_le_bytes()).unwrap();
                // No payload is sent: the length alone must reject the packet.
            });
            let client = Client::new_with_packet_limit(address,
                Timeouts::new(Duration::from_secs(1)), Source::Auto, limit).unwrap();
            let error = client.device(crate::AmsAddr::new([1, 2, 3, 4, 1, 1].into(), 851))
                .get_state().unwrap_err();
            assert!(matches!(error, Error::Reply(_, "packet exceeds configured limit", n) if n == length));
            server.join().unwrap();
        }
    }
}
