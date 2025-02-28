use serde::{Deserialize, Serialize};
use lsp_types::{
    CompletionItemKind, InsertTextFormat, TextEdit, Command, MarkupContent,
};
use serde_json::Value;

/// A type that represents a completion token with rich semantic data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionToken {
    /// The label of the completion item (display text).
    pub label: String,
    /// The kind of completion item (e.g., variable, function, etc.).
    pub kind: Option<CompletionItemKind>,
    /// A short description for the completion item.
    pub detail: Option<String>,
    /// Detailed documentation for the item, possibly in Markdown or plaintext.
    pub documentation: Option<MarkupContent>,
    /// Indicates if the item is deprecated.
    pub deprecated: Option<bool>,
    /// If true, this item is preselected when the completion list is shown.
    pub preselect: Option<bool>,
    /// Text used for sorting the completion items.
    pub sort_text: Option<String>,
    /// Text used for filtering the completion items.
    pub filter_text: Option<String>,
    /// The text that should be inserted when the item is picked.
    pub insert_text: Option<String>,
    /// Indicates whether the insert text is plain text or a snippet.
    pub insert_text_format: Option<InsertTextFormat>,
    /// The text edit to be applied to the document on selecting this item.
    pub text_edit: Option<TextEdit>,
    /// Additional text edits to be applied, which must not overlap with the main edit.
    pub additional_text_edits: Option<Vec<TextEdit>>,
    /// Optional command to run after inserting the completion.
    pub command: Option<Command>,
    /// Characters that, when typed, will confirm the completion.
    pub commit_characters: Option<Vec<String>>,
    /// A field to hold any extra data useful for resolving the completion item.
    pub data: Option<Value>,
}
