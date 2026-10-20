import { OutputChannel } from 'vscode';
import type {
    InspectDiagnostic,
    InspectRange,
    InspectResult,
    ModuleGraph,
} from '../shared/protocol';

export function logParse(
    channel: OutputChannel,
    label: string,
    result: InspectResult,
): void {
    header(channel, `Parse: ${label}`);
    summary(channel, result);
    diagnostics(channel, result.diagnostics);
    channel.appendLine('\nAST');
    channel.appendLine(JSON.stringify(result.ast, null, 2));
}

export function logDebug(
    channel: OutputChannel,
    label: string,
    result: InspectResult,
): void {
    header(channel, `Debug: ${label}`);
    channel.appendLine(`TOKENS (${result.tokens.length})`);
    for (const token of result.tokens) {
        channel.appendLine(
            `${formatRange(token.range).padEnd(18)} ${token.kind.padEnd(18)} ${JSON.stringify(token.lexeme)}`,
        );
    }
    channel.appendLine(`\nAST (${result.ast.length} declaration(s))`);
    channel.appendLine(JSON.stringify(result.ast, null, 2));
    channel.appendLine(`\nDIAGNOSTICS (${result.diagnostics.length})`);
    diagnostics(channel, result.diagnostics);
}

export function logAst(
    channel: OutputChannel,
    label: string,
    result: InspectResult,
): void {
    header(channel, `AST visualization: ${label}`);
    summary(channel, result);
    diagnostics(channel, result.diagnostics);
}

export function logModuleGraph(
    channel: OutputChannel,
    graph: ModuleGraph,
): void {
    header(channel, `Module graph: ${graph.workspaceRoot}`);
    channel.appendLine(
        `${graph.moduleCount} module(s), ${graph.importCount} import(s), entry ${graph.entry}`,
    );
}

function header(channel: OutputChannel, title: string): void {
    const rule = '='.repeat(72);
    channel.appendLine(`\n${rule}\n[${new Date().toISOString()}] ${title}\n${rule}`);
}

function summary(channel: OutputChannel, result: InspectResult): void {
    channel.appendLine(
        `${result.ast.length} declaration(s), ${result.diagnostics.length} diagnostic(s)`,
    );
}

function diagnostics(
    channel: OutputChannel,
    items: InspectDiagnostic[],
): void {
    if (items.length === 0) {
        channel.appendLine('No diagnostics.');
    }
    for (const { severity, range, message } of items) {
        channel.appendLine(
            `[${severity.toUpperCase()}] ${formatRange(range)} ${message}`,
        );
    }
}

function formatRange({ start, end }: InspectRange): string {
    return `${start.line + 1}:${start.character + 1}-${end.line + 1}:${end.character + 1}`;
}
