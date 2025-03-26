import { join } from 'node:path'
import { ExtensionContext } from 'vscode'
import {
    Executable,
    LanguageClient,
    LanguageClientOptions,
} from 'vscode-languageclient/node'

function createServerOptions(context: ExtensionContext): {
    run: Executable
    debug: Executable
} {
    const command = context.asAbsolutePath(join('server', 'xenomorph_lsp'))
    const options: Executable['options'] = {}
    const args = ['--plugins', '../../../target/debug/']
    return {
        run: { command, options, args },
        debug: { command, options, args },
    }
}

const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: 'file', language: 'xenomorph' }],
    //synchronize: {
    //    fileEvents: workspace.createFileSystemWatcher('**/.clientrc'),
    //},
}

let client: LanguageClient | undefined
export function activate(context: ExtensionContext) {
    client = new LanguageClient(
        'xenomorph_language_client',
        'Xenomorph Language Client',
        createServerOptions(context),
        clientOptions
    )

    client.start()
}

export function deactivate() {
    return client?.stop()
}
