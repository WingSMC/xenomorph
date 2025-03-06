import type { Context, ExampleSettings } from './types'

export function getDocumentSettings(
    this: Context,
    resource: string
): Thenable<ExampleSettings> {
    if (!this.hasConfigurationCapability) {
        return Promise.resolve(this.globalSettings)
    }
    let result = this.documentSettings.get(resource)
    if (!result) {
        result = this.conn.workspace.getConfiguration({
            scopeUri: resource,
            section: 'languageServerExample',
        })
        this.documentSettings.set(resource, result)
    }
    return result
}
