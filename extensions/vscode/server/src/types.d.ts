import type {
    Connection,
    Diagnostic,
    TextDocuments,
} from 'vscode-languageserver'
import type { TextDocument } from 'vscode-languageserver-textdocument'

export type Context = {
    hasWorkspaceFolderCapability: boolean
    hasConfigurationCapability: boolean
    hasDiagnosticRelatedInformationCapability: boolean
    globalSettings: ExampleSettings
    documentSettings: Map<string, Thenable<ExampleSettings>>
    conn: Connection
    documents: TextDocuments<TextDocument>
    getDocSettings: (resource: string) => Thenable<ExampleSettings>
    validateTextDocument: (document: TextDocument) => Promise<Diagnostic[]>
}

export interface ExampleSettings {
    maxNumberOfProblems: number
}
