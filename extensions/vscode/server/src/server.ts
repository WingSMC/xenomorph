import { TextDocument } from 'vscode-languageserver-textdocument'
import {
    createConnection,
    ProposedFeatures,
    TextDocuments,
} from 'vscode-languageserver/node'

import { DEFAULT_SETTINGS } from './config'
import { diagnostics } from './diagnostics'
import { getDocumentSettings } from './getDocumentSettings'
import { init } from './init'
import { onChange, onChangeWatched } from './on-change'
import { onClose } from './on-close'
import { onCompletion } from './on-completion'
import { onCompletionResolve } from './on-completion-resolve'
import { onConfigChange } from './on-config-change'
import { onInitialized } from './on-initialized'
import type { Context, ExampleSettings } from './types'
import { validateTextDocument } from './validate-document'

const CTX: Context = {} as Context
CTX.hasConfigurationCapability = false
CTX.hasWorkspaceFolderCapability = false
CTX.hasDiagnosticRelatedInformationCapability = false
CTX.globalSettings = DEFAULT_SETTINGS
CTX.documentSettings = new Map<string, Thenable<ExampleSettings>>()
CTX.conn = createConnection(ProposedFeatures.all)
CTX.documents = new TextDocuments(TextDocument)
CTX.getDocSettings = getDocumentSettings.bind(CTX)
CTX.validateTextDocument = validateTextDocument.bind(CTX)

CTX.conn.onInitialize(init.bind(CTX))
CTX.conn.onInitialized(onInitialized.bind(CTX))
CTX.conn.onDidChangeConfiguration(onConfigChange.bind(CTX))
CTX.conn.onDidChangeWatchedFiles(onChangeWatched.bind(CTX))
CTX.conn.onCompletion(onCompletion)
CTX.conn.onCompletionResolve(onCompletionResolve)
CTX.conn.languages.diagnostics.on(diagnostics.bind(CTX))

CTX.documents.onDidClose(onClose.bind(CTX))
CTX.documents.onDidChangeContent(onChange.bind(CTX))
CTX.documents.listen(CTX.conn)

CTX.conn.listen()
