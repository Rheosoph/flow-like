//! GenICam node-graph tests: a synthetic XML exercising every node type
//! against a MockPort, plus parse checks over real vendor XMLs (Hikrobot,
//! Imperx) from the GigeVision reference project.

use telegenic::GenicamError;
use telegenic::genicam::port::MockPort;
use telegenic::genicam::{AccessMode, parse_xml};

const SYNTHETIC: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<RegisterDescription ModelName="Test" VendorName="Test"
    xmlns="http://www.genicam.org/GenApi/Version_1_1">
  <Category Name="Root">
    <pFeature>Width</pFeature>
    <pFeature>ExposureTime</pFeature>
  </Category>

  <Integer Name="Width">
    <pValue>WidthReg</pValue>
    <Min>8</Min>
    <pMax>WidthMax</pMax>
    <Inc>4</Inc>
  </Integer>
  <IntReg Name="WidthReg">
    <Address>0x100</Address>
    <Length>4</Length>
    <AccessMode>RW</AccessMode>
    <pPort>Device</pPort>
    <Cachable>WriteThrough</Cachable>
    <Endianess>BigEndian</Endianess>
  </IntReg>
  <IntReg Name="WidthMax">
    <Address>0x104</Address>
    <Length>4</Length>
    <AccessMode>RO</AccessMode>
    <pPort>Device</pPort>
    <Endianess>BigEndian</Endianess>
  </IntReg>

  <Float Name="ExposureTime">
    <pValue>ExposureConv</pValue>
  </Float>
  <Converter Name="ExposureConv">
    <FormulaTo>FROM * 1000000</FormulaTo>
    <FormulaFrom>TO / 1000000.0</FormulaFrom>
    <pValue>ExposureReg</pValue>
  </Converter>
  <IntReg Name="ExposureReg">
    <Address>0x200</Address>
    <Length>4</Length>
    <AccessMode>RW</AccessMode>
    <pPort>Device</pPort>
    <Endianess>BigEndian</Endianess>
  </IntReg>

  <Enumeration Name="PixelFormat">
    <EnumEntry Name="Mono8"><Value>17301505</Value></EnumEntry>
    <EnumEntry Name="Mono12"><Value>17825797</Value></EnumEntry>
    <pValue>PixelFormatReg</pValue>
  </Enumeration>
  <IntReg Name="PixelFormatReg">
    <Address>0x300</Address>
    <Length>4</Length>
    <AccessMode>RW</AccessMode>
    <pPort>Device</pPort>
    <Endianess>BigEndian</Endianess>
  </IntReg>

  <Boolean Name="ReverseX">
    <pValue>ReverseXBit</pValue>
  </Boolean>
  <MaskedIntReg Name="ReverseXBit">
    <Address>0x400</Address>
    <Length>4</Length>
    <AccessMode>RW</AccessMode>
    <pPort>Device</pPort>
    <Endianess>BigEndian</Endianess>
    <Bit>31</Bit>
  </MaskedIntReg>

  <Command Name="AcquisitionStart">
    <pValue>AcqStartReg</pValue>
    <CommandValue>1</CommandValue>
  </Command>
  <IntReg Name="AcqStartReg">
    <Address>0x500</Address>
    <Length>4</Length>
    <AccessMode>WO</AccessMode>
    <pPort>Device</pPort>
    <Endianess>BigEndian</Endianess>
  </IntReg>

  <IntSwissKnife Name="PayloadSize">
    <pVariable Name="W">Width</pVariable>
    <pVariable Name="BPP">BytesPerPixel</pVariable>
    <Formula>W * 10 * BPP</Formula>
  </IntSwissKnife>
  <Integer Name="BytesPerPixel">
    <Value>1</Value>
  </Integer>

  <StringReg Name="DeviceUserID">
    <Address>0x600</Address>
    <Length>16</Length>
    <AccessMode>RW</AccessMode>
    <pPort>Device</pPort>
  </StringReg>

  <IntReg Name="CachedReg">
    <Address>0x700</Address>
    <Length>4</Length>
    <AccessMode>RO</AccessMode>
    <pPort>Device</pPort>
    <Cachable>WriteThrough</Cachable>
    <pInvalidator>WidthReg</pInvalidator>
    <Endianess>BigEndian</Endianess>
  </IntReg>

  <Port Name="Device"/>
