use flow_like_model_provider::{
    text_splitter::{MarkdownSplitter, TextSplitter},
    tokenizer::TokenizerSizer,
};

pub fn split_text(
    text: &str,
    splitter: Option<&TextSplitter<TokenizerSizer>>,
    md_splitter: Option<&MarkdownSplitter<TokenizerSizer>>,
) -> Vec<String> {
    if let Some(md_splitter) = md_splitter {
        md_splitter
            .chunks(text)
            .map(|item| item.to_string())
            .collect()
    } else if let Some(splitter) = splitter {
        splitter.chunks(text).map(|item| item.to_string()).collect()
    } else {
        println!("No splitter found");
        vec![]
    }
}
