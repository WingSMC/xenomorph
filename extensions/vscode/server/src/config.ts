import { ExampleSettings } from './types'

// The global settings, used when the `workspace/configuration` request is not supported by the client.
// Please note that this is not the case when using this server with the client provided in this example
// but could happen with other clients.
export const DEFAULT_SETTINGS: ExampleSettings = {
    maxNumberOfProblems: 1000,
}
