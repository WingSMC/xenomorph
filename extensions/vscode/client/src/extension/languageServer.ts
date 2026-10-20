import { OutputChannel, window, workspace } from 'vscode';
import {
    Executable,
    LanguageClient,
    LanguageClientOptions,
} from 'vscode-languageclient/node';
import { errorMessage } from './xeno';

const RELOAD_PLUGINS = 'xenomorph.reloadPlugins';

/** Owns the `xenomorph_lsp` process and its lifecycle. */
export class LanguageServer {
    private readonly client: LanguageClient;
    private readonly executable: string;
    private restarting: Promise<void> | undefined;

    public constructor(private readonly output: OutputChannel) {
        this.executable = workspace
            .getConfiguration('xenomorph.lsp')
            .get<string>('executable', 'xenomorph_lsp');
        const server: Executable = {
            command: this.executable,
            options: { cwd: workspace.workspaceFolders?.[0]?.uri.fsPath },
        };
        const options: LanguageClientOptions = {
            documentSelector: [{ scheme: 'file', language: 'xenomorph' }],
            outputChannel: output,
            middleware: {
                // The client registers the commands the server advertises;
                // plugin reloads go through here to report the outcome.
                executeCommand: (command, args, next) =>
                    command === RELOAD_PLUGINS
                        ? this.reloadPlugins(args)
                        : next(command, args),
            },
        };
        this.client = new LanguageClient(
            'xenomorph_language_client',
            'Xenomorph Language Client',
            { run: server, debug: server },
            options,
        );
    }

    public async start(): Promise<void> {
        this.output.appendLine(`[LSP] Starting ${this.executable} from PATH.`);
        try {
            await this.client.start();
        } catch (error) {
            const message = errorMessage(error);
            this.output.appendLine(`[LSP] Failed to start: ${message}`);
            void window.showErrorMessage(
                `Xenomorph LSP failed to start (${this.executable}). Ensure it is on PATH. ${message}`,
            );
        }
    }

    public restart(reason: string): Promise<void> {
        this.restarting ??= this.doRestart(reason).finally(() => {
            this.restarting = undefined;
        });
        return this.restarting;
    }

    public async stop(): Promise<void> {
        await this.restarting?.catch(() => undefined);
        try {
            await this.client.stop();
        } catch {
            await this.client.dispose().catch(() => undefined);
        }
    }

    private async doRestart(reason: string): Promise<void> {
        this.output.appendLine(`[LSP] Restarting (${reason})...`);
        try {
            await this.client.restart();
            this.output.appendLine('[LSP] Restarted successfully.');
        } catch (error) {
            const message = errorMessage(error);
            this.output.appendLine(`[LSP] Restart failed: ${message}`);
            this.output.show(true);
            void window.showErrorMessage(
                `Xenomorph LSP failed to restart. ${message}`,
            );
        }
    }

    private async reloadPlugins(args: unknown[]): Promise<void> {
        if (!this.client.isRunning()) {
            void window.showErrorMessage('The Xenomorph LSP is not running.');
            return;
        }
        try {
            await this.client.sendRequest('workspace/executeCommand', {
                command: RELOAD_PLUGINS,
                arguments: args,
            });
            this.output.appendLine(
                '[LSP] Plugins reloaded without restarting the server.',
            );
        } catch (error) {
            const message = errorMessage(error);
            this.output.appendLine(`[LSP] Plugin reload failed: ${message}`);
            void window.showErrorMessage(
                `Xenomorph plugin reload failed. ${message}`,
            );
        }
    }
}
