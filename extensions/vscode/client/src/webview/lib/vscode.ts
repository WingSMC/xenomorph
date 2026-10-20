import type { RenderMessage, WebviewMessage } from '../../shared/protocol';

interface VsCodeApi {
    postMessage(message: WebviewMessage): void;
}

declare function acquireVsCodeApi(): VsCodeApi;

const api = acquireVsCodeApi();

export function post(message: WebviewMessage): void {
    api.postMessage(message);
}

/** Calls `render` with every payload the extension sends to this view. */
export function onRender<T>(render: (data: T) => void): void {
    window.addEventListener('message', (event: MessageEvent<RenderMessage<T>>) => {
        if (event.data?.type === 'render') {
            render(event.data.data);
        }
    });
    post({ type: 'ready' });
}
