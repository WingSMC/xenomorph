import {
    CompletionItemKind,
    type CompletionItem,
    type TextDocumentPositionParams,
} from 'vscode-languageserver'

export function onCompletion(
    _textDocumentPosition: TextDocumentPositionParams
): CompletionItem[] {
    // The pass parameter contains the position of the text document in
    // which code complete got requested. For the example we ignore this
    // info and always provide the same completion items.
    return [
        {
            label: 'TypeScript',
            kind: CompletionItemKind.Text,
            data: 1,
        },
        {
            label: 'JavaScript',
            kind: CompletionItemKind.Text,
            data: 2,
        },
    ]
}
