import type { TextDocumentChangeEvent } from 'vscode-languageserver'
import type { TextDocument } from 'vscode-languageserver-textdocument'
import type { Context } from './types'

// Only keep settings for open documents
export function onClose(
    this: Context,
    e: TextDocumentChangeEvent<TextDocument>
) {
    this.documentSettings.delete(e.document.uri)
}
