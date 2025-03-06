import { ExtensionContext } from 'vscode';
import { LanguageClient } from 'vscode-languageclient/node';
import {
	clientOptions,
	createServerOptions,
} from './options';

let client: LanguageClient | undefined;

export function activate(
	context: ExtensionContext,
) {
	client = new LanguageClient(
		'languageServerExample',
		'Language Server Example',
		createServerOptions(context),
		clientOptions,
	);

	client.start();
}

export function deactivate() {
	if (!client) return undefined;
	return client.stop();
}
