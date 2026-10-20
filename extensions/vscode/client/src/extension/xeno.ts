import { spawn } from 'node:child_process';
import { dirname } from 'node:path';
import { CancellationError, CancellationToken, Uri, workspace } from 'vscode';
import type {
    InspectRange,
    InspectResult,
    ModuleGraph,
} from '../shared/protocol';

interface RunOptions {
    /** Resource used to read `xenomorph.parser.*` settings. */
    scope: Uri;
    cwd: string;
    stdin?: string;
    token?: CancellationToken;
}

/** Runs `xeno inspect` on `source` and returns its tokens, AST and diagnostics. */
export async function inspect(
    source: string,
    document: Uri,
    token?: CancellationToken,
): Promise<InspectResult> {
    const cwd =
        workspace.getWorkspaceFolder(document)?.uri.fsPath ??
        dirname(document.fsPath);
    const args = ['inspect'];
    const output = await runXeno(args, {
        scope: document,
        cwd,
        stdin: source,
        token,
    });
    return parseOutput(args, output, isInspectResult);
}

/** Runs `xeno graph --json` in the given workspace folder. */
export async function moduleGraph(root: Uri): Promise<ModuleGraph> {
    const args = ['graph', '--json'];
    const output = await runXeno(args, { scope: root, cwd: root.fsPath });
    const graph = parseOutput(args, output, isModuleGraph);
    return {
        ...graph,
        workspaceRoot: stripVerbatimPrefix(graph.workspaceRoot),
        modules: graph.modules.map((module) => ({
            ...module,
            absolutePath: stripVerbatimPrefix(module.absolutePath),
        })),
    };
}

/** Converts Windows `\\?\` paths (as produced by Rust's canonicalize) to regular ones. */
export function stripVerbatimPrefix(path: string): string {
    if (/^\\\\\?\\UNC\\/i.test(path)) {
        return `\\\\${path.slice(8)}`;
    }
    return path.startsWith('\\\\?\\') ? path.slice(4) : path;
}

export function errorMessage(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
}

function runXeno(args: string[], options: RunOptions): Promise<string> {
    const settings = workspace.getConfiguration(
        'xenomorph.parser',
        options.scope,
    );
    const command = settings.get<string>('executable', 'xeno');
    const timeout = settings.get<number>('timeout', 15_000);
    const commandLine = [command, ...args].join(' ');

    return new Promise((resolve, reject) => {
        const child = spawn(command, args, {
            cwd: options.cwd,
            shell: false,
            windowsHide: true,
        });
        let stdout = '';
        let stderr = '';
        let failure: Error | undefined;
        let settled = false;

        const abort = (error: Error) => {
            failure ??= error;
            child.kill();
        };
        const timer = setTimeout(
            () => abort(new Error(`'${commandLine}' timed out after ${timeout} ms.`)),
            timeout,
        );
        const cancellation = options.token?.onCancellationRequested(() =>
            abort(new CancellationError()),
        );
        const settle = (error: Error | undefined) => {
            if (settled) {
                return;
            }
            settled = true;
            clearTimeout(timer);
            cancellation?.dispose();
            if (error) {
                reject(error);
            } else {
                resolve(stdout);
            }
        };

        child.stdout.setEncoding('utf8').on('data', (chunk: string) => {
            stdout += chunk;
        });
        child.stderr.setEncoding('utf8').on('data', (chunk: string) => {
            stderr += chunk;
        });
        // The process may exit before reading stdin; 'close' reports the real failure.
        child.stdin.on('error', () => undefined);
        child.once('error', (error: NodeJS.ErrnoException) => {
            const reason =
                error.code === 'ENOENT'
                    ? `Could not find '${command}' on PATH.`
                    : error.message;
            settle(
                new Error(
                    `${reason} Configure xenomorph.parser.executable if needed.`,
                ),
            );
        });
        child.once('close', (code) => {
            if (failure) {
                settle(failure);
            } else if (code !== 0) {
                const detail = stderr.trim() || 'no error output';
                settle(
                    new Error(`'${commandLine}' exited with code ${code}: ${detail}`),
                );
            } else {
                settle(undefined);
            }
        });

        child.stdin.end(options.stdin);
    });
}

function parseOutput<T>(
    args: string[],
    output: string,
    isValid: (value: unknown) => value is T,
): T {
    let value: unknown;
    try {
        value = JSON.parse(output);
    } catch (error) {
        throw new Error(
            `Could not parse 'xeno ${args.join(' ')}' output: ${errorMessage(error)}`,
        );
    }
    if (!isValid(value)) {
        throw new Error(
            `'xeno ${args.join(' ')}' output does not match the expected protocol. Is the xeno executable up to date?`,
        );
    }
    return value;
}

type Fields = Record<string, unknown>;

const isObject = (value: unknown): value is Fields =>
    typeof value === 'object' && value !== null;

const hasNumbers = (value: Fields, ...keys: string[]) =>
    keys.every((key) => typeof value[key] === 'number');

const hasStrings = (value: Fields, ...keys: string[]) =>
    keys.every((key) => typeof value[key] === 'string');

export function isInspectRange(value: unknown): value is InspectRange {
    return (
        isObject(value) &&
        isObject(value.start) &&
        isObject(value.end) &&
        hasNumbers(value.start, 'line', 'character') &&
        hasNumbers(value.end, 'line', 'character')
    );
}

function isInspectResult(value: unknown): value is InspectResult {
    return (
        isObject(value) &&
        Array.isArray(value.tokens) &&
        Array.isArray(value.ast) &&
        Array.isArray(value.diagnostics)
    );
}

function isModuleGraph(value: unknown): value is ModuleGraph {
    return (
        isObject(value) &&
        value.schemaVersion === 1 &&
        hasStrings(value, 'workspaceRoot', 'entry') &&
        hasNumbers(value, 'moduleCount', 'importCount') &&
        Array.isArray(value.modules) &&
        value.modules.every(
            (module) =>
                isObject(module) &&
                hasStrings(module, 'path', 'absolutePath') &&
                typeof module.entry === 'boolean' &&
                hasNumbers(module, 'declarations') &&
                isObject(module.diagnostics) &&
                hasNumbers(module.diagnostics, 'errors', 'warnings', 'infos'),
        ) &&
        Array.isArray(value.imports) &&
        value.imports.every(
            (edge) => isObject(edge) && hasStrings(edge, 'importer', 'imported'),
        )
    );
}
