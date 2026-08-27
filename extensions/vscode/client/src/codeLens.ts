import {
    CancellationError,
    CancellationToken,
    CodeLens,
    CodeLensProvider,
    Disposable,
    Event,
    EventEmitter,
    OutputChannel,
    TextDocument,
    workspace,
} from 'vscode';
import {
    errorMessage,
    InspectResult,
    SourceTarget,
    toRange,
} from './inspectionTypes';
import { runInspector } from './inspector';

export class XenomorphCodeLensProvider implements CodeLensProvider, Disposable {
    private readonly changedEmitter = new EventEmitter<void>();
    private readonly cache = new Map<
        string,
        { version: number; result: InspectResult }
    >();
    private readonly configurationSubscription: Disposable;
    private lastError = '';

    public readonly onDidChangeCodeLenses: Event<void> =
        this.changedEmitter.event;

    public constructor(private readonly channel: OutputChannel) {
        this.configurationSubscription = workspace.onDidChangeConfiguration(
            (event) => {
                if (
                    event.affectsConfiguration('xenomorph.codeLens') ||
                    event.affectsConfiguration('xenomorph.parser')
                ) {
                    this.cache.clear();
                    this.changedEmitter.fire();
                }
            },
        );
    }

    public async provideCodeLenses(
        document: TextDocument,
        cancellationToken: CancellationToken,
    ): Promise<CodeLens[]> {
        const enabled = workspace
            .getConfiguration('xenomorph.codeLens', document.uri)
            .get<boolean>('enabled', true);
        if (!enabled) {
            return [];
        }

        try {
            const key = document.uri.toString();
            const cached = this.cache.get(key);
            const result =
                cached?.version === document.version
                    ? cached.result
                    : await runInspector(
                          document.getText(),
                          document.uri,
                          cancellationToken,
                      );
            if (cached?.version !== document.version) {
                this.cache.set(key, { version: document.version, result });
            }
            this.lastError = '';

            return result.ast.flatMap((declaration) => {
                if (!declaration.range) {
                    return [];
                }
                const range = toRange(declaration.range);
                const target: SourceTarget = {
                    uri: document.uri.toString(),
                    range: declaration.range,
                };
                return [
                    new CodeLens(range, {
                        title: '$(check) Parse',
                        command: 'xenomorph.parse',
                        arguments: [target],
                    }),
                    new CodeLens(range, {
                        title: '$(debug-alt) Debug tokens + AST',
                        command: 'xenomorph.debug',
                        arguments: [target],
                    }),
                    new CodeLens(range, {
                        title: '$(type-hierarchy) View AST',
                        command: 'xenomorph.showAst',
                        arguments: [target],
                    }),
                ];
            });
        } catch (error) {
            if (error instanceof CancellationError) {
                return [];
            }
            const message = errorMessage(error);
            if (message !== this.lastError) {
                this.channel.appendLine(`[CodeLens] ${message}`);
                this.lastError = message;
            }
            return [];
        }
    }

    public dispose(): void {
        this.configurationSubscription.dispose();
        this.changedEmitter.dispose();
        this.cache.clear();
    }
}
