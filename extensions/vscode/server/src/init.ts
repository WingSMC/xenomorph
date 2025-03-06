import {
    TextDocumentSyncKind,
    type CancellationToken,
    type InitializeParams,
    type InitializeResult,
    type ResultProgressReporter,
    type WorkDoneProgressReporter,
} from 'vscode-languageserver'
import type { Context } from './types'

export function init(
    this: Context,
    params: InitializeParams,
    _token: CancellationToken,
    _workDoneProgress: WorkDoneProgressReporter,
    _resultProgress?: ResultProgressReporter<never> | undefined
) {
    const capabilities = params.capabilities

    // Does the client support the `workspace/configuration` request?
    // If not, we fall back using global settings.
    this.hasConfigurationCapability = !!(
        capabilities.workspace && !!capabilities.workspace.configuration
    )
    this.hasWorkspaceFolderCapability = !!(
        capabilities.workspace && !!capabilities.workspace.workspaceFolders
    )
    this.hasDiagnosticRelatedInformationCapability = !!(
        capabilities.textDocument &&
        capabilities.textDocument.publishDiagnostics &&
        capabilities.textDocument.publishDiagnostics.relatedInformation
    )

    const result: InitializeResult = {
        capabilities: {
            textDocumentSync: TextDocumentSyncKind.Incremental,
            // Tell the client that this server supports code completion.
            completionProvider: {
                resolveProvider: true,
            },
            diagnosticProvider: {
                interFileDependencies: false,
                workspaceDiagnostics: false,
            },
        },
    }
    if (this.hasWorkspaceFolderCapability) {
        result.capabilities.workspace = {
            workspaceFolders: {
                supported: true,
            },
        }
    }
    return result
}
