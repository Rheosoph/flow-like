// use ignore::WalkBuilder;
// use serde::{Deserialize, Serialize};
// use std::{
//     collections::{HashMap, HashSet},
//     path::PathBuf,
//     time::SystemTime,
// };

// use crate::utils::compression::{compress_to_file, from_compressed};

// #[derive(Serialize, Deserialize, Clone, Debug)]
// pub struct FileWatcher {
//     pub cache_dir: PathBuf,
//     pub watch_targets: Vec<PathBuf>,
//     pub last_state: HashMap<PathBuf, FileWatchEntry>,
//     pub hash_lookup: HashSet<String>,
//     pub current_state: HashSet<PathBuf>,
// }

// impl FileWatcher {
//     pub async fn new(cache_dir: PathBuf, watch_targets: &[PathBuf]) -> FileWatcher {
//         let mut deduped_targets: HashSet<PathBuf> = HashSet::new();
//         for target in watch_targets.iter() {
//             if let Ok(target) = target.canonicalize() {
//                 deduped_targets.insert(target);
//             }
//         }
//         let watch_targets = deduped_targets.into_iter().collect();
//         let manifest_path = cache_dir.join(".watch.json.gz");
//         if manifest_path.exists() {
//             if let Ok(mut watch) = from_compressed::<FileWatcher, _>(manifest_path) {
//                 watch.cache_dir = cache_dir;
//                 watch.watch_targets = watch_targets;
//                 watch.build_hash_lookup();
//                 return watch;
//             }
//         }
//         FileWatcher {
//             cache_dir,
//             watch_targets,
//             hash_lookup: HashSet::new(),
//             last_state: HashMap::new(),
//             current_state: HashSet::new(),
//         }
//     }

//     pub fn get_new(&mut self) -> Vec<PathBuf> {
//         let mut new_files = vec![];
//         for entry in self.current_state.iter() {
//             if !self.last_state.contains_key(entry) {
//                 new_files.push(entry.clone());
//             }
//         }
//         new_files
//     }

//     pub fn get_changed(&mut self) -> Vec<FileWatchEntry> {
//         let mut changed_files = vec![];
//         for entry in self.current_state.iter() {
//             if let Some(entry) = self.last_state.get_mut(entry) {
//                 if entry.was_modified() {
//                     changed_files.push(entry.clone());
//                 }
//             }
//         }
//         changed_files
//     }

//     pub fn get_deleted(&mut self) -> Vec<FileWatchEntry> {
//         let mut deleted_files = vec![];
//         for (path, entry) in self.last_state.iter() {
//             if !self.current_state.contains(path) {
//                 deleted_files.push(entry.clone());
//             }
//         }
//         deleted_files
//     }

//     // returns false if the file is a duplicate of the current state => if it is already indexed somewhere else
//     pub fn resolve(&mut self, file: &FileWatchEntry, remove: bool) -> bool {
//         if let Ok(path) = file.file_path.canonicalize() {
//             self.current_state.remove(&path);
//             if remove {
//                 self.last_state.remove(&path);
//                 self.build_hash_lookup();
//                 return true;
//             }

//             if self.is_duplicate(file) {
//                 return false;
//             }

//             self.last_state.insert(path, file.clone());
//             self.build_hash_lookup();
//             return true;
//         }

//         false
//     }

//     pub fn get_duplicates(&self, file: &FileWatchEntry) -> Vec<FileWatchEntry> {
//         let mut duplicates = vec![];
//         for (_, entry) in self.last_state.iter() {
//             if entry.last_hash == file.last_hash {
//                 duplicates.push(entry.clone());
//             }
//         }
//         duplicates
//     }

//     pub fn is_duplicate(&self, file: &FileWatchEntry) -> bool {
//         self.hash_lookup.contains(&file.last_hash)
//     }

//     pub fn build_hash_lookup(&mut self) {
//         self.hash_lookup.clear();
//         for (_, entry) in self.last_state.iter() {
//             self.hash_lookup.insert(entry.last_hash.clone());
//         }
//     }

//     pub async fn save_state(&mut self) {
//         self.current_state.clear();
//         self.hash_lookup.clear();
//         let target_dir = self.cache_dir.join(".watch.json.gz");
//         let parent = target_dir.parent().unwrap();
//         if !parent.exists() {
//             std::fs::create_dir_all(parent).unwrap();
//         }
//         match compress_to_file(self, target_dir) {
//             Ok(_) => {}
//             Err(e) => {
//                 println!("Error saving state: {}", e);
//             }
//         }

//         println!(
//             "Saved State to {}",
//             self.cache_dir.join(".watch.json.gz").to_str().unwrap()
//         );
//         self.build_hash_lookup();
//     }

//     pub async fn build_state(&mut self) -> Vec<PathBuf> {
//         let mut state: Vec<PathBuf> = Vec::new();

//         // Process watch targets in parallel for efficiency
//         let new_paths: Vec<PathBuf> = self
//             .watch_targets
//             .iter()
//             .flat_map(|target| {
//                 let mut target_paths: Vec<PathBuf> = Vec::new();

