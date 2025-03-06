import {
    DidChangeConfigurationNotification,
    type InitializedParams,
} from 'vscode-languageserver'
import type { Context } from './types'

export function onInitialized(this: Context, _e: InitializedParams) {
    if (this.hasConfigurationCapability) {
        this.conn.client.register(
            DidChangeConfigurationNotification.type,
            undefined
        )
    }
    if (this.hasWorkspaceFolderCapability) {
        this.conn.workspace.onDidChangeWorkspaceFolders((_event) => {
            this.conn.console.log('Workspace folder change event received.')
        })
    }
}
