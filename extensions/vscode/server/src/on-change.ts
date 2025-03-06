import type {
    DidChangeWatchedFilesParams,
    TextDocumentChangeEvent,
} from 'vscode-languageserver'
import type { TextDocument } from 'vscode-languageserver-textdocument'
import type { Context } from './types'

// The content of a text document has changed. This event is emitted
// when the text document first opened or when its content has changed.
export function onChange(
    this: Context,
    change: TextDocumentChangeEvent<TextDocument>
) {
    this.validateTextDocument(change.document)
}

export function onChangeWatched(
    this: Context,
    _change: DidChangeWatchedFilesParams
) {
    // Monitored files have change in VSCode
    this.conn.console.log('We received a file change event')
}
