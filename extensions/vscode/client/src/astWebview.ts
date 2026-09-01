import { randomBytes } from 'node:crypto';
import {
    Selection,
    TextEditorRevealType,
    Uri,
    ViewColumn,
    WebviewPanel,
    window,
    workspace,
} from 'vscode';
import {
    InspectResult,
    isInspectRange,
    ResolvedTarget,
    toRange,
} from './inspectionTypes';

let astPanel: WebviewPanel | undefined;

export function showAstWebview(
    target: ResolvedTarget,
    result: InspectResult,
): void {
    const title = target.range
        ? `Xenomorph AST · line ${target.range.start.line + 1}`
        : `Xenomorph AST · ${target.document.uri.path.split('/').pop()}`;

    if (!astPanel) {
        astPanel = window.createWebviewPanel(
            'xenomorph.ast',
            title,
            { viewColumn: ViewColumn.Beside, preserveFocus: false },
            { enableScripts: true, retainContextWhenHidden: true },
        );
        astPanel.onDidDispose(() => {
            astPanel = undefined;
        });
        astPanel.webview.onDidReceiveMessage((message: unknown) => {
            void handleWebviewMessage(message);
        });
    } else {
        astPanel.title = title;
        astPanel.reveal(ViewColumn.Beside, false);
    }

    astPanel.webview.html = astHtml(astPanel, target, result);
}

export function disposeAstWebview(): void {
    astPanel?.dispose();
    astPanel = undefined;
}

async function handleWebviewMessage(message: unknown): Promise<void> {
    if (!message || typeof message !== 'object') {
        return;
    }
    const candidate = message as {
        type?: unknown;
        uri?: unknown;
        range?: unknown;
    };
    if (
        candidate.type !== 'reveal' ||
        typeof candidate.uri !== 'string' ||
        !isInspectRange(candidate.range)
    ) {
        return;
    }

    const document = await workspace.openTextDocument(Uri.parse(candidate.uri));
    const range = document.validateRange(toRange(candidate.range));
    const editor = await window.showTextDocument(document, {
        viewColumn: ViewColumn.One,
        preserveFocus: true,
    });
    editor.selection = new Selection(range.start, range.end);
    editor.revealRange(range, TextEditorRevealType.InCenterIfOutsideViewport);
}

