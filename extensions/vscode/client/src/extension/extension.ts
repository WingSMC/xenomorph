import {
    commands,
    ExtensionContext,
    languages,
    OutputChannel,
    Selection,
    TextEditorRevealType,
    Uri,
    ViewColumn,
    window,
    workspace,
} from 'vscode';
import type { AstViewData, ModuleGraph, WebviewMessage } from '../shared/protocol';
import { XenomorphCodeLensProvider } from './codeLens';
import { LanguageServer } from './languageServer';
import { logAst, logDebug, logModuleGraph, logParse } from './log';
import {
    offsetResult,
    resolveTarget,
    resolveWorkspaceFolder,
    SourceTarget,
    toRange,
} from './targets';
import { ViewPanel } from './viewPanel';
import { errorMessage, inspect, isInspectRange, moduleGraph } from './xeno';

type InspectMode = 'parse' | 'debug' | 'ast';

let server: LanguageServer | undefined;

export function activate(context: ExtensionContext): void {
    const output = window.createOutputChannel('Xenomorph');
    const codeLens = new XenomorphCodeLensProvider(output);
    const astPanel = new ViewPanel<AstViewData>(context.extensionUri, 'ast', revealRange);
    const graphPanel = new ViewPanel<ModuleGraph>(context.extensionUri, 'graph', openModule);
    server = new LanguageServer(output);

    const inspectCommand = (mode: InspectMode) => (target?: SourceTarget) =>
        reportErrors(output, 'Inspector', () =>
            runInspection(output, astPanel, mode, target),
        );
    const graphCommand = (asJson: boolean) => () =>
        reportErrors(output, 'Module Graph', () =>
            showModuleGraph(output, graphPanel, asJson),
        );
    const lsp = server;

    context.subscriptions.push(
        output,
        codeLens,
        astPanel,
        graphPanel,
        languages.registerCodeLensProvider(
            { language: 'xenomorph', scheme: 'file' },
            codeLens,
        ),
        commands.registerCommand('xenomorph.parse', inspectCommand('parse')),
        commands.registerCommand('xenomorph.debug', inspectCommand('debug')),
        commands.registerCommand('xenomorph.showAst', inspectCommand('ast')),
        commands.registerCommand('xenomorph.showModuleGraph', graphCommand(false)),
        commands.registerCommand('xenomorph.showModuleGraphJson', graphCommand(true)),
        commands.registerCommand('xenomorph.restartLsp', () =>
            lsp.restart('manual command'),
        ),
    );

    void server.start();
}

export async function deactivate(): Promise<void> {
    await server?.stop();
    server = undefined;
}

async function runInspection(
    output: OutputChannel,
    astPanel: ViewPanel<AstViewData>,
    mode: InspectMode,
    sourceTarget?: SourceTarget,
): Promise<void> {
    const target = await resolveTarget(sourceTarget);
    if (!target) {
        return;
    }
    const result = await inspect(target.source, target.document.uri);
    if (target.range) {
        offsetResult(result, target.range.start);
    }

    switch (mode) {
        case 'parse':
            logParse(output, target.label, result);
            output.show(true);
            break;
        case 'debug':
            logDebug(output, target.label, result);
            output.show(true);
            break;
        case 'ast': {
            logAst(output, target.label, result);
            const where = target.range
                ? `line ${target.range.start.line + 1}`
                : target.document.uri.path.split('/').pop();
            astPanel.show(`Xenomorph AST · ${where}`, {
                uri: target.document.uri.toString(),
                label: target.label,
                ast: result.ast,
                diagnostics: result.diagnostics,
            });
            break;
        }
    }
}

async function showModuleGraph(
    output: OutputChannel,
    graphPanel: ViewPanel<ModuleGraph>,
    asJson: boolean,
): Promise<void> {
    const folder = await resolveWorkspaceFolder();
    if (!folder) {
        return;
    }
    const graph = await moduleGraph(folder);
    logModuleGraph(output, graph);

    if (!asJson) {
        graphPanel.show(`Xenomorph Module Graph · ${graph.entry}`, graph);
        return;
    }
    const document = await workspace.openTextDocument({
        content: JSON.stringify(graph, null, 2),
        language: 'json',
    });
    await window.showTextDocument(document, ViewColumn.Beside);
}

async function revealRange(message: WebviewMessage): Promise<void> {
    if (message.type !== 'reveal' || !isInspectRange(message.range)) {
        return;
    }
    const document = await workspace.openTextDocument(Uri.parse(message.uri));
    const range = document.validateRange(toRange(message.range));
    const editor = await window.showTextDocument(document, {
        viewColumn: ViewColumn.One,
        preserveFocus: true,
    });
    editor.selection = new Selection(range.start, range.end);
    editor.revealRange(range, TextEditorRevealType.InCenterIfOutsideViewport);
}

async function openModule(message: WebviewMessage): Promise<void> {
    if (message.type !== 'open' || typeof message.path !== 'string') {
        return;
    }
    const document = await workspace.openTextDocument(Uri.file(message.path));
    await window.showTextDocument(document, {
        viewColumn: ViewColumn.One,
        preserveFocus: true,
    });
}

async function reportErrors(
    output: OutputChannel,
    source: string,
    action: () => Promise<void>,
): Promise<void> {
    try {
        await action();
    } catch (error) {
        const message = errorMessage(error);
        output.appendLine(`[${source}] ${message}`);
        output.show(true);
        void window.showErrorMessage(message);
    }
}