//                 // If it's a file, push it directly to the state
//                 if target.is_file() {
//                     target_paths.push(target.clone());
//                 } else if target.is_dir() {
//                     // Walk through directories and add files to the state
//                     let dirs: Vec<PathBuf> = WalkBuilder::new(target)
//                         .git_ignore(true)
//                         .hidden(true)
//                         .ignore(true)
//                         .build()
//                         .filter_map(|entry| {
//                             if let Ok(entry) = entry {
//                                 let path = entry.into_path();
//                                 if path.is_file() {
//                                     return path.canonicalize().ok();
//                                 }
//                             }
//                             None
//                         })
//                         .collect();

//                     target_paths.extend(dirs); // Add all the valid paths from the directory walk
//                 }

//                 target_paths
//             })
//             .collect();

//         state.extend(new_paths); // Add all new paths to the state

//         // Update current state
//         self.current_state = HashSet::from_iter(state.iter().cloned());

//         state
//     }
// }

// #[derive(Serialize, Deserialize, Clone, Debug)]
// pub struct FileWatchEntry {
//     pub file_path: PathBuf,
//     pub last_modified: SystemTime,
//     pub last_hash: String,
// }

// impl FileWatchEntry {
//     pub fn new(file_path: &PathBuf) -> FileWatchEntry {
//         let last_modified = std::fs::metadata(file_path).unwrap().modified().unwrap();
//         let last_hash = crate::utils::hash::hash_file(file_path);
//         FileWatchEntry {
//             file_path: file_path.canonicalize().unwrap().clone(),
//             last_modified,
//             last_hash,
//         }
//     }

//     pub fn was_modified(&self) -> bool {
//         let new_modified = std::fs::metadata(&self.file_path)
//             .unwrap()
//             .modified()
//             .unwrap();
//         if new_modified != self.last_modified {
//             let new_hash = crate::utils::hash::hash_file(&self.file_path);
//             if new_hash != self.last_hash {
//                 return true;
//             }
//         }
//         false
//     }
// }

// #[cfg(test)]
// mod tests {
//     use std::{fs, io::Write};

//     use super::*;

//     use serde::Deserialize;

//     fn create_file(path: PathBuf, content: &str) {
//         let mut file = std::fs::File::create(&path).unwrap();
//         file.write_all(content.as_bytes()).unwrap();
//     }

//     #[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
//     struct TestStruct {
//         id: i32,
//         name: String,
//         vector: Vec<f32>,
//     }

//     #[tokio::test]
//     async fn test_watch_creation() -> flow_like_types::Result<()> {
//         let test_path = PathBuf::from(format!("./tmp/{}", cuid2::create_id()));
//         let cache_path = PathBuf::from(format!("./tmp/{}", cuid2::create_id()));
//         std::fs::create_dir_all(&test_path).unwrap();
//         std::fs::create_dir_all(&cache_path).unwrap();

//         let mut file_watcher = FileWatcher::new(cache_path, &[test_path.clone()]).await;

//         create_file(test_path.join(cuid2::create_id()), "File 1");
//         create_file(test_path.join(cuid2::create_id()), "File 2");
//         create_file(test_path.join(cuid2::create_id()), "File 3");

//         let state = file_watcher.build_state().await;

//         assert_eq!(state.len(), 3);

//         Ok(())
//     }

//     #[tokio::test]
//     async fn test_watch_new_changed_deleted() -> flow_like_types::Result<()> {
//         let test_path = PathBuf::from(format!("./tmp/{}", cuid2::create_id()));
//         let cache_path = PathBuf::from(format!("./tmp/{}", cuid2::create_id()));
//         std::fs::create_dir_all(&test_path).unwrap();
//         std::fs::create_dir_all(&cache_path).unwrap();

//         let mut file_watcher = FileWatcher::new(cache_path.clone(), &[test_path.clone()]).await;

//         create_file(test_path.join("Changed"), "File 4");
//         create_file(test_path.join("Deleted"), "File 5");
//         create_file(test_path.join(cuid2::create_id()), "File 1");
//         create_file(test_path.join(cuid2::create_id()), "File 2");
//         create_file(test_path.join(cuid2::create_id()), "File 3");

//         let state = file_watcher.build_state().await;
//         for file in state.iter() {
//             let entry = FileWatchEntry::new(file);
//             file_watcher.resolve(&entry, false);
//         }

//         assert_eq!(state.len(), 5);

//         file_watcher.save_state().await;

//         // wait for 5 seconds
//         tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

//         create_file(test_path.join("Changed"), "File 4 -> 8");
//         create_file(test_path.join("New"), "New File 1");
//         fs::remove_file(test_path.join("Deleted"))?;

//         let mut file_watcher = FileWatcher::new(cache_path, &[test_path.clone()]).await;
//         let state = file_watcher.build_state().await;

//         assert_eq!(file_watcher.last_state.len(), 5);
//         assert_eq!(state.len(), 5);

//         let new_files = file_watcher.get_new();
//         let changed_files = file_watcher.get_changed();
//         let deleted_files = file_watcher.get_deleted();

//         assert_eq!(new_files.len(), 1);
//         assert_eq!(
//             new_files.first().unwrap().clone(),
//             test_path.join("New").canonicalize().unwrap()
//         );
//         assert_eq!(changed_files.len(), 1);
//         assert_eq!(
//             changed_files.first().unwrap().file_path,
//             test_path.join("Changed").canonicalize().unwrap()
//         );
//         assert_eq!(deleted_files.len(), 1);

//         Ok(())
//     }
// }
