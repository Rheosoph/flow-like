//! The minimal GenICam device description the emulator serves. Every feature
//! bottoms out in a 4-byte, 4-aligned, big-endian register the GVCP server
//! backs, so each read/write maps to a single register transaction. `PayloadSize`
//! is an `IntSwissKnife` over `Width`/`Height` (mono8, one byte per pixel).

/// The `RegisterDescription` served (zipped) over READ_MEMORY.
pub const GENICAM_XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<RegisterDescription ModelName="TheaterCam" VendorName="Valstad"
    xmlns="http://www.genicam.org/GenApi/Version_1_1">
  <Integer Name="Width"><pValue>WidthReg</pValue><Min>8</Min><Max>8192</Max></Integer>
  <IntReg Name="WidthReg">
    <Address>0x2000</Address><Length>4</Length><AccessMode>RW</AccessMode>
    <pPort>Device</pPort><Endianess>BigEndian</Endianess>
  </IntReg>
  <Integer Name="Height"><pValue>HeightReg</pValue><Min>8</Min><Max>8192</Max></Integer>
  <IntReg Name="HeightReg">
    <Address>0x2004</Address><Length>4</Length><AccessMode>RW</AccessMode>
    <pPort>Device</pPort><Endianess>BigEndian</Endianess>
  </IntReg>
  <IntSwissKnife Name="PayloadSize">
    <pVariable Name="W">Width</pVariable>
    <pVariable Name="H">Height</pVariable>
    <Formula>W * H</Formula>
  </IntSwissKnife>
  <Command Name="AcquisitionStart">
    <pValue>AcqReg</pValue><CommandValue>1</CommandValue>
  </Command>
  <Command Name="AcquisitionStop">
    <pValue>AcqReg</pValue><CommandValue>0</CommandValue>
  </Command>
  <IntReg Name="AcqReg">
    <Address>0x2008</Address><Length>4</Length><AccessMode>RW</AccessMode>
    <pPort>Device</pPort><Endianess>BigEndian</Endianess>
  </IntReg>
  <Enumeration Name="AcquisitionMode">
    <EnumEntry Name="Continuous"><Value>0</Value></EnumEntry>
    <EnumEntry Name="SingleFrame"><Value>1</Value></EnumEntry>
    <pValue>AcqModeReg</pValue>
  </Enumeration>
  <IntReg Name="AcqModeReg">
    <Address>0x200C</Address><Length>4</Length><AccessMode>RW</AccessMode>
    <pPort>Device</pPort><Endianess>BigEndian</Endianess>
  </IntReg>
  <Enumeration Name="PixelFormat">
    <EnumEntry Name="Mono8"><Value>17301505</Value></EnumEntry>
    <pValue>PixelFormatReg</pValue>
  </Enumeration>
  <IntReg Name="PixelFormatReg">
    <Address>0x2010</Address><Length>4</Length><AccessMode>RW</AccessMode>
    <pPort>Device</pPort><Endianess>BigEndian</Endianess>
  </IntReg>
  <Float Name="ExposureTime"><pValue>ExposureReg</pValue></Float>
  <FloatReg Name="ExposureReg">
    <Address>0x2014</Address><Length>4</Length><AccessMode>RW</AccessMode>
    <pPort>Device</pPort><Endianess>BigEndian</Endianess>
  </FloatReg>
  <Float Name="Gain"><pValue>GainReg</pValue></Float>
  <FloatReg Name="GainReg">
    <Address>0x2018</Address><Length>4</Length><AccessMode>RW</AccessMode>
    <pPort>Device</pPort><Endianess>BigEndian</Endianess>
  </FloatReg>
  <Port Name="Device"/>
</RegisterDescription>"#;
