//! The GenICam XML URL register format:
//! `Local:Filename.zip;Address;Size` (hex, no 0x prefix), `File:path`, or
//! `http://...`. Schemes are case-insensitive; real devices emit any casing.

#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum XmlUrl {
    /// The file lives in device memory at `address`.
    Local {
        filename: String,
        address: u64,
        size: usize,
    },
    File(String),
    Http(String),
}

impl XmlUrl {
    pub fn parse(url: &str) -> Result<Self, String> {
        let url = url.trim().trim_matches(char::from(0));
        let lower = url.to_ascii_lowercase();
        if let Some(rest) = strip_scheme(url, &lower, "local:") {
            let mut parts = rest.split(';');
            let filename = parts.next().unwrap_or("").trim().to_string();
            let address = parts.next().ok_or("missing address")?.trim();
            let size = parts.next().ok_or("missing size")?.trim();
            let address =
                u64::from_str_radix(address, 16).map_err(|e| format!("bad address: {e}"))?;
            let size = usize::from_str_radix(size, 16).map_err(|e| format!("bad size: {e}"))?;
            Ok(Self::Local {
                filename,
                address,
                size,
            })
        } else if let Some(rest) = strip_scheme(url, &lower, "file:") {
            // Allow both "File:C:\x.xml" and "file:///path/x.xml".
            Ok(Self::File(rest.trim_start_matches("//").to_string()))
        } else if lower.starts_with("http://") || lower.starts_with("https://") {
            Ok(Self::Http(url.to_string()))
        } else {
            Err(format!("unrecognized GenICam URL '{url}'"))
        }
    }

    /// `true` when the referenced file is a PKZIP archive.
    pub fn is_zip(&self) -> bool {
        let name = match self {
            Self::Local { filename, .. } => filename.as_str(),
            Self::File(path) => path.as_str(),
            Self::Http(url) => url.as_str(),
        };
        name.to_ascii_lowercase().ends_with(".zip")
    }
}

fn strip_scheme<'a>(url: &'a str, lower: &str, scheme: &str) -> Option<&'a str> {
    lower.starts_with(scheme).then(|| &url[scheme.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_local_zip() {
        let url =
            XmlUrl::parse("local:OptoEngineering_Itala_GEV_v3.2.1.zip;FE800000;D967").unwrap();
        assert_eq!(
            url,
            XmlUrl::Local {
                filename: "OptoEngineering_Itala_GEV_v3.2.1.zip".into(),
                address: 0xFE80_0000,
                size: 0xD967,
            }
        );
        assert!(url.is_zip());
    }

    #[test]
    fn parses_uppercase_local_xml() {
        let url = XmlUrl::parse("Local:camera.xml;10000;1A2B\0\0\0").unwrap();
        assert_eq!(
            url,
            XmlUrl::Local {
                filename: "camera.xml".into(),
                address: 0x10000,
                size: 0x1A2B
            }
        );
        assert!(!url.is_zip());
    }

    #[test]
    fn parses_file_and_http() {
        assert_eq!(
            XmlUrl::parse("File:///tmp/cam.xml").unwrap(),
            XmlUrl::File("/tmp/cam.xml".into())
        );
        assert!(matches!(
            XmlUrl::parse("http://cam/genicam.xml").unwrap(),
            XmlUrl::Http(_)
        ));
        assert!(XmlUrl::parse("gopher:whatever").is_err());
    }
}
