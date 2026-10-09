// Tests for the AmsNetId/AmsAddr types.

use std::net::Ipv4Addr;
use std::str::FromStr;

use crate::netid::*;

#[test]
fn test_netid() {
    let netid = AmsNetId::new(5, 123, 8, 9, 1, 1);
    assert!(!netid.is_zero());
    assert_eq!(netid.to_string(), "5.123.8.9.1.1");

    assert_eq!(netid, AmsNetId::from_ip(Ipv4Addr::new(5, 123, 8, 9), 1, 1));

    assert_eq!(netid, AmsNetId::from_slice(&[5, 123, 8, 9, 1, 1]).unwrap());
    assert_eq!(netid, AmsNetId::from([5, 123, 8, 9, 1, 1]));
    assert!(AmsNetId::from_slice(&[0; 8]).is_none());

    assert_eq!(netid, AmsNetId::from_str("5.123.8.9.1.1").unwrap());
    assert_eq!(netid, AmsNetId::from_str("5.123.8.9").unwrap());
    assert!(AmsNetId::from_str("256.123.8.9.1.1").is_err());
    assert!(AmsNetId::from_str("blah").is_err());
}

#[test]
fn test_addr() {
    let netid = AmsNetId::new(5, 123, 8, 9, 1, 1);
    let addr = AmsAddr::new(netid, 851);
    assert_eq!(addr.netid(), netid);
    assert_eq!(addr.port(), 851);
    assert_eq!(addr.to_string(), "5.123.8.9.1.1:851");

    let mut buf = vec![];
    addr.write_to(&mut buf).unwrap();
    assert_eq!(buf, [5, 123, 8, 9, 1, 1, 0x53, 3]);

    assert_eq!(AmsAddr::read_from(&mut buf.as_slice()).unwrap(), addr);

    assert_eq!(addr, AmsAddr::from_str("5.123.8.9:851").unwrap());
    assert!(AmsAddr::from_str("5.123.8.9.1.1:88851").is_err());
    assert!(AmsAddr::from_str("256.123.8.9.1.1:851").is_err());
    assert!(AmsAddr::from_str("5.123.8.9.1.1").is_err());
    assert!(AmsAddr::from_str(":88851").is_err());
    assert!(AmsAddr::from_str("blah").is_err());
    assert!(AmsAddr::from_str("").is_err());
}