</RegisterDescription>"#;

fn setup() -> (telegenic::genicam::Genicam, MockPort) {
    let graph = parse_xml(SYNTHETIC).expect("parse synthetic xml");
    let port = MockPort::new(0x1000);
    port.set_u32_be(0x100, 640); // Width
    port.set_u32_be(0x104, 4096); // WidthMax
    port.set_u32_be(0x200, 20_000); // Exposure µs
    port.set_u32_be(0x300, 17_301_505); // Mono8
    port.set_u32_be(0x400, 0); // ReverseX off
    port.set_u32_be(0x700, 42);
    (graph, port)
}

#[test]
fn integer_through_register() {
    let (mut g, port) = setup();
    let width = g.lookup("Width").unwrap();
    assert_eq!(g.int_value(width, &port).unwrap(), 640);
    assert_eq!(g.int_bounds(width, &port).unwrap(), (8, 4096));
    assert_eq!(g.int_increment(width, &port).unwrap(), 4);

    g.set_int_value(width, 800, &port).unwrap();
    assert_eq!(port.u32_be(0x100), 800);
    assert_eq!(g.int_value(width, &port).unwrap(), 800);
}

#[test]
fn converter_roundtrip() {
    let (mut g, port) = setup();
    let exposure = g.lookup("ExposureTime").unwrap();
    let v = g.float_value(exposure, &port).unwrap();
    assert!((v - 0.02).abs() < 1e-9, "20000µs = 0.02s, got {v}");

    g.set_float_value(exposure, 0.005, &port).unwrap();
    assert_eq!(port.u32_be(0x200), 5000);
}

#[test]
fn enumeration_by_name() {
    let (mut g, port) = setup();
    let pf = g.lookup("PixelFormat").unwrap();
    assert_eq!(g.string_value(pf, &port).unwrap(), "Mono8");
    assert_eq!(g.enum_entries(pf).unwrap(), ["Mono8", "Mono12"]);

    g.set_enum_entry(pf, "Mono12", &port).unwrap();
    assert_eq!(port.u32_be(0x300), 17_825_797);
    assert_eq!(g.string_value(pf, &port).unwrap(), "Mono12");

    let err = g.set_enum_entry(pf, "Nope", &port).unwrap_err();
    assert!(matches!(err, GenicamError::NoSuchEntry(..)));
}

#[test]
fn boolean_through_masked_bit() {
    let (mut g, port) = setup();
    let rx = g.lookup("ReverseX").unwrap();
    assert!(!g.bool_value(rx, &port).unwrap());

    g.set_bool_value(rx, true, &port).unwrap();
    // BE GenICam Bit 31 of a 4-byte register == conventional bit 0.
    assert_eq!(port.u32_be(0x400), 1);
    assert!(g.bool_value(rx, &port).unwrap());
}

#[test]
fn command_writes_command_value() {
    let (mut g, port) = setup();
    let start = g.lookup("AcquisitionStart").unwrap();
    g.execute(start, &port).unwrap();
    assert_eq!(port.u32_be(0x500), 1);

    let reg = g.lookup("AcqStartReg").unwrap();
    let err = g.int_value(reg, &port).unwrap_err();
    assert!(
        matches!(err, GenicamError::Access(_)),
        "WO register must not read"
    );
}

#[test]
fn swissknife_formula_over_variables() {
    let (mut g, port) = setup();
    let payload = g.lookup("PayloadSize").unwrap();
    // Width(640) * 10 * BytesPerPixel(1)
    assert_eq!(g.int_value(payload, &port).unwrap(), 6400);
}

