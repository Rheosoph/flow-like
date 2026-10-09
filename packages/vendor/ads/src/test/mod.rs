// Code used in the crate test suite.

use std::convert::{TryFrom, TryInto};
use std::io::{Read, Write};
use std::mem::size_of;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use byteorder::{ByteOrder, ReadBytesExt, WriteBytesExt, LE};
use once_cell::sync::Lazy;
use zerocopy::{
    byteorder::{U32, U64},
    AsBytes, FromBytes, FromZeroes,
};

use crate::client::{AddNotif, AdsHeader, IndexLength, IndexLengthRW};
use crate::{file, index};

// Test modules.
mod test_client;
mod test_netid;
mod test_udp;

// Since Cargo tests run multi-threaded, start one server per thread and
// handle clients from the test functions in that thread.
thread_local! {
    pub static SERVER: Lazy<(u16, Arc<Mutex<ServerOpts>>)> = Lazy::new(|| {
        let opts = Arc::new(Mutex::new(ServerOpts::default()));

        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        let opts_server = opts.clone();
        thread::spawn(move || {
            let mut server = Server {
                opts: opts_server,
                state: (crate::AdsState::Run, 0),
                data: vec![0; 1024],
                file_ptr: None,
                notif: None,
            };
            for client in socket.incoming().flatten() {
                // We only need to handle one client concurrently.
                server.handle_client(client);
            }
        });

        (port, opts)
    });
}

// Configures different ways the server should behave.
#[derive(Default)]
pub struct ServerOpts {
    pub timeout: Option<Duration>,
    pub no_reply: bool,
    pub garbage_header: bool,
    pub bad_notif: bool,
    pub ignore_invokeid: bool,
}

pub fn config_test_server(opts: ServerOpts) -> u16 {
    SERVER.with(|obj| {
        let (port, server_opts) = &**obj;
        *server_opts.lock().unwrap() = opts;
        *port
    })
}

struct Server {
    opts: Arc<Mutex<ServerOpts>>,
    data: Vec<u8>,
    // If the test file is opened for writing, and the read/write position.
    file_ptr: Option<(bool, usize)>,
    // If a notification has been added, the (offset, size) to send.
    notif: Option<(usize, usize)>,
    // The simulated device state.
    state: (crate::AdsState, u16),
}

