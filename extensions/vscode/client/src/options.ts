import { join } from 'node:path';
import {
	ExtensionContext,
	workspace,
} from 'vscode';
import { LanguageClientOptions } from 'vscode-languageclient';
import { TransportKind } from 'vscode-languageclient/node';

const path = join('server', 'out', 'server.js');
export function createServerOptions(
	context: ExtensionContext,
) {
	const serverModule =
		context.asAbsolutePath(path);

	return {
		run: {
			module: serverModule,
			transport: TransportKind.ipc,
		},
		debug: {
			module: serverModule,
			transport: TransportKind.ipc,
		},
	};
}

// Options to control the language client
export const clientOptions: LanguageClientOptions =
	{
		documentSelector: [
			{ scheme: 'file', language: 'plaintext' },
		],
		synchronize: {
			fileEvents:
				workspace.createFileSystemWatcher(
					'**/.clientrc',
				),
		},
	};
