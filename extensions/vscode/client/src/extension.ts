import {
    commands,
    ExtensionContext,
    languages,
    OutputChannel,
    Range,
    Uri,
    window,
    workspace,
} from 'vscode';
import {
    Executable,
    LanguageClient,
    LanguageClientOptions,
} from 'vscode-languageclient/node';
import { disposeAstWebview, showAstWebview } from './astWebview';
import { XenomorphCodeLensProvider } from './codeLens';
import {
    errorMessage,
    offsetResult,
    ResolvedTarget,
    SourceTarget,
    toRange,
} from './inspectionTypes';
import {
    logDebugResult,
    logParseResult,
    logVisualization,
    runInspector,
} from './inspector';

function createServerOptions(): {
    run: Executable;
    debug: Executable;
} {
    const command = workspace
        .getConfiguration('xenomorph.lsp')
        .get<string>('executable', 'xenomorph_lsp');
    const options: Executable['options'] = {
        cwd: workspace.workspaceFolders?.[0]?.uri.fsPath,
    };
    const executable = { command, options };
    return { run: executable, debug: executable };
}

let client: LanguageClient | undefined;
let output: OutputChannel | undefined;

export function activate(context: ExtensionContext): void {
    output = window.createOutputChannel('Xenomorph');
    const codeLensProvider = new XenomorphCodeLensProvider(output);
    context.subscriptions.push(
        output,
        codeLensProvider,
        languages.registerCodeLensProvider(
            { language: 'xenomorph', scheme: 'file' },
            codeLensProvider,
        ),
        commands.registerCommand('xenomorph.parse', (target?: SourceTarget) =>
            inspectCommand('parse', target),
        ),
        commands.registerCommand('xenomorph.debug', (target?: SourceTarget) =>
            inspectCommand('debug', target),
        ),
        commands.registerCommand('xenomorph.showAst', (target?: SourceTarget) =>
            inspectCommand('ast', target),
        ),
    );

    const clientOptions: LanguageClientOptions = {
        documentSelector: [{ scheme: 'file', language: 'xenomorph' }],
        outputChannel: output,
    };
    const lspExecutable = workspace
        .getConfiguration('xenomorph.lsp')
        .get<string>('executable', 'xenomorph_lsp');
    output.appendLine(`[LSP] Starting ${lspExecutable} from PATH.`);

    client = new LanguageClient(
        'xenomorph_language_client',
        'Xenomorph Language Client',
        createServerOptions(),
        clientOptions,
    );

    void client.start().catch((error: unknown) => {
        const message = errorMessage(error);
        output?.appendLine(`[LSP] Failed to start: ${message}`);
        void window.showErrorMessage(
            `Xenomorph LSP failed to start (${lspExecutable}). Ensure it is on PATH. ${message}`,
        );
    });
}

async function inspectCommand(
    mode: 'parse' | 'debug' | 'ast',
    sourceTarget?: SourceTarget,
): Promise<void> {
    if (!output) {
        return;
    }

    const target = await resolveTarget(sourceTarget);
    if (!target) {
        return;
    }

    try {
        const result = await runInspector(target.source, target.document.uri);
        if (target.range) {
            offsetResult(result, target.range.start);
        }

        if (mode === 'debug') {
            logDebugResult(output, target, result);
            output.show(true);
        } else if (mode === 'parse') {
            logParseResult(output, target, result);
            output.show(true);
        } else {
            logVisualization(output, target, result);
            showAstWebview(target, result);
        }
    } catch (error) {
        const message = errorMessage(error);
        output.appendLine(`[Inspector] ${message}`);
        output.show(true);
        void window.showErrorMessage(message);
    }
}

async function resolveTarget(
    sourceTarget?: SourceTarget,
): Promise<ResolvedTarget | undefined> {
    const activeDocument = window.activeTextEditor?.document;
    const uri = sourceTarget?.uri
        ? Uri.parse(sourceTarget.uri)
        : activeDocument?.uri;

    if (!uri) {
        void window.showInformationMessage('Open a Xenomorph document first.');
        return undefined;
    }

    const document = await workspace.openTextDocument(uri);
    if (document.languageId !== 'xenomorph') {
        void window.showInformationMessage(
            'The active document is not a Xenomorph file.',
        );
        return undefined;
    }

    let range: Range | undefined;
    if (sourceTarget?.range) {
        range = document.validateRange(toRange(sourceTarget.range));
    }
    const label = range
        ? `${document.fileName}:${range.start.line + 1}`
        : document.fileName;

    return {
        document,
        range,
        source: range ? document.getText(range) : document.getText(),
        label,
    };
}

export async function deactivate(): Promise<void> {
    disposeAstWebview();
    if (!client) {
        return;
    }

    try {
        await client.stop();
    } catch {
        await client.dispose().catch(() => undefined);
    } finally {
        client = undefined;
    }
}
