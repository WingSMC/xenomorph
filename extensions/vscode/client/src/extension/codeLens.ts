import {
    CancellationError,
    CancellationToken,
    CodeLens,
    CodeLensProvider,
    Disposable,
    EventEmitter,
    OutputChannel,
    TextDocument,
    workspace,
} from 'vscode';
import type { InspectResult } from '../shared/protocol';
import { SourceTarget, toRange } from './targets';
import { errorMessage, inspect } from './xeno';

const ACTIONS = [
    { title: '$(check) Parse', command: 'xenomorph.parse' },
    { title: '$(debug-alt) Debug tokens + AST', command: 'xenomorph.debug' },
    { title: '$(type-hierarchy) View AST', command: 'xenomorph.showAst' },
];

/** Adds Parse / Debug / View AST actions above each top-level declaration. */
export class XenomorphCodeLensProvider implements CodeLensProvider, Disposable {
    private readonly changed = new EventEmitter<void>();
    private readonly cache = new Map<string, { version: number; result: InspectResult }>();
    private readonly settingsListener: Disposable;
    private lastError = '';

    public readonly onDidChangeCodeLenses = this.changed.event;

    public constructor(private readonly output: OutputChannel) {
        this.settingsListener = workspace.onDidChangeConfiguration((event) => {
            if (
                event.affectsConfiguration('xenomorph.codeLens') ||
                event.affectsConfiguration('xenomorph.parser')
            ) {
                this.cache.clear();
                this.changed.fire();
            }
        });
    }

    public async provideCodeLenses(
        document: TextDocument,
        token: CancellationToken,
    ): Promise<CodeLens[]> {
        const enabled = workspace
            .getConfiguration('xenomorph.codeLens', document.uri)
            .get<boolean>('enabled', false);
        if (!enabled) {
            return [];
        }

        let result: InspectResult;
        try {
            result = await this.inspect(document, token);
            this.lastError = '';
        } catch (error) {
            if (!(error instanceof CancellationError)) {
                this.logOnce(errorMessage(error));
            }
            return [];
        }

        const uri = document.uri.toString();
        return result.ast.flatMap(({ range }) => {
            if (!range) {
                return [];
            }
            const target: SourceTarget = { uri, range };
            return ACTIONS.map(
                (action) =>
                    new CodeLens(toRange(range), { ...action, arguments: [target] }),
            );
        });
    }

    public dispose(): void {
        this.settingsListener.dispose();
        this.changed.dispose();
        this.cache.clear();
    }

    private async inspect(
        document: TextDocument,
        token: CancellationToken,
    ): Promise<InspectResult> {
        const key = document.uri.toString();
        const cached = this.cache.get(key);
        if (cached?.version === document.version) {
            return cached.result;
        }
        const result = await inspect(document.getText(), document.uri, token);
        this.cache.set(key, { version: document.version, result });
        return result;
    }

    // CodeLenses refresh on every edit; avoid flooding the log with one error.
    private logOnce(message: string): void {
        if (message !== this.lastError) {
            this.output.appendLine(`[CodeLens] ${message}`);
            this.lastError = message;
        }
    }
}
