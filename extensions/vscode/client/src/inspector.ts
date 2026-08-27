import { spawn } from 'node:child_process';
import { dirname } from 'node:path';
import {
    CancellationError,
    CancellationToken,
    OutputChannel,
    Uri,
    workspace,
} from 'vscode';
import {
    errorMessage,
    formatRange,
    InspectDiagnostic,
    InspectResult,
    isInspectResult,
    ResolvedTarget,
} from './inspectionTypes';

export function runInspector(
    source: string,
    documentUri: Uri,
    cancellationToken?: CancellationToken,
): Promise<InspectResult> {
    const configuration = workspace.getConfiguration(
        'xenomorph.parser',
        documentUri,
    );
    const command = configuration.get<string>('executable', 'xeno');
    const timeout = configuration.get<number>('timeout', 15_000);
    const workspaceFolder = workspace.getWorkspaceFolder(documentUri);
    const cwd = workspaceFolder?.uri.fsPath ?? dirname(documentUri.fsPath);

    return new Promise((resolve, reject) => {
        const child = spawn(command, ['inspect'], {
            cwd,
            env: process.env,
            shell: false,
            windowsHide: true,
        });
        let stdout = '';
        let stderr = '';
        let processError: Error | undefined;
        let settled = false;

        const timer = setTimeout(() => {
            processError = new Error(
                `Xenomorph parser timed out after ${timeout} ms (${command} inspect).`,
            );
            child.kill();
        }, timeout);
        const cancellation = cancellationToken?.onCancellationRequested(() => {
            processError = new CancellationError();
            child.kill();
        });

        const cleanUp = () => {
            clearTimeout(timer);
            cancellation?.dispose();
        };
        const fail = (error: Error) => {
            if (settled) {
                return;
            }
            settled = true;
            cleanUp();
            reject(error);
        };

        child.stdout.setEncoding('utf8');
        child.stdout.on('data', (chunk: string) => {
            stdout += chunk;
        });
        child.stderr.setEncoding('utf8');
        child.stderr.on('data', (chunk: string) => {
            stderr += chunk;
        });
        child.stdin.on('error', () => undefined);
        child.once('error', (error) => {
            const detail =
                (error as NodeJS.ErrnoException).code === 'ENOENT'
                    ? `Could not find '${command}' on PATH.`
                    : error.message;
            fail(
                new Error(
                    `${detail} Configure xenomorph.parser.executable if needed.`,
                ),
            );
        });
        child.once('close', (code) => {
            if (settled) {
                return;
            }
            if (processError) {
                fail(processError);
                return;
            }
            if (code !== 0) {
                fail(
                    new Error(
                        `Xenomorph parser exited with code ${code}: ${stderr.trim() || 'no error output'}`,
                    ),
                );
                return;
            }

            try {
                const result: unknown = JSON.parse(stdout);
                if (!isInspectResult(result)) {
                    throw new Error(
                        'the response did not match the inspection protocol',
                    );
                }
                settled = true;
                cleanUp();
                resolve(result);
            } catch (error) {
                fail(
                    new Error(
                        `Could not read '${command} inspect' output: ${errorMessage(error)}${stderr ? `\n${stderr.trim()}` : ''}`,
                    ),
                );
            }
        });

        child.stdin.end(source);
    });
}

export function logParseResult(
    channel: OutputChannel,
    target: ResolvedTarget,
    result: InspectResult,
): void {
    writeHeader(channel, `Parse: ${target.label}`);
    channel.appendLine(
        `${result.ast.length} declaration(s), ${result.diagnostics.length} diagnostic(s)`,
    );
    writeDiagnostics(channel, result.diagnostics);
    channel.appendLine('\nAST');
    channel.appendLine(JSON.stringify(result.ast, null, 2));
}

export function logDebugResult(
    channel: OutputChannel,
    target: ResolvedTarget,
    result: InspectResult,
): void {
    writeHeader(channel, `Debug: ${target.label}`);
    channel.appendLine(`TOKENS (${result.tokens.length})`);
    for (const token of result.tokens) {
        channel.appendLine(
            `${formatRange(token.range).padEnd(18)} ${token.kind.padEnd(18)} ${JSON.stringify(token.lexeme)}`,
        );
    }
    channel.appendLine(`\nAST (${result.ast.length} declaration(s))`);
    channel.appendLine(JSON.stringify(result.ast, null, 2));
    channel.appendLine(`\nDIAGNOSTICS (${result.diagnostics.length})`);
    writeDiagnostics(channel, result.diagnostics);
}

export function logVisualization(
    channel: OutputChannel,
    target: ResolvedTarget,
    result: InspectResult,
): void {
    writeHeader(channel, `AST visualization: ${target.label}`);
    channel.appendLine(
        `${result.ast.length} declaration(s), ${result.diagnostics.length} diagnostic(s)`,
    );
    writeDiagnostics(channel, result.diagnostics);
}

function writeHeader(channel: OutputChannel, title: string): void {
    channel.appendLine(`\n${'='.repeat(72)}`);
    channel.appendLine(`[${new Date().toISOString()}] ${title}`);
    channel.appendLine('='.repeat(72));
}

function writeDiagnostics(
    channel: OutputChannel,
    diagnostics: InspectDiagnostic[],
): void {
    if (diagnostics.length === 0) {
        channel.appendLine('No diagnostics.');
        return;
    }
    for (const diagnostic of diagnostics) {
        channel.appendLine(
            `[${diagnostic.severity.toUpperCase()}] ${formatRange(diagnostic.range)} ${diagnostic.message}`,
        );
    }
}