function astHtml(
    panel: WebviewPanel,
    target: ResolvedTarget,
    result: InspectResult,
): string {
    const nonce = randomBytes(16).toString('hex');
    const payload = Buffer.from(
        JSON.stringify({
            uri: target.document.uri.toString(),
            label: target.label,
            ast: result.ast,
            diagnostics: result.diagnostics,
        }),
        'utf8',
    ).toString('base64');
    const csp = panel.webview.cspSource;

    return `<!doctype html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${csp} 'nonce-${nonce}'; script-src 'nonce-${nonce}';">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Xenomorph AST</title>
    <style nonce="${nonce}">
        :root { color-scheme: light dark; }
        * { box-sizing: border-box; }
        body { margin: 0; color: var(--vscode-foreground); background: var(--vscode-editor-background); font-family: var(--vscode-font-family); overflow: hidden; }
        header { height: 52px; display: flex; align-items: center; gap: 8px; padding: 8px 12px; border-bottom: 1px solid var(--vscode-panel-border); background: var(--vscode-sideBar-background); }
        .brand { font-weight: 700; margin-right: 8px; white-space: nowrap; }
        .source { color: var(--vscode-descriptionForeground); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
        button { border: 1px solid var(--vscode-button-border, transparent); color: var(--vscode-button-foreground); background: var(--vscode-button-background); padding: 5px 9px; border-radius: 4px; cursor: pointer; }
        button:hover { background: var(--vscode-button-hoverBackground); }
        main { display: grid; grid-template-columns: minmax(0, 1fr) 260px; height: calc(100vh - 52px); }
        #viewport { width: 100%; height: 100%; touch-action: none; cursor: grab; }
        #viewport.dragging { cursor: grabbing; }
        aside { border-left: 1px solid var(--vscode-panel-border); padding: 14px; overflow: auto; background: var(--vscode-sideBar-background); }
        aside h2 { font-size: 13px; text-transform: uppercase; letter-spacing: .08em; color: var(--vscode-descriptionForeground); margin: 0 0 10px; }
        #node-kind { font-weight: 700; word-break: break-word; }
        #node-label { margin-top: 6px; color: var(--vscode-symbolIcon-stringForeground); word-break: break-word; }
        #node-range { margin-top: 8px; color: var(--vscode-descriptionForeground); font-family: var(--vscode-editor-font-family); }
        #hint { margin-top: 16px; line-height: 1.45; color: var(--vscode-descriptionForeground); }
        #diagnostics { margin-top: 20px; }
        .diagnostic { margin: 8px 0; padding: 8px; border-left: 3px solid var(--vscode-editorWarning-foreground); background: var(--vscode-textBlockQuote-background); font-size: 12px; line-height: 1.4; }
        .diagnostic.error { border-color: var(--vscode-editorError-foreground); }
        .edge { fill: none; stroke: var(--vscode-editorIndentGuide-background); stroke-width: 1.5; }
        .node { cursor: pointer; }
        .node rect { fill: var(--vscode-editorWidget-background); stroke: var(--vscode-focusBorder); stroke-width: 1; rx: 8; filter: drop-shadow(0 2px 3px rgba(0, 0, 0, .18)); }
        .node:hover rect, .node.selected rect { stroke: var(--vscode-symbolIcon-classForeground); stroke-width: 2; }
        .node .kind { fill: var(--vscode-symbolIcon-classForeground); font: 600 12px var(--vscode-font-family); text-anchor: middle; }
        .node .label { fill: var(--vscode-foreground); font: 11px var(--vscode-editor-font-family); text-anchor: middle; }
        .node .badge { fill: var(--vscode-badge-background); }
        .node .badge-text { fill: var(--vscode-badge-foreground); font: 10px var(--vscode-font-family); text-anchor: middle; }
        .empty { fill: var(--vscode-descriptionForeground); font: 14px var(--vscode-font-family); text-anchor: middle; }
        @media (max-width: 760px) { main { grid-template-columns: 1fr; } aside { display: none; } }
    </style>
</head>
<body>
    <header>
        <span class="brand">Xenomorph AST</span>
        <span class="source" id="source"></span>
        <button id="expand" title="Expand every node">Expand all</button>
        <button id="collapse" title="Collapse child nodes">Collapse all</button>
        <button id="zoom-out" title="Zoom out">−</button>
        <button id="fit" title="Fit tree">Fit</button>
        <button id="zoom-in" title="Zoom in">+</button>
    </header>
    <main>
        <svg id="viewport" aria-label="Abstract syntax tree"><g id="scene"></g></svg>
        <aside>
            <h2>Selected node</h2>
            <div id="node-kind">Select a node</div>
            <div id="node-label"></div>
            <div id="node-range"></div>
            <div id="hint">Click a node to reveal its source. Double-click a parent to collapse or expand it. Drag to pan and use the mouse wheel to zoom.</div>
            <section id="diagnostics"><h2>Diagnostics</h2><div id="diagnostic-list"></div></section>
        </aside>
    </main>
    <script nonce="${nonce}">
        const vscode = acquireVsCodeApi();
        const bytes = Uint8Array.from(atob('${payload}'), character => character.charCodeAt(0));
        const data = JSON.parse(new TextDecoder().decode(bytes));
        const svg = document.getElementById('viewport');
        const scene = document.getElementById('scene');
        const collapsed = new Set();
        let selectedId = null;
        let transform = { x: 30, y: 30, scale: 1 };
        let graphSize = { width: 800, height: 600 };
        let dragging = false;
        let previousPointer = { x: 0, y: 0 };

        document.getElementById('source').textContent = data.label;
        const root = { kind: 'Document', label: data.ast.length + ' declaration(s)', children: data.ast, id: 'root' };
        const assignIds = (node, prefix) => {
            node.id = node.id || prefix;
            (node.children || []).forEach((child, index) => assignIds(child, prefix + '.' + index));
        };
        assignIds(root, 'root');

        const svgElement = (name, attributes = {}) => {
            const element = document.createElementNS('http://www.w3.org/2000/svg', name);
            Object.entries(attributes).forEach(([key, value]) => element.setAttribute(key, String(value)));
            return element;
        };
        const abbreviated = (value, length = 24) => value && value.length > length ? value.slice(0, length - 1) + '…' : value || '';
        const formatPosition = position => (position.line + 1) + ':' + (position.character + 1);

        function visibleTree(node, depth = 0, parent = null) {
            const entry = { node, depth, parent, children: [], x: 0, y: depth * 116 + 45 };
            if (!collapsed.has(node.id)) {
                entry.children = (node.children || []).map(child => visibleTree(child, depth + 1, entry));
            }
            return entry;
        }

        function layout(entry, state = { leaf: 0, maxDepth: 0 }) {
            state.maxDepth = Math.max(state.maxDepth, entry.depth);
            entry.children.forEach(child => layout(child, state));
            if (entry.children.length === 0) {
                entry.x = state.leaf++ * 210 + 110;
            } else {
                entry.x = entry.children.reduce((sum, child) => sum + child.x, 0) / entry.children.length;
            }
            return state;
        }

        function flatten(entry, result = []) {
            result.push(entry);
            entry.children.forEach(child => flatten(child, result));
            return result;
        }

        function render(shouldFit = false) {
            scene.replaceChildren();
            if (data.ast.length === 0) {
                const text = svgElement('text', { x: 400, y: 240, class: 'empty' });
                text.textContent = 'No AST was produced. Check diagnostics for syntax errors.';
                scene.append(text);
                graphSize = { width: 800, height: 480 };
                if (shouldFit) fit();
                return;
            }

            const tree = visibleTree(root);
            const state = layout(tree);
            const visible = flatten(tree);
            graphSize = {
                width: Math.max(800, state.leaf * 210 + 20),
                height: Math.max(480, (state.maxDepth + 1) * 116 + 60),
            };

            for (const entry of visible) {
                for (const child of entry.children) {
                    scene.append(svgElement('path', {
                        class: 'edge',
                        d: 'M ' + entry.x + ' ' + (entry.y + 29) + ' C ' + entry.x + ' ' + (entry.y + 74) + ', ' + child.x + ' ' + (child.y - 45) + ', ' + child.x + ' ' + (child.y - 29),
                    }));
                }
            }

            for (const entry of visible) {
                const node = entry.node;
                const group = svgElement('g', {
                    class: 'node' + (selectedId === node.id ? ' selected' : ''),
                    transform: 'translate(' + entry.x + ' ' + entry.y + ')',
                    'data-node-id': node.id,
                    tabindex: '0',
                    role: 'button',
                });
                const title = svgElement('title');
                title.textContent = node.kind + (node.label ? ': ' + node.label : '');
                group.append(title, svgElement('rect', { x: -88, y: -29, width: 176, height: 58 }));
                const kind = svgElement('text', { class: 'kind', x: 0, y: node.label ? -4 : 4 });
                kind.textContent = abbreviated(node.kind, 26);
                group.append(kind);
                if (node.label) {
                    const label = svgElement('text', { class: 'label', x: 0, y: 15 });
                    label.textContent = abbreviated(String(node.label), 28);
                    group.append(label);
                }
                if ((node.children || []).length > 0) {
                    group.append(svgElement('circle', { class: 'badge', cx: 78, cy: -20, r: 10 }));
                    const badge = svgElement('text', { class: 'badge-text', x: 78, y: -16 });
                    badge.textContent = collapsed.has(node.id) ? '+' : '−';
                    group.append(badge);
                }
                group.addEventListener('click', event => {
                    event.stopPropagation();
                    selectNode(node);
                });
                group.addEventListener('dblclick', event => {
                    event.stopPropagation();
                    if ((node.children || []).length > 0) {
                        collapsed.has(node.id) ? collapsed.delete(node.id) : collapsed.add(node.id);
                        render(true);
                    }
                });
                group.addEventListener('keydown', event => {
                    if (event.key === 'Enter' || event.key === ' ') selectNode(node);
                });
                scene.append(group);
            }
            applyTransform();
            if (shouldFit) fit();
        }

        function selectNode(node) {
            selectedId = node.id;
            document.getElementById('node-kind').textContent = node.kind;
            document.getElementById('node-label').textContent = node.label || '';
            document.getElementById('node-range').textContent = node.range ? formatPosition(node.range.start) + ' – ' + formatPosition(node.range.end) : 'No source range';
            if (node.range) vscode.postMessage({ type: 'reveal', uri: data.uri, range: node.range });
            scene.querySelectorAll('.node').forEach(element => {
                element.classList.toggle('selected', element.getAttribute('data-node-id') === node.id);
            });
        }

        function applyTransform() {
            scene.setAttribute('transform', 'translate(' + transform.x + ' ' + transform.y + ') scale(' + transform.scale + ')');
        }
        function fit() {
            const width = Math.max(svg.clientWidth, 1);
            const height = Math.max(svg.clientHeight, 1);
            transform.scale = Math.min(width / graphSize.width, height / graphSize.height, 1.35) * 0.92;
            transform.x = (width - graphSize.width * transform.scale) / 2;
            transform.y = Math.max(18, (height - graphSize.height * transform.scale) / 2);
            applyTransform();
        }
        function zoom(factor, originX = svg.clientWidth / 2, originY = svg.clientHeight / 2) {
            const nextScale = Math.min(3, Math.max(0.15, transform.scale * factor));
            const worldX = (originX - transform.x) / transform.scale;
            const worldY = (originY - transform.y) / transform.scale;
            transform.x = originX - worldX * nextScale;
            transform.y = originY - worldY * nextScale;
            transform.scale = nextScale;
            applyTransform();
        }

        svg.addEventListener('wheel', event => {
            event.preventDefault();
            const bounds = svg.getBoundingClientRect();
            zoom(event.deltaY < 0 ? 1.12 : 0.89, event.clientX - bounds.left, event.clientY - bounds.top);
        }, { passive: false });
        svg.addEventListener('pointerdown', event => {
            if (event.target.closest && event.target.closest('.node')) return;
            dragging = true;
            previousPointer = { x: event.clientX, y: event.clientY };
            svg.classList.add('dragging');
            svg.setPointerCapture(event.pointerId);
        });
        svg.addEventListener('pointermove', event => {
            if (!dragging) return;
            transform.x += event.clientX - previousPointer.x;
            transform.y += event.clientY - previousPointer.y;
            previousPointer = { x: event.clientX, y: event.clientY };
            applyTransform();
        });
        svg.addEventListener('pointerup', () => { dragging = false; svg.classList.remove('dragging'); });
        document.getElementById('fit').addEventListener('click', fit);
        document.getElementById('zoom-in').addEventListener('click', () => zoom(1.2));
        document.getElementById('zoom-out').addEventListener('click', () => zoom(0.8));
        document.getElementById('expand').addEventListener('click', () => { collapsed.clear(); render(true); });
        document.getElementById('collapse').addEventListener('click', () => {
            const visit = node => {
                if ((node.children || []).length > 0 && node.id !== 'root') collapsed.add(node.id);
                (node.children || []).forEach(visit);
            };
            visit(root);
            render(true);
        });
        window.addEventListener('resize', fit);

        const diagnosticList = document.getElementById('diagnostic-list');
        if (data.diagnostics.length === 0) {
            diagnosticList.textContent = 'No diagnostics.';
        } else {
            data.diagnostics.forEach(diagnostic => {
                const item = document.createElement('div');
                item.className = 'diagnostic ' + diagnostic.severity;
                item.textContent = diagnostic.severity.toUpperCase() + ' ' + formatPosition(diagnostic.range.start) + ' — ' + diagnostic.message;
                diagnosticList.append(item);
            });
        }
        render(true);
    </script>
</body>
</html>`;
}