impl Server {
    fn handle_client(&mut self, mut socket: TcpStream) {
        let opts = self.opts.clone();
        loop {
            let opts = opts.lock().unwrap();
            let mut header = AdsHeader::new_zeroed();
            if let Err(e) = socket.read_exact(header.as_bytes_mut()) {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    // connection was closed
                    return;
                }
                panic!("unexpected receive error: {}", e);
            }
            println!(">>> {:?}", header);
            let mut data = vec![0; header.data_length.get() as usize];
            socket.read_exact(&mut data).unwrap();

            if opts.no_reply {
                return;
            }

            let (reply_data, error) = match header.command.get() {
                1 => self.do_devinfo(&data),
                2 => self.do_read(&data),
                3 => self.do_write(&data),
                4 => self.do_read_state(&data),
                5 => self.do_write_control(&data),
                6 => self.do_add_notif(&data),
                7 => self.do_del_notif(&data),
                9 => self.do_read_write(&data),
                _ => (vec![], 0x701),
            };

            // Generate a notification if they are enabled.
            if let Some((off, len)) = &self.notif {
                self.send_notification(*off, *len, &header, opts.bad_notif, &mut socket);
            }

            let mut reply_header = AdsHeader::new_zeroed();
            if opts.garbage_header {
                reply_header.ams_cmd = 234;
            }
            reply_header.length.set(32 + reply_data.len() as u32);
            reply_header.dest_netid = header.src_netid;
            reply_header.dest_port = header.src_port;
            reply_header.src_netid = header.dest_netid;
            reply_header.src_port = header.dest_port;
            reply_header.command = header.command;
            reply_header.state_flags.set(header.state_flags.get() | 1);
            reply_header.data_length.set(reply_data.len() as u32);
            reply_header.error_code.set(error);
            if !opts.ignore_invokeid {
                reply_header.invoke_id = header.invoke_id;
            }
            println!("<<< {:?}", reply_header);

            socket.write_all(reply_header.as_bytes()).unwrap();
            socket.write_all(&reply_data).unwrap();
        }
    }

    fn send_notification(&self, off: usize, len: usize, header: &AdsHeader,
                         bad: bool, socket: &mut TcpStream) {
        let data_len = std::mem::size_of::<SingleNotification>() + len;

        let mut notif_header = SingleNotification::default();
        notif_header.len.set(data_len as u32 - 4);
        notif_header.stamps.set(if bad { u32::MAX } else { 1 });
        notif_header.stamp.set(0x9988776655443322);
        notif_header.samples.set(1);
        notif_header.handle.set(132);
        notif_header.size.set(len as u32);

        let mut ads_header = AdsHeader::new_zeroed();
        ads_header.length.set(32 + data_len as u32);
        ads_header.dest_netid = header.src_netid;
        ads_header.dest_port = header.src_port;
        ads_header.src_netid = header.dest_netid;
        ads_header.src_port = header.dest_port;
        ads_header.command.set(crate::client::Command::Notification as u16);
        ads_header.state_flags.set(4);
        ads_header.data_length.set(data_len as u32);
        println!("not: {:?}", ads_header);

        socket.write_all(ads_header.as_bytes()).unwrap();
        socket.write_all(notif_header.as_bytes()).unwrap();
        socket.write_all(&self.data[off..][..len]).unwrap();
    }

    fn do_devinfo(&self, data: &[u8]) -> (Vec<u8>, u32) {
        if !data.is_empty() {
            return (vec![], 0x706);
        }
        // no error, major 7, minor 1
        let mut out = 0u32.to_le_bytes().to_vec();
        out.write_u8(7).unwrap();
        out.write_u8(1).unwrap();
        out.write_u16::<LE>(4024).unwrap();
        out.extend(b"Nice device\0\0\0\0\0");
        (out, 0)
    }

    fn do_read_state(&self, data: &[u8]) -> (Vec<u8>, u32) {
        if !data.is_empty() {
            return (vec![], 0x706);
        }
        let mut out = 0u32.to_le_bytes().to_vec();
        out.write_u16::<LE>(self.state.0 as u16).unwrap();
        out.write_u16::<LE>(self.state.1).unwrap();
        (out, 0)
    }

    fn do_write_control(&mut self, mut data: &[u8]) -> (Vec<u8>, u32) {
        if data.len() != 8 {
            return (vec![], 0x706);
        }
        let adsstate = data.read_u16::<LE>().unwrap();
        let devstate = data.read_u16::<LE>().unwrap();
        let mut out = vec![];
        match crate::AdsState::try_from(adsstate) {
            Err(_) | Ok(crate::AdsState::Invalid) => {
                out.write_u32::<LE>(0x70B).unwrap();
            }
            Ok(adsstate) => {
                self.state = (adsstate, devstate);
                out.write_u32::<LE>(0).unwrap();
            }
        }
        (out, 0)
    }

    fn do_read(&self, data: &[u8]) -> (Vec<u8>, u32) {
        if data.len() != size_of::<IndexLength>() {
            return (vec![], 0x706);
        }
        let request = IndexLength::read_from(data).unwrap();
        let grp = request.index_group.get();
        let mut off = request.index_offset.get() as usize;
        let len = request.length.get() as usize;
        let mut out = Vec::new();
        out.write_u32::<LE>(0).unwrap();
        // Simulate symbol access.
        if grp == index::RW_SYMVAL_BYHANDLE {
            if off != 77 {
                return (vec![], 0x710);
            }
            off = 1020; // symbol lives at the end of self.data
        } else if grp != index::PLC_RW_M {
            return (vec![], 0x702);
        }
        if off + len > self.data.len() {
            return (vec![], 0x703);
        }
        out.write_u32::<LE>(request.length.get()).unwrap();
        out.extend(&self.data[off..][..len]);
        (out, 0)
    }

    fn do_write(&mut self, data: &[u8]) -> (Vec<u8>, u32) {
        if data.len() < size_of::<IndexLength>() {
            return (vec![], 0x706);
        }
        let request = IndexLength::read_from(&data[..12]).unwrap();
        let grp = request.index_group.get();
        let mut off = request.index_offset.get() as usize;
        let len = request.length.get() as usize;

        if grp == index::RW_SYMVAL_BYHANDLE {
            if off != 77 {
                return (vec![], 0x710);
            }
            off = 1020;
        } else if grp == index::RELEASE_SYMHANDLE {
            if off != 77 {
                return (vec![], 0x710);
            }
            return (0u32.to_le_bytes().into(), 0);
        } else if grp != index::PLC_RW_M {
            return (vec![], 0x702);
        }
        if off + len > self.data.len() {
            return (vec![], 0x703);
        }
        if data.len() != size_of::<IndexLength>() + len {
            return (vec![], 0x706);
        }
        self.data[off..][..len].copy_from_slice(&data[12..]);
        (0u32.to_le_bytes().into(), 0)
    }

    fn do_read_write(&mut self, data: &[u8]) -> (Vec<u8>, u32) {
        if data.len() < size_of::<IndexLengthRW>() {
            return (vec![], 0x706);
        }
        let request = IndexLengthRW::read_from(&data[..16]).unwrap();
        let off = request.index_offset.get();
        let rlen = request.read_length.get() as usize;
        let wlen = request.write_length.get() as usize;
        let mut out = 0u32.to_le_bytes().to_vec();

        // Simulate file and symbol access.
        match request.index_group.get() {
            index::SUMUP_READ_EX => {
                let mut mdata: Vec<u8> = vec![];
                let mut rdata: Vec<u8> = vec![];
                for i in 0..off as usize {
                    let rlen = LE::read_u32(&data[16 + i*12 + 8..]) as usize;
                    let (mut d, e) = self.do_read(&data[16 + i*12..][..12]);
                    mdata.write_u32::<LE>(e).unwrap();
                    d.resize(rlen + 8, 0);
                    mdata.write_u32::<LE>(d.len() as u32 - 8).unwrap();
                    rdata.extend(&d[8..]);
                }
                out.write_u32::<LE>((mdata.len() + rdata.len()) as u32).unwrap();
                out.extend(mdata);
                out.extend(rdata);
            }
            index::SUMUP_WRITE => {
                let mut woff = 16 + off as usize*12;
                out.write_u32::<LE>(4 * off).unwrap();
                for i in 0..off as usize {
                    let wlen = LE::read_u32(&data[16 + i*12 + 8..]) as usize;
                    let mut subdata = data[16 + i*12..][..12].to_vec();
                    subdata.extend(&data[woff..][..wlen]);
                    woff += wlen;
                    let (_, e) = self.do_write(&subdata);
                    out.write_u32::<LE>(e).unwrap();
                }
            }
            index::SUMUP_READWRITE => {
                let mut mdata: Vec<u8> = vec![];
                let mut rdata: Vec<u8> = vec![];
                let mut woff = 16 + off as usize*16;
                for i in 0..off as usize {
                    let wlen = LE::read_u32(&data[16 + i*16 + 12..]) as usize;
                    let mut subdata = data[16 + i*16..][..16].to_vec();
                    subdata.extend(&data[woff..][..wlen]);
                    woff += wlen;
                    let (d, e) = self.do_read_write(&subdata);
                    mdata.write_u32::<LE>(e).unwrap();
                    if d.len() > 8 {
                        mdata.write_u32::<LE>(d.len() as u32 - 8).unwrap();
                        rdata.extend(&d[8..]);
                    } else {
                        mdata.write_u32::<LE>(0).unwrap();
                    }
                }
                out.write_u32::<LE>((mdata.len() + rdata.len()) as u32).unwrap();
                out.extend(mdata);
                out.extend(rdata);
            }
            index::SUMUP_ADDDEVNOTE => {
                out.write_u32::<LE>(8 * off).unwrap();
                for i in 0..off as usize {
                    let (d, e) = self.do_add_notif(&data[16 + i*40..][..40]);
                    out.write_u32::<LE>(e).unwrap();
                    if d.len() > 4 {
                        out.extend(&d[4..]);
                    } else {
                        out.write_u32::<LE>(0).unwrap();
                    }
                }
            }
            index::SUMUP_DELDEVNOTE => {
                out.write_u32::<LE>(4 * off).unwrap();
                for i in 0..off as usize {
                    let (_, e) = self.do_del_notif(&data[16 + i*4..][..4]);
                    out.write_u32::<LE>(e).unwrap();
                }
            }
            index::FILE_OPEN => {
                if &data[16..] != b"/etc/passwd" {
                    return (vec![], 0x70C);
                }
                if self.file_ptr.is_some() {
                    return (vec![], 0x708);
                }
                let write = off & (file::WRITE | file::APPEND) != 0;
                out.write_u32::<LE>(4).unwrap();
                out.write_u32::<LE>(42).unwrap();
                self.file_ptr = Some((write, 0));
            }
            index::FILE_CLOSE => {
                if !data[16..].is_empty() {
                    return (vec![], 0x70B);
                }
                if off != 42 {
                    return (vec![], 0x70C);
                }
                out.write_u32::<LE>(0).unwrap();
                self.file_ptr = None;
            }
            index::FILE_WRITE => {
                if let Some((true, ptr)) = &mut self.file_ptr {
                    out.write_u32::<LE>(0).unwrap();
                    *ptr += wlen;
                } else {
                    return (vec![], 0x704);
                }
            }
            index::FILE_READ => {
                if let Some((false, ptr)) = &mut self.file_ptr {
                    let cur = *ptr;
                    *ptr = (*ptr + rlen).min(888);
                    let amount = *ptr - cur;
                    out.write_u32::<LE>(amount as u32).unwrap();
                    out.resize(out.len() + amount, 0);
                } else {
                    return (vec![], 0x704);
                }
            }
            index::FILE_DELETE => {
                if &data[16..] != b"/etc/passwd" {
                    return (vec![], 0x70C);
                }
                if self.file_ptr.is_some() {
                    // send an unknown error number
                    return (vec![], 0xFFFF);
                }
                out.write_u32::<LE>(0).unwrap();
            }
            index::GET_SYMHANDLE_BYNAME => {
                if &data[16..] != b"SYMBOL" {
                    return (vec![], 0x710);
                }
                out.write_u32::<LE>(4).unwrap();
                out.write_u32::<LE>(77).unwrap();
            }
            _ => return (vec![], 0x702),
        }
        (out, 0)
    }

    fn do_add_notif(&mut self, data: &[u8]) -> (Vec<u8>, u32) {
        if data.len() != size_of::<AddNotif>() {
            return (vec![], 0x706);
        }
        let request = AddNotif::read_from(data).unwrap();
        let off = request.index_offset.get() as usize;
        let len = request.length.get() as usize;

        if request.index_group.get() != index::PLC_RW_M {
            return (vec![], 0x702);
        }
        if off + len > self.data.len() {
            return (vec![], 0x703);
        }
        self.notif = Some((off, len));
        let mut out = 0u32.to_le_bytes().to_vec();
        out.write_u32::<LE>(132).unwrap(); // handle
        (out, 0)
    }

    fn do_del_notif(&mut self, data: &[u8]) -> (Vec<u8>, u32) {
        if data.len() != 4 {
            return (vec![], 0x706);
        }
        let handle = u32::from_le_bytes(data.try_into().unwrap());
        if handle != 132 {
            return (vec![], 0x714);
        }
        if self.notif.is_none() {
            return (vec![], 0x714);
        }
        self.notif = None;
        (0u32.to_le_bytes().into(), 0)
    }
}

#[derive(AsBytes, FromBytes, FromZeroes, Debug, Default)]
#[repr(C)]
struct SingleNotification {
    len:     U32<LE>,
    stamps:  U32<LE>,
    stamp:   U64<LE>,
    samples: U32<LE>,
    handle:  U32<LE>,
    size:    U32<LE>,
}
