use std::{fs, io, path::Path};

use crate::{frontend_payload, ios_frontend};

pub const INSTALL_MARKER: &str = "editors/install.marker";

pub fn write_if_changed(path: &Path, contents: &[u8]) -> io::Result<()> {
    if fs::read(path).ok().as_deref() != Some(contents) {
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| io::Error::other("missing parent"))?,
        )?;
        fs::write(path, contents)?;
    }
    Ok(())
}

/// Keep editor URLs intact while moving their WASM bytes into an Android feature.
pub fn stage(
    source: &Path,
    embedded: &Path,
    module_assets: &Path,
) -> io::Result<ios_frontend::StagedFrontend> {
    let staged = ios_frontend::stage_selected(
        source,
        embedded,
        &module_assets.join(frontend_payload::RESOURCE_DIR),
        |path| {
            path.to_str()
                .is_some_and(|path| frontend_payload::android_editor_mime_type(path).is_some())
        },
    )?;
    write_if_changed(
        &module_assets.join(INSTALL_MARKER),
        b"flow-like-editors-v1\n",
    )?;
    Ok(staged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "flow-like-android-frontend-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("out")).unwrap();
            Self(root)
        }

        fn write(&self, path: &str, contents: &[u8]) {
            let file = self.0.join("out").join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, contents).unwrap();
        }

        fn stage(&self) -> ios_frontend::StagedFrontend {
            stage(
                &self.0.join("out"),
                &self.0.join("embedded"),
                &self.0.join("moduleAssets"),
            )
            .unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn moves_editor_chunks_and_preserves_all_other_assets_and_licenses() {
        let fixture = Fixture::new();
        let files = [
            (
                "craft/pdfcraft/craft.wasm.gz.000",
                b"first".as_slice(),
                true,
            ),
            ("craft/pdfcraft/craft.wasm.gz.001", b"second", true),
            ("wordcraft/wordcraft.wasm.gz.000", b"word", true),
            ("craft/pdfcraft/runtime.json", b"{}", false),
            (
                "craft/pdfcraft/host.js",
                b"fetch('craft.wasm.gz.000')",
                false,
            ),
            (
                "craft/pdfcraft/licenses/sources/krilla.tar.gz",
                b"archive",
                false,
            ),
            ("wordcraft/licenses/LICENSE", b"license", false),
            ("board_canvas.wasm", b"board", false),
            ("ort-wasm-simd-threaded.wasm", b"ort", false),
            ("index.html", b"<html>app</html>", false),
        ];
        for (path, bytes, _) in files {
            fixture.write(path, bytes);
        }
        let staged = fixture.stage();
        assert_eq!(staged.bundled_bytes, 15);
        assert_eq!(
            staged.bundled_paths,
            files
                .iter()
                .filter(|file| file.2)
                .map(|file| file.0)
                .collect::<Vec<_>>()
        );
        for (path, bytes, bundled) in files {
            let embedded = fixture.0.join("embedded").join(path);
            let resource = fixture.0.join("moduleAssets/frontend-wasm").join(path);
            let (kept, omitted) = if bundled {
                (resource, embedded)
            } else {
                (embedded, resource)
            };
            assert_eq!(fs::read(kept).unwrap(), bytes);
            assert!(!omitted.exists(), "{path}");
            assert_eq!(fs::read(fixture.0.join("out").join(path)).unwrap(), bytes);
        }
        assert!(
            fixture
                .0
                .join("moduleAssets")
                .join(INSTALL_MARKER)
                .is_file()
        );
    }

    #[test]
    fn refreshes_stale_chunks_and_restores_assets_after_gradle_clean() {
        let fixture = Fixture::new();
        let chunk = "craft/pdfcraft/craft.wasm.gz.000";
        let stale = "craft/pdfcraft/craft.wasm.gz.001";
        fixture.write(chunk, b"old");
        fixture.write(stale, b"stale");
        fixture.stage();
        fixture.write(chunk, b"new");
        fs::remove_file(fixture.0.join("out").join(stale)).unwrap();
        fixture.stage();
        let resources = fixture.0.join("moduleAssets/frontend-wasm");
        assert_eq!(fs::read(resources.join(chunk)).unwrap(), b"new");
        assert!(!resources.join(stale).exists());
        let marker = fixture.0.join("moduleAssets").join(INSTALL_MARKER);
        let before = fs::metadata(&marker).unwrap().modified().unwrap();
        let chunk_before = fs::metadata(resources.join(chunk))
            .unwrap()
            .modified()
            .unwrap();
        fixture.stage();
        assert_eq!(fs::metadata(&marker).unwrap().modified().unwrap(), before);
        assert_eq!(
            fs::metadata(resources.join(chunk))
                .unwrap()
                .modified()
                .unwrap(),
            chunk_before
        );
        fs::remove_dir_all(fixture.0.join("moduleAssets")).unwrap();
        fixture.stage();
        assert_eq!(fs::read(resources.join(chunk)).unwrap(), b"new");
        assert!(marker.is_file());
    }
}
