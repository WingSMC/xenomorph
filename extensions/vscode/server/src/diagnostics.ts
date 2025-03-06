import {
    DocumentDiagnosticReportKind,
    type DocumentDiagnosticParams,
    type DocumentDiagnosticReport,
} from 'vscode-languageserver'
import type { Context } from './types'
import { validateTextDocument } from './validate-document'

export async function diagnostics(
    this: Context,
    params: DocumentDiagnosticParams
) {
    const document = this.documents.get(params.textDocument.uri)
    if (document !== undefined) {
        return {
            kind: DocumentDiagnosticReportKind.Full,
            items: await validateTextDocument.bind(this)(document),
        } satisfies DocumentDiagnosticReport
    } else {
        // We don't know the document. We can either try to read it from disk
        // or we don't report problems for it.
        return {
            kind: DocumentDiagnosticReportKind.Full,
            items: [],
        } satisfies DocumentDiagnosticReport
    }
}
