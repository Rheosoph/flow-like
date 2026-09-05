// use lopdf::Document;
// use std::fs::File;
// use std::path::PathBuf;

// use super::hash::hash_bytes;

// #[derive(Debug)]
// pub struct Page {
//     pub number: u32,
//     pub text: String,
//     pub images: Vec<String>,
//     pub reference: String,
// }

// pub fn extract_pdf(file_path: &PathBuf) -> flow_like_types::Result<Vec<Page>> {
//     let parent = file_path.parent().unwrap();
//     let pdf_name = file_path.file_stem().unwrap().to_str().unwrap();
//     let image_dir = parent.join(pdf_name);
//     let mut pages: Vec<Page> = vec![];
//     let doc = Document::load(file_path)?;
//     for (page_num, page_id) in doc.get_pages() {
//         let mut page_content = Page {
//             number: page_num,
//             text: "".to_string(),
//             images: vec![],
//             reference: file_path.to_str().unwrap().to_string(),
//         };

//         let images = match doc.get_page_images(page_id) {
//             Ok(images) => images,
//             Err(e) => {
//                 println!("Error extracting images from page: {:?}", e);
//                 vec![]
//             }
//         };

//         let text = match doc.extract_text(&[page_num]) {
//             Ok(text) => text,
//             Err(e) => {
//                 println!("Error extracting text from page: {:?}", e);
//                 "".to_string()
//             }
//         };
//         page_content.text = text;

//         if images.is_empty() {
//             pages.push(page_content);
//             continue;
//         }

//         let page_dir = image_dir.join(format!("page_{}", page_num));
//         if !page_dir.exists() {
//             std::fs::create_dir_all(&page_dir)?;
//         }

//         for image in images.iter() {
//             let content = image.content;
//             let img = match image::load_from_memory(content) {
//                 Ok(img) => img,
//                 Err(e) => {
//                     println!("Error loading image: {:?}", e);
//                     continue;
//                 }
//             };
//             let hash = hash_bytes(content);
//             let image_path = page_dir.join(format!("image_{}.png", hash));
//             let mut file = File::create(&image_path)?;
//             match img.write_to(&mut file, image::ImageFormat::Png) {
//                 Ok(_) => {
//                     page_content
//                         .images
//                         .push(image_path.to_str().unwrap().to_string());
//                 }
//                 Err(e) => {
//                     println!("Error writing image: {:?}", e);
//                 }
//             };
//         }

//         pages.push(page_content);
//     }

//     Ok(pages)
// }

// #[cfg(test)]
// mod tests {
//     use std::io::Write;

//     use flow_like_types::{reqwest, tokio};

//     use super::*;

//     #[tokio::test]
//     async fn parse_pdf() -> flow_like_types::Result<()> {
//         if std::env::var("CI").is_ok() {
//             eprintln!("Skipping parse_pdf in CI");
//             return Ok(());
//         }

//         let download_link = "https://de.wikipedia.org/api/rest_v1/page/pdf/BMW";
//         let pdf_path = PathBuf::from("./tmp/BMW.pdf");

//         if let Some(dir) = pdf_path.parent() {
//             std::fs::create_dir_all(dir)?;
//         }

//         let client = reqwest::Client::builder()
//             .user_agent("flow-like-tests/0.1 (+https://github.com/your-org/flow-like)")
//             .build()?;

//         let response = client
//             .get(download_link)
//             .header(reqwest::header::ACCEPT, "application/pdf")
//             .send()
//             .await?
//             .error_for_status()?;

//         let bytes = response.bytes().await?;
//         let mut file = File::create(&pdf_path)?;
//         file.write_all(&bytes)?;

//         let pdf = extract_pdf(&pdf_path)?;
//         assert!(!pdf.is_empty());
//         assert_ne!(pdf.first().unwrap().text, "");

//         Ok(())
//     }
// }
