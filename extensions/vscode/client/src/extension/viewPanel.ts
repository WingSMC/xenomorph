import { randomBytes } from 'node:crypto';
import {
    Disposable,
    Uri,
    ViewColumn,
    Webview,
    WebviewPanel,
    window,
} from 'vscode';
import type {
    RenderMessage,
    ViewKind,
    WebviewMessage,
} from '../shared/protocol';

/**
 * A single reusable webview tab running one of the Svelte views bundled in
 * `out/webview`. Data is pushed with `show` once the view reports it is ready.
 */
export class ViewPanel<T> implements Disposable {
    private panel: WebviewPanel | undefined;
    private data: T | undefined;
    private ready = false;

    public constructor(
        private readonly extensionUri: Uri,
        private readonly view: ViewKind,
        private readonly onMessage: (message: WebviewMessage) => Promise<void>,
    ) {}

    public show(title: string, data: T): void {
        this.data = data;
        if (this.panel) {
            this.panel.title = title;
            this.panel.reveal(ViewColumn.Beside, false);
            this.render();
            return;
        }

        const assets = Uri.joinPath(this.extensionUri, 'client', 'out', 'webview');
        const panel = window.createWebviewPanel(
            `xenomorph.${this.view}`,
            title,
            { viewColumn: ViewColumn.Beside, preserveFocus: false },
            {
                enableScripts: true,
                retainContextWhenHidden: true,
                localResourceRoots: [assets],
            },
        );
        panel.onDidDispose(() => {
            this.panel = undefined;
            this.ready = false;
        });
        panel.webview.onDidReceiveMessage((message: WebviewMessage) => {
            if (message.type === 'ready') {
                this.ready = true;
                this.render();
            } else {
                this.onMessage(message).catch((error: unknown) => {
                    void window.showErrorMessage(String(error));
                });
            }
        });
        panel.webview.html = html(panel.webview, assets, this.view);
        this.panel = panel;
    }

    public dispose(): void {
        this.panel?.dispose();
    }

    private render(): void {
        if (this.panel && this.ready && this.data !== undefined) {
            const message: RenderMessage<T> = { type: 'render', data: this.data };
            void this.panel.webview.postMessage(message);
        }
    }
}

function html(webview: Webview, assets: Uri, view: ViewKind): string {
    const nonce = randomBytes(16).toString('base64');
    const asset = (file: string) =>
        webview.asWebviewUri(Uri.joinPath(assets, file)).toString();
    return `<!doctype html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <link rel="stylesheet" href="${asset('webview.css')}">
</head>
<body data-view="${view}">
    <script type="module" nonce="${nonce}" src="${asset('webview.js')}"></script>
</body>
</html>`;
}
