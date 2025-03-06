import type { CompletionItem } from 'vscode-languageserver'

// This handler resolves additional information for the item selected in
// the completion list.
export function onCompletionResolve(item: CompletionItem): CompletionItem {
    if (item.data === 1) {
        item.detail = 'TypeScript details'
        item.documentation = 'TypeScript documentation'
    } else if (item.data === 2) {
        item.detail = 'JavaScript details'
        item.documentation = 'JavaScript documentation'
    }
    return item
}
