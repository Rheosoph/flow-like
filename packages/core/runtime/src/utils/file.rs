// use id3::TagLike;
// use ignore::WalkBuilder;
// use pdf::file::FileOptions;
// use rayon::prelude::*;
// use schemars::JsonSchema;
// use serde::{Deserialize, Serialize};
// use std::ffi::OsStr;
// use std::{fs, path::PathBuf, time::SystemTime};

// #[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default)]
// pub struct FileMetadata {
//     // General file information
//     pub file_name: String,
//     pub file_path: PathBuf,
//     pub file_extension: String,
//     pub file_size: u64,
//     pub mime_type: String,

//     // Timestamps
//     pub creation_time: Option<SystemTime>,
//     pub modification_time: Option<SystemTime>,
//     pub access_time: Option<SystemTime>,
// }

// impl FileMetadata {
//     pub fn new(file_path: &PathBuf) -> Self {
//         let metadata = fs::metadata(file_path).unwrap();
//         let mime_type = mime_guess::from_path(file_path).first_or_octet_stream();

//         let mut file_metadata = FileMetadata {
//             file_name: file_path
//                 .file_name()
//                 .unwrap()
//                 .to_string_lossy()
//                 .into_owned(),
//             file_path: file_path.canonicalize().unwrap_or(file_path.clone()),
//             file_size: metadata.len(),
//             file_extension: file_path
//                 .extension()
//                 .unwrap_or(OsStr::new(""))
//                 .to_str()
//                 .unwrap()
//                 .to_string(),
//             mime_type: mime_type.to_string(),
//             creation_time: metadata.created().ok(),
//             modification_time: metadata.modified().ok(),
//             access_time: metadata.accessed().ok(),
//             ..Default::default()
//         };

//         if file_metadata.file_extension.to_lowercase() == "pdf" {
//             file_metadata = file_metadata.from_pdf();
//         }

//         if mime_type.type_() == mime_guess::mime::IMAGE {
//             file_metadata = file_metadata.from_img();
//         }

//         if mime_type.type_() == mime_guess::mime::AUDIO {
//             file_metadata = file_metadata.from_audio();
//         }

//         if mime_type.type_() == mime_guess::mime::VIDEO {
//             file_metadata = file_metadata.from_mp4();
//         }

//         file_metadata
//     }

//     pub fn from_folder(folder_path: &PathBuf) -> Vec<FileMetadata> {
//         WalkBuilder::new(folder_path)
//             .git_ignore(true)
//             .hidden(true)
//             .ignore(true)
//             .build()
//             .par_bridge()
//             .filter_map(|result| {
//                 result.ok().and_then(|entry| {
//                     let path = entry.path().to_path_buf();
//                     if path.is_file() {
//                         Some(FileMetadata::new(&path))
//                     } else {
//                         None
//                     }
//                 })
//             })
//             .collect::<Vec<FileMetadata>>()
//     }

//     fn from_pdf(&mut self) -> Self {
//         let doc = FileOptions::cached().open(&self.file_path).ok();
//         if let Some(doc) = doc {
//             if let Some(ref info) = doc.trailer.info_dict {
//                 if let Some(author) = &info.author {
//                     self.author = author.to_string().ok();
//                 };

//                 if let Some(creator) = &info.creator {
//                     self.creator = creator.to_string().ok();
//                 };

//                 if let Some(producer) = &info.producer {
//                     self.producer = producer.to_string().ok();
//                 };

//                 if let Some(title) = &info.title {
//                     self.title = title.to_string().ok();
//                 };

//                 if let Some(subject) = &info.subject {
//                     self.subject = subject.to_string().ok();
//                 };

//                 if let Some(keywords) = &info.keywords {
//                     self.keywords = keywords.to_string().ok();
//                 };
//             }

//             self.pages = Some(doc.pages().count() as u32);
//         }

//         self.clone()
//     }

//     fn from_img(&mut self) -> Self {
//         self.clone()
//     }

//     fn from_audio(&mut self) -> Self {
//         if let Ok(tag) = id3::Tag::read_from_path(&self.file_path) {
//             self.artist = tag.artist().map(|s| s.to_string());
//             self.album = tag.album().map(|s| s.to_string());
//             self.track_title = tag.title().map(|s| s.to_string());
//             self.genre = tag.genre().map(|s| s.to_string());
//             self.year = tag.year();
//             self.track_number = tag.track();
//             self.duration = tag.duration();
//         }

//         self.clone()
//     }

//     fn from_mp4(&mut self) -> Self {
//         if let Ok(tag) = mp4ameta::Tag::read_from_path(&self.file_path) {
//             self.artist = tag.artist().map(|s| s.to_string());
//             self.album = tag.album().map(|s| s.to_string());
//             self.track_title = tag.title().map(|s| s.to_string());
//             self.genre = tag.genre().map(|s| s.to_string());
//             self.keywords = Some(
//                 tag.keywords()
//                     .map(|s| s.to_string())
//                     .collect::<Vec<String>>()
//                     .join(", "),
//             );
//         }
//         self.clone()
//     }
// }