#[test]
fn string_register_roundtrip() {
    let (mut g, port) = setup();
    let id = g.lookup("DeviceUserID").unwrap();
    g.set_string_value(id, "cam-7", &port).unwrap();
    assert_eq!(g.string_value(id, &port).unwrap(), "cam-7");
}

#[test]
fn cache_and_invalidation() {
    let (mut g, port) = setup();
    let cached = g.lookup("CachedReg").unwrap();
    assert_eq!(g.int_value(cached, &port).unwrap(), 42);

    // Behind the cache's back: no change visible.
    port.set_u32_be(0x700, 43);
    assert_eq!(g.int_value(cached, &port).unwrap(), 42);

    // Writing WidthReg invalidates CachedReg via pInvalidator.
    let width = g.lookup("Width").unwrap();
    g.set_int_value(width, 808, &port).unwrap();
    assert_eq!(g.int_value(cached, &port).unwrap(), 43);

    port.set_u32_be(0x700, 44);
    g.invalidate_caches();
    assert_eq!(g.int_value(cached, &port).unwrap(), 44);
}

#[test]
fn injected_defaults_present() {
    let (mut g, port) = setup();
    let tl = g.lookup("TLParamsLocked").unwrap();
    assert_eq!(g.int_value(tl, &port).unwrap(), 0);
    g.set_int_value(tl, 1, &port).unwrap();
    assert_eq!(g.int_value(tl, &port).unwrap(), 1);

    let scps = g.lookup("GevSCPSPacketSize").unwrap();
    port.set_u32_be(0xd04, 0xc000_05dc); // flags set, size 1500
    assert_eq!(g.int_value(scps, &port).unwrap(), 1500);
}

#[test]
fn access_modes() {
    let (g, _) = setup();
    assert_eq!(g.access_mode(g.lookup("Width").unwrap()), AccessMode::RW);
    assert_eq!(g.access_mode(g.lookup("WidthMax").unwrap()), AccessMode::RO);
    assert_eq!(
        g.access_mode(g.lookup("PayloadSize").unwrap()),
        AccessMode::RO
    );
    assert_eq!(
        g.access_mode(g.lookup("AcquisitionStart").unwrap()),
        AccessMode::WO
    );
}

#[test]
fn parses_hikrobot_xml() {
    let xml = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/Hikrobot.xml"
    ))
    .expect("read Hikrobot.xml");
    let graph = parse_xml(&xml).expect("parse Hikrobot");
    assert!(
        graph.len() > 1000,
        "expected thousands of nodes, got {}",
        graph.len()
    );
    for feature in [
        "Width",
        "Height",
        "PixelFormat",
        "ExposureTime",
        "Gain",
        "AcquisitionStart",
        "AcquisitionStop",
        "PayloadSize",
        "TLParamsLocked",
    ] {
        assert!(graph.lookup(feature).is_ok(), "missing {feature}");
    }
    let pf = graph.lookup("PixelFormat").unwrap();
    let entries = graph.enum_entries(pf).expect("PixelFormat entries");
    assert!(entries.iter().any(|e| e == "Mono8"), "entries: {entries:?}");
}

#[test]
fn parses_imperx_xml() {
    let xml = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/Imperx.xml"
    ))
    .expect("read Imperx.xml");
    let graph = parse_xml(&xml).expect("parse Imperx");
    assert!(graph.len() > 300, "got {}", graph.len());
    for feature in [
        "Width",
        "Height",
        "PixelFormat",
        "AcquisitionStart",
        "PayloadSize",
    ] {
        assert!(graph.lookup(feature).is_ok(), "missing {feature}");
    }
}

#[test]
fn imperx_width_reads_through_mock_registers() {
    let xml = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/Imperx.xml"
    ))
    .expect("read Imperx.xml");
    let mut graph = parse_xml(&xml).expect("parse Imperx");
    let port = MockPort::new(0x20000);
    // WidthReg is an IntReg at 0xD300, big-endian.
    port.set_u32_be(0xd300, 1024);
    let width = graph.lookup("Width").unwrap();
    assert_eq!(graph.int_value(width, &port).unwrap(), 1024);
}
