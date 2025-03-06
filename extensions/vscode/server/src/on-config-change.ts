import type { DidChangeConfigurationParams } from 'vscode-languageserver'
import { DEFAULT_SETTINGS } from './config'
import type { Context } from './types'

export function onConfigChange(
    this: Context,
    change: DidChangeConfigurationParams
) {
    if (this.hasConfigurationCapability) {
        // Reset all cached document settings
        this.documentSettings.clear()
    } else {
        this.globalSettings =
            change.settings.languageServerExample || DEFAULT_SETTINGS
    }
    // Refresh the diagnostics since the `maxNumberOfProblems` could have changed.
    // We could optimize things here and re-fetch the setting first can compare it
    // to the existing setting, but this is out of scope for this example.
    this.conn.languages.diagnostics.refresh()
}
