// use std::{
//     collections::HashSet,
//     path::{Path, PathBuf},
//     process::Stdio,
// };

// use flow_like_types::tokio::{self, io::BufReader};

// use crate::utils::hash::hash_file;
// pub struct Pandoc {}

// impl Default for Pandoc {
//     fn default() -> Self {
//         Self::new()
//     }
// }

// impl Pandoc {
//     pub fn new() -> Pandoc {
//         Pandoc {}
//     }

//     pub async fn transform_to_markdown(
//         &self,
//         source: &PathBuf,
//         source_hash: Option<String>,
//         target_dir: &Path,
//     ) -> flow_like_types::Result<PathBuf> {
//         if !Pandoc::file_valid(source) {
//             return Err(flow_like_types::anyhow!(
//                 "Source file is not a valid pandoc supported format"
//             ));
//         }

//         let source_hash = match source_hash {
//             Some(hash) => hash,
//             None => hash_file(source),
//         };

//         let target_file = target_dir.join(format!("{}.md", source_hash));
//         let target_image_dir = target_dir.join(&source_hash);

//         if !target_image_dir.exists() {
//             std::fs::create_dir_all(&target_image_dir)?;
//         }

//         let program = PathBuf::from("pandoc");
//         let mut sidecar = crate::utils::execute::async_sidecar(&program).await?;
//         let target_image_dir_arg =
//             format!("--extract-media={}", target_image_dir.to_str().unwrap());
//         let args = vec![
//             source.to_str().unwrap(),
//             "-o",
//             target_file.to_str().unwrap(),
//             "-t",
//             "gfm",
//             &target_image_dir_arg,
//         ];
//         let mut child = sidecar
//             .args(args)
//             .stdout(Stdio::piped())
//             .stderr(Stdio::piped())
//             .spawn()?;

//         let stdout = child.stdout.take().expect("Failed to capture stdout");
//         let stderr = child.stderr.take().expect("Failed to capture stderr");

//         // Wrap stdout and stderr in buffered readers.
//         let stdout_reader = BufReader::new(stdout);
//         let stderr_reader = BufReader::new(stderr);

//         let mut stdout_lines = stdout_reader.lines();
//         let mut stderr_lines = stderr_reader.lines();

//         tokio::spawn(async move {
//             while let Some(line) = stdout_lines.next_line().await.unwrap_or(None) {
//                 println!("[PANDOC] stdout: {}", line);
//             }
//         });

//         tokio::spawn(async move {
//             while let Some(line) = stderr_lines.next_line().await.unwrap_or(None) {
//                 eprintln!("[PANDOC ERROR] stderr: {}", line);
//             }
//         });

//         let status = child.wait().await?;
//         println!("[PANDOC] Child process exited with: {}", status);

//         Ok(target_file)
//     }

//     pub fn file_valid(file: &Path) -> bool {
//         let mut supported_extensions = HashSet::new();
//         supported_extensions.insert("markdown".to_string());
//         supported_extensions.insert("mdown".to_string());
//         supported_extensions.insert("mkdn".to_string());
//         supported_extensions.insert("mkd".to_string());
//         supported_extensions.insert("md".to_string());
//         supported_extensions.insert("commonmark".to_string());

//         supported_extensions.insert("rst".to_string());
//         supported_extensions.insert("org".to_string());
//         supported_extensions.insert("t2t".to_string());

//         supported_extensions.insert("tex".to_string());
//         supported_extensions.insert("latex".to_string());
//         supported_extensions.insert("ltx".to_string());

//         supported_extensions.insert("html".to_string());
//         supported_extensions.insert("htm".to_string());

//         supported_extensions.insert("docbook".to_string());
//         supported_extensions.insert("docx".to_string());
//         supported_extensions.insert("odt".to_string());
//         supported_extensions.insert("epub".to_string());
//         supported_extensions.insert("fb2".to_string());
//         supported_extensions.insert("icml".to_string());
//         supported_extensions.insert("jats".to_string());
//         supported_extensions.insert("ipynb".to_string());
//         supported_extensions.insert("json".to_string());
//         supported_extensions.insert("markdown_strict".to_string());
//         supported_extensions.insert("mediawiki".to_string());
//         supported_extensions.insert("man".to_string());
//         supported_extensions.insert("ms".to_string());
//         supported_extensions.insert("muse".to_string());
//         supported_extensions.insert("native".to_string());
//         supported_extensions.insert("opml".to_string());
//         supported_extensions.insert("pptx".to_string());
//         supported_extensions.insert("revealjs".to_string());
//         supported_extensions.insert("rtf".to_string());
//         supported_extensions.insert("s5".to_string());
//         supported_extensions.insert("slidy".to_string());
//         supported_extensions.insert("texinfo".to_string());
//         supported_extensions.insert("textile".to_string());

//         let extension = match file.extension() {
//             Some(ext) => ext.to_str().unwrap().to_string(),
//             None => "".to_string(),
//         };

//         let extension = extension.to_lowercase();

//         supported_extensions.contains(&extension)
//     }
// }
