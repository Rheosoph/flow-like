use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use fake::{Fake, faker::lorem::en::*};

#[crate::register_node]
#[derive(Default)]
pub struct FakeWord;

#[async_trait]
impl NodeLogic for FakeWord {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_word",
            "Fake Word",
            "Generates a random lorem ipsum word for mocking data",
            "Utils/Faker/Lorem",
        );
        node.set_flowscript_name("faker.lorem", "word");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin("word", "Word", "Generated word", VariableType::String);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let word: String = Word().fake();
        context.set_pin_value("word", json!(word)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FakeWords;

#[async_trait]
impl NodeLogic for FakeWords {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_words",
            "Fake Words",
            "Generates random lorem ipsum words for mocking data",
            "Utils/Faker/Lorem",
        );
        node.set_flowscript_name("faker.lorem", "words");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "min_count",
            "Min Count",
            "Minimum number of words",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(3)));
        node.add_input_pin(
            "max_count",
            "Max Count",
            "Maximum number of words",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(6)));
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "words",
            "Words",
            "Generated words as array",
            VariableType::Generic,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let min: i64 = context.evaluate_pin("min_count").await?;
        let max: i64 = context.evaluate_pin("max_count").await?;
        let words: Vec<String> = Words(min as usize..max as usize).fake();
        context.set_pin_value("words", json!(words)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FakeSentence;

#[async_trait]
impl NodeLogic for FakeSentence {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_sentence",
            "Fake Sentence",
            "Generates a random lorem ipsum sentence for mocking data",
            "Utils/Faker/Lorem",
        );
        node.set_flowscript_name("faker.lorem", "sentence");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "min_words",
            "Min Words",
            "Minimum words in sentence",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(4)));
        node.add_input_pin(
            "max_words",
            "Max Words",
            "Maximum words in sentence",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "sentence",
            "Sentence",
            "Generated sentence",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let min: i64 = context.evaluate_pin("min_words").await?;
        let max: i64 = context.evaluate_pin("max_words").await?;
        let sentence: String = Sentence(min as usize..max as usize).fake();
        context.set_pin_value("sentence", json!(sentence)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FakeSentences;

#[async_trait]
impl NodeLogic for FakeSentences {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_sentences",
            "Fake Sentences",
            "Generates random lorem ipsum sentences for mocking data",
            "Utils/Faker/Lorem",
        );
        node.set_flowscript_name("faker.lorem", "sentences");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "min_count",
            "Min Count",
            "Minimum number of sentences",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(2)));
        node.add_input_pin(
            "max_count",
            "Max Count",
            "Maximum number of sentences",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5)));
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "sentences",
            "Sentences",
            "Generated sentences as array",
            VariableType::Generic,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let min: i64 = context.evaluate_pin("min_count").await?;
        let max: i64 = context.evaluate_pin("max_count").await?;
        let sentences: Vec<String> = Sentences(min as usize..max as usize).fake();
        context.set_pin_value("sentences", json!(sentences)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FakeParagraph;

#[async_trait]
impl NodeLogic for FakeParagraph {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_paragraph",
            "Fake Paragraph",
            "Generates a random lorem ipsum paragraph for mocking data",
            "Utils/Faker/Lorem",
        );
        node.set_flowscript_name("faker.lorem", "paragraph");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "min_sentences",
            "Min Sentences",
            "Minimum sentences in paragraph",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(3)));
        node.add_input_pin(
            "max_sentences",
            "Max Sentences",
            "Maximum sentences in paragraph",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(7)));
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "paragraph",
            "Paragraph",
            "Generated paragraph",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let min: i64 = context.evaluate_pin("min_sentences").await?;
        let max: i64 = context.evaluate_pin("max_sentences").await?;
        let paragraph: String = Paragraph(min as usize..max as usize).fake();
        context.set_pin_value("paragraph", json!(paragraph)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FakeParagraphs;

#[async_trait]
impl NodeLogic for FakeParagraphs {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_paragraphs",
            "Fake Paragraphs",
            "Generates random lorem ipsum paragraphs for mocking data",
            "Utils/Faker/Lorem",
        );
        node.set_flowscript_name("faker.lorem", "paragraphs");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "min_count",
            "Min Count",
            "Minimum number of paragraphs",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(2)));
        node.add_input_pin(
            "max_count",
            "Max Count",
            "Maximum number of paragraphs",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(4)));
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "paragraphs",
            "Paragraphs",
            "Generated paragraphs as array",
            VariableType::Generic,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let min: i64 = context.evaluate_pin("min_count").await?;
        let max: i64 = context.evaluate_pin("max_count").await?;
        let paragraphs: Vec<String> = Paragraphs(min as usize..max as usize).fake();
        context
            .set_pin_value("paragraphs", json!(paragraphs))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}
