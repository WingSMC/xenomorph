import { spawn } from 'node:child_process';
import { OutputChannel, Uri, workspace } from 'vscode';
import { errorMessage } from './inspectionTypes';

export interface ModuleGraphDiagnosticCounts {
    errors: number;
    warnings: number;
    infos: number;
}

export interface ModuleGraphNode {
    path: string;
    absolutePath: string;
    entry: boolean;
    declarations: number;
    diagnostics: ModuleGraphDiagnosticCounts;
}

export interface ModuleGraphEdge {
    importer: string;
    imported: string;
}

export interface ModuleGraph {
    schemaVersion: number;
    workspaceRoot: string;
    entry: string;
    moduleCount: number;
    importCount: number;
    modules: ModuleGraphNode[];
    imports: ModuleGraphEdge[];
}

export function runModuleGraph(workspaceUri: Uri): Promise<ModuleGraph> {
    const configuration = workspace.getConfiguration(
        'xenomorph.parser',
        workspaceUri,
    );
    const command = configuration.get<string>('executable', 'xeno');
    const timeout = configuration.get<number>('timeout', 15_000);

    return new Promise((resolve, reject) => {
        const child = spawn(command, ['graph', '--json'], {
            cwd: workspaceUri.fsPath,
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
                `Xenomorph parser timed out after ${timeout} ms (${command} graph --json).`,
            );
            child.kill();
        }, timeout);
        const cleanUp = () => clearTimeout(timer);
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
                if (!isModuleGraph(result)) {
                    throw new Error(
                        'the response did not match the module graph protocol',
                    );
                }
                settled = true;
                cleanUp();
                resolve(normalizeModuleGraphPaths(result));
            } catch (error) {
                fail(
                    new Error(
                        `Could not read '${command} graph --json' output: ${errorMessage(error)}${stderr ? `\n${stderr.trim()}` : ''}`,
                    ),
                );
            }
        });
    });
}

function normalizeModuleGraphPaths(graph: ModuleGraph): ModuleGraph {
    return {
        ...graph,
        workspaceRoot: normalizeWindowsVerbatimPath(graph.workspaceRoot),
        modules: graph.modules.map((module) => ({
            ...module,
            absolutePath: normalizeWindowsVerbatimPath(module.absolutePath),
        })),
    };
}

export function normalizeWindowsVerbatimPath(value: string): string {
    const uncPrefix = '\\\\?\\UNC\\';
    if (value.toUpperCase().startsWith(uncPrefix.toUpperCase())) {
        return `\\\\${value.slice(uncPrefix.length)}`;
    }

    const prefix = '\\\\?\\';
    return value.startsWith(prefix) ? value.slice(prefix.length) : value;
}

export function logModuleGraph(
    channel: OutputChannel,
    graph: ModuleGraph,
): void {
    channel.appendLine(`\n${'='.repeat(72)}`);
    channel.appendLine(
        `[${new Date().toISOString()}] Module graph: ${graph.workspaceRoot}`,
    );
    channel.appendLine('='.repeat(72));
    channel.appendLine(JSON.stringify(graph, null, 2));
}

function isModuleGraph(value: unknown): value is ModuleGraph {
    if (!value || typeof value !== 'object') {
        return false;
    }
    const graph = value as Partial<ModuleGraph>;
    return (
        graph.schemaVersion === 1 &&
        typeof graph.workspaceRoot === 'string' &&
        typeof graph.entry === 'string' &&
        typeof graph.moduleCount === 'number' &&
        typeof graph.importCount === 'number' &&
        Array.isArray(graph.modules) &&
        graph.modules.every(isModuleGraphNode) &&
        Array.isArray(graph.imports) &&
        graph.imports.every(isModuleGraphEdge)
    );
}

function isModuleGraphNode(value: unknown): value is ModuleGraphNode {
    if (!value || typeof value !== 'object') {
        return false;
    }
    const node = value as Partial<ModuleGraphNode>;
    return (
        typeof node.path === 'string' &&
        typeof node.absolutePath === 'string' &&
        typeof node.entry === 'boolean' &&
        typeof node.declarations === 'number' &&
        isDiagnosticCounts(node.diagnostics)
    );
}

function isDiagnosticCounts(
    value: unknown,
): value is ModuleGraphDiagnosticCounts {
    if (!value || typeof value !== 'object') {
        return false;
    }
    const counts = value as Partial<ModuleGraphDiagnosticCounts>;
    return (
        typeof counts.errors === 'number' &&
        typeof counts.warnings === 'number' &&
        typeof counts.infos === 'number'
    );
}

function isModuleGraphEdge(value: unknown): value is ModuleGraphEdge {
    if (!value || typeof value !== 'object') {
        return false;
    }
    const edge = value as Partial<ModuleGraphEdge>;
    return (
        typeof edge.importer === 'string' && typeof edge.imported === 'string'
    );
}
